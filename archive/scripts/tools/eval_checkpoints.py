#!/usr/bin/env python3
"""Paired evaluation of several checkpoints on one identical seed band.

`eval/run_eval.py` evaluates one policy per invocation and writes the whole
protocol report.  When the question is "did this training change help?", the
useful statistic is the *paired* one: the same seeds, two (or more) checkpoints,
and the discordant counts (`A` wins where `B` loses, and the reverse) instead of
two independent win rates with overlapping confidence intervals.

This tool loads any number of checkpoints and reports, per checkpoint:

* wins / episodes and the Wilson 95% interval,
* the kill-turn summary of the wins,
* the paired discordant counts and an exact McNemar p-value against the first
  checkpoint (the reference).

It never trains and never writes a replay, so it is cheap to run repeatedly on a
fixed band while iterating.

```bash
python tools/eval_checkpoints.py --scenario real \
    --seed-start 43001 --seed-count 500 --reference baseline \
    --checkpoint baseline=models/a.npz \
    --checkpoint candidate=models/b.npz \
    --out reports/paired_eval_ab.json
```
"""

from __future__ import annotations

import argparse
import json
import math
import sys
import time
from pathlib import Path
from typing import Any, Dict, List, Optional

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python"))


def wilson(wins: int, total: int, z: float = 1.959963984540054) -> List[float]:
    if total <= 0:
        return [0.0, 0.0]
    p = wins / total
    denom = 1.0 + z * z / total
    centre = p + z * z / (2 * total)
    spread = z * math.sqrt(p * (1 - p) / total + z * z / (4 * total * total))
    return [max(0.0, (centre - spread) / denom), min(1.0, (centre + spread) / denom)]


def mcnemar_exact(b: int, c: int) -> float:
    """Two-sided exact McNemar p-value for `b`/`c` discordant pairs."""
    n = b + c
    if n == 0:
        return 1.0
    k = min(b, c)
    tail = sum(math.comb(n, i) for i in range(0, k + 1)) / (2.0**n)
    return float(min(1.0, 2.0 * tail))


def evaluate_checkpoint(
    name: str,
    path: Path,
    scenario_name: str,
    seeds: List[int],
    encoder,
    action_effects: bool,
    action_effect_cap: int,
    record_seeds: bool = True,
    lookahead: int = 0,
    beam: int = 8,
    branch: int = 3,
    horizon: int = 1,
) -> Dict[str, Any]:
    from lcb.baselines import NeuralLookaheadPolicy, NeuralPolicy
    from lcb.evaluate import run_episode
    from lcb.nn import PolicyValueNet
    from lcb.scenarios import scenario as scenario_of

    scene = scenario_of(scenario_name)
    net = PolicyValueNet.load(path, action_dim=encoder.action_dim)
    policy = NeuralPolicy(
        net,
        encoder,
        sample=False,
        name=name,
        action_effects=action_effects,
        action_effect_cap=action_effect_cap,
    )
    if lookahead:
        policy = NeuralLookaheadPolicy(
            net,
            encoder,
            beam=beam,
            branch=branch,
            horizon=horizon,
            name=name,
            action_effects=action_effects,
            action_effect_cap=action_effect_cap,
        )
    wins: List[int] = []
    kill_turns: List[int] = []
    boss_left: List[float] = []
    damage: List[float] = []
    survivors: List[float] = []
    winners: Dict[str, int] = {}
    lookahead_switches = 0
    for seed in seeds:
        record = run_episode(policy, scene, seed, record_replay=False)
        if record.error:
            raise RuntimeError(f"{name} seed {seed}: {record.error}")
        if getattr(policy, "last_plan_source", "") == "lookahead":
            lookahead_switches += 1
        stats = record.stats
        won = bool(stats.get("won"))
        if won:
            wins.append(seed)
            kill_turns.append(int(stats.get("kill_turn") or 0))
        boss_left.append(float(stats.get("boss_hp_left") or 0.0))
        damage.append(float(stats.get("damage_to_enemies") or 0.0))
        survivors.append(float(stats.get("survivors") or 0.0))
        key = str(stats.get("winner"))
        winners[key] = winners.get(key, 0) + 1
    payload: Dict[str, Any] = {
        "checkpoint": str(path),
        "episodes": len(seeds),
        "wins": len(wins),
        "win_rate": len(wins) / len(seeds) if seeds else 0.0,
        "wilson_95": wilson(len(wins), len(seeds)),
        "winners": winners,
        "boss_hp_left_median": _median(boss_left),
        "damage_to_enemies_mean": sum(damage) / len(damage) if damage else 0.0,
        "survivors_mean": sum(survivors) / len(survivors) if survivors else 0.0,
        "kill_turn_median": _median([float(k) for k in kill_turns]),
        "kill_turn_p25": _quantile([float(k) for k in kill_turns], 0.25),
        "fastest_kill_turn": min(kill_turns) if kill_turns else None,
    }
    if lookahead:
        payload["lookahead"] = {
            "beam": beam,
            "branch": branch,
            "horizon": horizon,
            "episodes_using_lookahead_plan": lookahead_switches,
        }
    if record_seeds:
        payload["win_seeds"] = wins
    return payload


def _median(values: List[float]) -> Optional[float]:
    return _quantile(values, 0.5)


