#!/usr/bin/env python3
"""Run the known RR6 Sinking setup as a scripted forward-model oracle.

This is an analysis benchmark, not a policy and not a training-data generator:
Ishmael Bygone Days OC + Rodion Rime Shank OC, followed by Yi Sang Bygone Days,
then the existing generic Greedy fallback.  Every action is still selected from
`LimbusEnv.legal_actions()`, and every turn resolves through `step_turn`.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Dict, List, Optional

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python"))

from lcb.baselines import GreedyPolicy  # noqa: E402
from lcb.env import EgoAction, LimbusEnv  # noqa: E402
from lcb.features import Encoder  # noqa: E402
from lcb.plans import canonical, first_legal_plan  # noqa: E402
from lcb.scenarios import scenario  # noqa: E402


def unit_for_identity(obs: Dict[str, Any], identity: str) -> Optional[str]:
    for unit in obs.get("units", []):
        if unit.get("identity") == identity and unit.get("kind") == "sinner":
            return str(unit["id"])
    return None


def fallback_plan(env: LimbusEnv, overrides: Dict[str, EgoAction]) -> List[Any]:
    """Use the first legal action for everyone except explicitly scripted actors."""
    legal = env.legal_actions()
    plan = first_legal_plan(env.observe(), legal)
    by_actor = {getattr(action, "actor", None): action for action in plan}
    by_actor.update(overrides)
    return [by_actor[getattr(action, "actor", None)] for action in plan]


def choose_ego(env: LimbusEnv, actor: str, ego: str, kind: str) -> EgoAction:
    matches = [
        action for action in env.legal_actions()
        if isinstance(action, EgoAction)
        and action.actor == actor
        and action.ego == ego
        and action.kind == kind
    ]
    if not matches:
        raise RuntimeError(f"no legal {kind} {ego} for actor {actor}")
    return matches[0]


def run(seed: int, scenario_name: str, yi_kind: str, max_turns: int) -> Dict[str, Any]:
    scene = scenario(scenario_name)
    env = LimbusEnv(strict=scene.strict)
    env.reset(
        seed,
        enemies=list(scene.enemies),
        max_turns=max_turns,
        enemy_hp_scale=scene.enemy_hp_scale,
        infinite_ego_resources=True,
    )
    encoder = Encoder()
    greedy = GreedyPolicy(encoder.table, cap=6)
    greedy.start_episode(env.observe())
    records: List[Dict[str, Any]] = []

    for turn in range(1, max_turns + 1):
        before = env.observe()
        if before.get("winner"):
            break
        ishmael = unit_for_identity(before, "10813")
        rodion = unit_for_identity(before, "10913")
        yi = unit_for_identity(before, "10110")
        if not all((ishmael, rodion, yi)):
            raise RuntimeError("the fixed oracle team is missing Ishmael/Rodion/Yi Sang")

        overrides: Dict[str, EgoAction] = {}
        if turn == 1:
            overrides[ishmael] = choose_ego(env, ishmael, "20807", "Overclock")
            overrides[rodion] = choose_ego(env, rodion, "20903", "Overclock")
        elif turn == 2:
            overrides[yi] = choose_ego(env, yi, "20106", yi_kind)

        if overrides:
            plan = fallback_plan(env, overrides)
        else:
            plan = greedy.plan(env, env.observe(), env.legal_actions())
        result = env.step_turn(plan)
        if not result.get("ok"):
            raise RuntimeError(f"oracle turn {turn} rejected: {result.get('error')}")
        after = env.observe()
        boss = next(
            (u for u in after.get("units", []) if str(u.get("id", "")).endswith("-9567")), None
        )
        sinking = ((boss or {}).get("statuses") or {}).get("Sinking") or {}
        records.append(
            {
                "turn": turn,
                "phase": "ishmael_rodion_setup" if turn == 1 else "yi_sang_setup" if turn == 2 else "greedy_burst",
                "plan": [json.loads(canonical(action)) for action in plan],
                "state_hash_before": result.get("state_hash_before"),
                "state_hash_after": result.get("state_hash_after"),
                "transition_hash": result.get("transition_hash"),
                "search_key": result.get("search_key"),
                "stats": result.get("stats"),
                "boss_hp": (boss or {}).get("hp"),
                "boss_sinking": {
                    "potency": sinking.get("potency", 0),
                    "count": sinking.get("count", 0),
                },
            }
        )

    final = env.observe()
    boss = next((u for u in final.get("units", []) if str(u.get("id", "")).endswith("-9567")), None)
    stats = [record.get("stats") or {} for record in records]
    return {
        "policy": "KnownHumanSetupOracle",
        "scenario": scene.to_dict() | {"infinite_ego_resources": True},
        "seed": seed,
        "winner": final.get("winner"),
        "won": final.get("winner") == "Sinners",
        "kill_turn": records[-1]["turn"] if final.get("winner") == "Sinners" and records else None,
        "boss_hp_left": (boss or {}).get("hp"),
        "peak_sinking_potency": max((r["boss_sinking"]["potency"] for r in records), default=0),
        "peak_sinking_count": max((r["boss_sinking"]["count"] for r in records), default=0),
        "total_sinking_damage": sum(float(s.get("sinking_damage") or 0) for s in stats),
        "total_direct_damage": sum(float(s.get("damage_to_enemies") or 0) for s in stats),
        "turns": records,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenario", default="real", choices=("real", "short", "burst"))
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--yi-kind", choices=("Awakening", "Overclock"), default="Awakening")
    parser.add_argument("--max-turns", type=int, default=30)
    parser.add_argument("--out", type=Path, default=ROOT / "reports" / "human_setup_oracle.json")
    args = parser.parse_args()
    result = run(args.seed, args.scenario, args.yi_kind, args.max_turns)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({k: result[k] for k in ("scenario", "seed", "won", "kill_turn", "boss_hp_left", "peak_sinking_potency", "peak_sinking_count", "total_sinking_damage", "total_direct_damage")}, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
