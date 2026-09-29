#!/usr/bin/env python3
"""Action-level agreement between two checkpoints (both evaluated by argmax).

A win rate can stay flat for two very different reasons: the trained policy is
identical to the reference, or it differs but the difference does not matter.
This tool measures the first directly - it walks the same seeds with both
networks and counts how often the argmax plan differs, per actor and in total.

```bash
python tools/action_agreement.py \
    --a models/ppo_plan_E_suffix_trigger001.npz \
    --b models/exp_critic_algo.npz \
    --seed-start 600001 --seed-count 60
```
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Dict, List, Sequence

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python"))


def walk(net, encoder, scenario, seeds: Sequence[int], effect_cap: int) -> List[Dict[str, Any]]:
    from lcb.env import LimbusEnv
    from lcb.plans import actor_order, canonical, group_candidates
    from lcb.baselines import NeuralPolicy

    policy = NeuralPolicy(net, encoder, sample=False, name="agreement")
    decisions: List[Dict[str, Any]] = []
    for seed in seeds:
        env = LimbusEnv(strict=scenario.strict)
        env.reset(
            seed,
            enemies=list(scenario.enemies),
            max_turns=scenario.max_turns,
            enemy_hp_scale=scenario.enemy_hp_scale,
            infinite_ego_resources=scenario.infinite_ego_resources,
        )
        while True:
            obs = env.observe()
            if obs.get("winner") or obs.get("phase") == "Finished":
                break
            legal = env.legal_actions()
            plan = policy.plan(env, obs, legal)
            decisions.append(
                {
                    "seed": seed,
                    "turn": int(obs.get("turn") or 0),
                    "plan": [canonical(action) for action in plan],
                }
            )
            if not plan:
                break
            info = env.step_turn(plan)
            if not info.get("ok"):
                break
    return decisions


def outcome_consensus(reports: Sequence[Dict[str, Any]]) -> Dict[str, Any]:
    """How much of the win rate is decided by the seed rather than the policy.

    Given per-seed win records of several policies from `tools/eval_checkpoints.py`
    (`win_seeds`), this counts how many policies win each seed.  If almost every
    seed is either won by all policies or lost by all of them, then the outcome is
    a property of the seed (the encounter's coin/speed rolls) and not of the
    policy, which is what a wide "no amount of training moves the number" plateau
    looks like from the outside.
    """
    if not reports:
        return {}
    seed_sets = []
    all_seeds = set()
    for report in reports:
        wins = set(int(seed) for seed in report.get("win_seeds") or [])
        seed_sets.append(wins)
        all_seeds |= set(int(seed) for seed in report.get("seeds") or [])
    if not all_seeds:
        # Fall back to the union of the win seeds when the report has no seed list.
        all_seeds = set().union(*seed_sets) if seed_sets else set()
    counts = [sum(1 for wins in seed_sets if seed in wins) for seed in sorted(all_seeds)]
    n = len(seed_sets)
    histogram = {str(k): counts.count(k) for k in range(n + 1)}
    unanimous = counts.count(0) + counts.count(n)
    unanimous_wins = counts.count(n)
    unanimous_losses = counts.count(0)
    return {
        "policies": n,
        "seeds": len(all_seeds),
        "wins_per_seed_histogram": histogram,
        "all_policies_agree": unanimous,
        "all_agree_rate": unanimous / len(all_seeds) if all_seeds else 0.0,
        "all_won": unanimous_wins,
        "all_lost": unanimous_losses,
        "contested_seeds": len(all_seeds) - unanimous,
        "mean_policies_winning_per_seed": (
            sum(counts) / len(counts) if counts else 0.0
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--a")
    parser.add_argument("--b")
    parser.add_argument("--scenario", default="real")
    parser.add_argument("--seed-start", type=int, default=600001)
    parser.add_argument("--seed-count", type=int, default=60)
    parser.add_argument("--action-effects", action=argparse.BooleanOptionalAction, default=True)
    parser.add_argument("--action-effect-cap", type=int, default=8)
    parser.add_argument("--out", default="")
    parser.add_argument(
        "--consensus",
        nargs="*",
        default=[],
        help="paired-evaluation reports whose per-seed outcomes are compared",
    )
    args = parser.parse_args()

    if args.consensus:
        reports = []
        for entry in args.consensus:
            name, _, raw = entry.partition("=")
            path = Path(raw or name)
            with path.open(encoding="utf-8") as handle:
                payload = json.load(handle)
            arms = payload.get("arms", {})
            if not arms:
                continue
            for arm_name, arm in arms.items():
                seed_count = payload.get("seed_count")
                start = payload.get("seed_start")
                arm = dict(arm)
                if seed_count and start is not None and "seeds" not in arm:
                    arm["seeds"] = list(range(int(start), int(start) + int(seed_count)))
                arm["_policy"] = f"{path.stem}:{arm_name}"
                reports.append(arm)
        summary = outcome_consensus(reports)
        summary["sources"] = [report["_policy"] for report in reports]
        print(json.dumps(summary, ensure_ascii=False, indent=2))
        if args.out:
            out = Path(args.out)
            out.parent.mkdir(parents=True, exist_ok=True)
            with out.open("w", encoding="utf-8") as handle:
                json.dump(summary, handle, ensure_ascii=False, indent=2)
            print(f"wrote {out}")
        return 0

    if not args.a or not args.b:
        raise SystemExit("--a and --b are required unless --consensus is used")

    from lcb.features import Encoder
    from lcb.nn import PolicyValueNet
    from lcb.scenarios import scenario as scenario_of

    encoder = Encoder(include_action_effects=args.action_effects)
    scene = scenario_of(args.scenario)
    seeds = list(range(args.seed_start, args.seed_start + args.seed_count))
    net_a = PolicyValueNet.load(args.a, action_dim=encoder.action_dim)
    net_b = PolicyValueNet.load(args.b, action_dim=encoder.action_dim)

    walk_a = walk(net_a, encoder, scene, seeds, args.action_effect_cap)
    walk_b = walk(net_b, encoder, scene, seeds, args.action_effect_cap)

    total = min(len(walk_a), len(walk_b))
    identical = 0
    actor_matches = 0
    actor_total = 0
    differing_turns: List[Dict[str, Any]] = []
    for left, right in zip(walk_a[:total], walk_b[:total]):
        same_seed = left["seed"] == right["seed"] and left["turn"] == right["turn"]
        if not same_seed:
            continue
        if left["plan"] == right["plan"]:
            identical += 1
        else:
            differing_turns.append({"seed": left["seed"], "turn": left["turn"]})
        for action_left, action_right in zip(left["plan"], right["plan"]):
            actor_total += 1
            if action_left == action_right:
                actor_matches += 1

    payload = {
        "a": args.a,
        "b": args.b,
        "scenario": args.scenario,
        "seeds": {"first": seeds[0], "count": len(seeds)},
        "turns_compared": total,
        "identical_turns": identical,
        "turn_agreement": identical / total if total else 0.0,
        "actor_agreement": actor_matches / actor_total if actor_total else 0.0,
        "differing_turns": differing_turns[:50],
    }
    print(json.dumps(payload, ensure_ascii=False, indent=2))
    if args.out:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        with out.open("w", encoding="utf-8") as handle:
            json.dump(payload, handle, ensure_ascii=False, indent=2)
        print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
