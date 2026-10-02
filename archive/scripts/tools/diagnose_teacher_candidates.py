#!/usr/bin/env python3
"""Measure first-turn candidate recall against an uncapped Teacher.

This is a diagnostic, not a training path. The uncapped Teacher supplies a
reference first-turn plan from the same initial state; the limited Teacher is
then checked for whether each reference action survived its candidate filter.
No status, identity, or E.G.O route is encoded in the metric.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Dict, List

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python"))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenario", default="real")
    parser.add_argument("--seed-start", type=int, default=92001)
    parser.add_argument("--seed-count", type=int, default=50)
    parser.add_argument("--candidate-mode", choices=["top", "diverse"], default="top")
    parser.add_argument("--candidate-cap", type=int, default=6)
    parser.add_argument("--horizon", type=int, default=1)
    parser.add_argument("--plan-width", type=int, default=32)
    parser.add_argument("--turn-width", type=int, default=4)
    parser.add_argument("--leaf-value-weight", type=float, default=1.0)
    parser.add_argument("--turns", type=int, default=1)
    parser.add_argument("--out", default="")
    args = parser.parse_args()

    from lcb.env import LimbusEnv
    from lcb.features import Encoder
    from lcb.plans import canonical
    from lcb.scenarios import scenario
    from lcb.teacher import BeamTeacher, TeacherConfig

    scene = scenario(args.scenario)
    encoder = Encoder()

    def make_teacher(mode: str, cap: int) -> BeamTeacher:
        return BeamTeacher(
            encoder.table,
            encoder,
            TeacherConfig(
                horizon=args.horizon,
                plan_width=args.plan_width,
                candidate_cap=cap,
                turn_width=args.turn_width,
                score_mode="heuristic",
                candidate_mode=mode,
                leaf_value_weight=args.leaf_value_weight,
                max_turns=scene.max_turns,
                enemy_hp_scale=scene.enemy_hp_scale,
                infinite_ego_resources=scene.infinite_ego_resources,
            ),
        )

    rows: List[Dict[str, Any]] = []
    action_total = 0
    action_retained = 0
    exact_plans = 0
    for seed in range(args.seed_start, args.seed_start + args.seed_count):
        reference = make_teacher("all", 0)
        limited = make_teacher(args.candidate_mode, args.candidate_cap)
        reference_env = LimbusEnv(strict=scene.strict)
        limited_env = LimbusEnv(strict=scene.strict)
        reset = dict(
            enemies=list(scene.enemies),
            max_turns=scene.max_turns,
            enemy_hp_scale=scene.enemy_hp_scale,
            infinite_ego_resources=scene.infinite_ego_resources,
        )
        reference_env.reset(seed, **reset)
        episode_steps = 0
        episode_exact = 0
        episode_retained = 0
        episode_actions = 0
        while episode_steps < args.turns:
            reference_obs = reference_env.observe()
            if reference_obs.get("winner") or reference_obs.get("phase") == "Finished":
                break
            limited_env = reference_env.clone_state()
            limited = make_teacher(args.candidate_mode, args.candidate_cap)
            reference_plan, _value, _groups, actors = reference.plan_turn(
                reference_env, reference_obs, reference_env.legal_actions()
            )
            limited_plan, _value, _groups, _actors = limited.plan_turn(
                limited_env, limited_env.observe(), limited_env.legal_actions()
            )
            retained = 0
            for actor, action in zip(actors, reference_plan):
                episode_actions += 1
                action_total += 1
                options = limited.last_candidate_groups.get(actor, [])
                if canonical(action) in {canonical(option) for option in options}:
                    retained += 1
                    episode_retained += 1
                    action_retained += 1
            same_plan = [canonical(action) for action in reference_plan] == [
                canonical(action) for action in limited_plan
            ]
            episode_exact += int(same_plan)
            if not reference_plan or not reference_env.step_turn(reference_plan).get("ok"):
                break
            episode_steps += 1
        exact_plans += episode_exact
        rows.append(
            {
                "seed": seed,
                "steps": episode_steps,
                "reference_plan_actions": episode_actions,
                "retained_actions": episode_retained,
                "action_recall": episode_retained / episode_actions if episode_actions else 0.0,
                "exact_plan_rate": episode_exact / episode_steps if episode_steps else 0.0,
                "limited_search": dict(limited.stats) if episode_steps else {},
            }
        )

    report = {
        "scenario": scene.to_dict(),
        "reference": {"candidate_mode": "all", "candidate_cap": 0},
        "limited": {
            "candidate_mode": args.candidate_mode,
            "candidate_cap": args.candidate_cap,
            "horizon": args.horizon,
            "plan_width": args.plan_width,
            "turn_width": args.turn_width,
            "leaf_value_weight": args.leaf_value_weight,
        },
        "seeds": {
            "first": args.seed_start,
            "last": args.seed_start + args.seed_count - 1,
            "count": args.seed_count,
        },
        "action_recall": action_retained / action_total if action_total else 0.0,
        "exact_plan_rate": exact_plans / sum(row["steps"] for row in rows) if rows and sum(row["steps"] for row in rows) else 0.0,
        "action_total": action_total,
        "action_retained": action_retained,
        "episodes": rows,
    }
    text = json.dumps(report, ensure_ascii=False, indent=2)
    if args.out:
        Path(args.out).parent.mkdir(parents=True, exist_ok=True)
        Path(args.out).write_text(text + "\n", encoding="utf-8")
    print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