def _quantile(values: List[float], q: float) -> Optional[float]:
    if not values:
        return None
    ordered = sorted(values)
    if len(ordered) == 1:
        return ordered[0]
    position = q * (len(ordered) - 1)
    low = int(math.floor(position))
    high = int(math.ceil(position))
    if low == high:
        return ordered[low]
    weight = position - low
    return ordered[low] * (1 - weight) + ordered[high] * weight


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenario", default="real")
    parser.add_argument("--seed-start", type=int, default=43001)
    parser.add_argument("--seed-count", type=int, default=100)
    parser.add_argument(
        "--checkpoint",
        action="append",
        required=True,
        help="name=path (repeatable); the first one is the paired reference",
    )
    parser.add_argument("--reference", default="", help="name of the reference arm")
    parser.add_argument(
        "--action-effects", action=argparse.BooleanOptionalAction, default=None,
        help="default: read the feature switches from each checkpoint's own report",
    )
    parser.add_argument("--action-effect-cap", type=int, default=8)
    parser.add_argument("--lookahead", action=argparse.BooleanOptionalAction, default=False)
    parser.add_argument("--beam", type=int, default=8)
    parser.add_argument("--branch", type=int, default=3)
    parser.add_argument("--horizon", type=int, default=1)
    parser.add_argument("--out", default="")
    args = parser.parse_args()

    from lcb.features import Encoder, encoder_for_checkpoint

    arms: List[tuple] = []
    for entry in args.checkpoint:
        if "=" not in entry:
            raise SystemExit(f"--checkpoint needs name=path, got {entry!r}")
        name, _, raw = entry.partition("=")
        arms.append((name.strip(), Path(raw.strip())))
    reference_name = args.reference or arms[0][0]
    if reference_name not in {name for name, _ in arms}:
        raise SystemExit(f"reference {reference_name!r} is not one of the arms")

    # Each arm is evaluated with the encoder its own report records, so arms with
    # different feature widths can be compared on the same seeds.  A silent width
    # mismatch is impossible: `PolicyValueNet.load` rejects a narrower checkpoint.
    encoder = encoder_for_checkpoint(arms[0][1])
    if args.action_effects is not None:
        encoder = Encoder(
            include_action_effects=args.action_effects,
            include_action_ids=bool(getattr(encoder, "include_action_ids", False)),
            include_target_slots=bool(getattr(encoder, "include_target_slots", False)),
        )
    seeds = list(range(args.seed_start, args.seed_start + args.seed_count))
    started = time.time()
    results: Dict[str, Any] = {}
    for name, path in arms:
        if not path.exists():
            raise SystemExit(f"missing checkpoint {path}")
        arm_started = time.time()
        arm_encoder = (
            encoder if args.action_effects is not None else encoder_for_checkpoint(path)
        )
        print(
            f"evaluating {name} on {len(seeds)} seeds ({args.scenario}, "
            f"action_dim={arm_encoder.action_dim})...",
            flush=True,
        )
        payload = evaluate_checkpoint(
            name,
            path,
            args.scenario,
            seeds,
            arm_encoder,
            arm_encoder.include_action_effects,
            args.action_effect_cap,
            lookahead=args.lookahead,
            beam=args.beam,
            branch=args.branch,
            horizon=args.horizon,
        )
        payload["action_dim"] = arm_encoder.action_dim
        payload["elapsed_seconds"] = round(time.time() - arm_started, 1)
        results[name] = payload
        print(
            f"  {name}: {payload['wins']}/{payload['episodes']} "
            f"({payload['win_rate']:.3f}) median kill {payload['kill_turn_median']}",
            flush=True,
        )

    reference_wins = set(results[reference_name]["win_seeds"])
    paired: Dict[str, Any] = {}
    for name in results:
        if name == reference_name:
            continue
        wins = set(results[name]["win_seeds"])
        both = len(reference_wins & wins)
        only_ref = len(reference_wins - wins)
        only_arm = len(wins - reference_wins)
        neither = len(seeds) - both - only_ref - only_arm
        paired[name] = {
            "reference": reference_name,
            "both_won": both,
            "reference_only": only_ref,
            "arm_only": only_arm,
            "both_lost": neither,
            "net_gain": only_arm - only_ref,
            "mcnemar_exact_p": mcnemar_exact(only_ref, only_arm),
        }
        print(
            f"  paired {name} vs {reference_name}: +{only_arm} / -{only_ref} "
            f"(net {only_arm - only_ref:+d}, p={paired[name]['mcnemar_exact_p']:.4f})",
            flush=True,
        )

    output = {
        "scenario": args.scenario,
        "seed_start": args.seed_start,
        "seed_count": args.seed_count,
        "reference": reference_name,
        "reference_action_dim": results[reference_name]["action_dim"],
        "protocol": {
            "strict": True,
            "infinite_ego_resources": True,
            "action_effects": args.action_effects,
            "action_effect_cap": args.action_effect_cap,
            "argmax": True,
            "lookahead": (
                {"beam": args.beam, "branch": args.branch, "horizon": args.horizon}
                if args.lookahead
                else None
            ),
        },
        "elapsed_seconds": round(time.time() - started, 1),
        "arms": results,
        "paired": paired,
    }
    if args.out:
        out = Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        with out.open("w", encoding="utf-8") as handle:
            json.dump(output, handle, ensure_ascii=False, indent=2)
        print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
