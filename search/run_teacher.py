#!/usr/bin/env python3
"""Stage A runner: collect multi-turn search teacher data (TRAINING_PLAN.md §4).

```bash
python search/run_teacher.py --scenario burst --seed-start 1 --seed-count 200 \
    --horizon 3 --plan-width 32 --turn-width 4 --out data/teacher
```

Writes one `.npz` per seed (so a long run can be resumed) plus
`teacher_index.json` with the per-seed outcome and the search budget, and the
replays of the fastest wins.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
import multiprocessing
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path
from typing import Any, Dict, List, Tuple

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python"))


def _run_one(args: Tuple[int, Dict[str, Any]]) -> Dict[str, Any]:
    seed, cfg = args
    from lcb.dataset import save_dataset
    from lcb.features import Encoder
    from lcb.scenarios import scenario
    from lcb.teacher import BeamTeacher, TeacherConfig, ValueWeights, encode_samples

    encoder = Encoder()
    teacher = BeamTeacher(
        encoder.table,
        encoder,
        TeacherConfig(
            horizon=cfg["horizon"],
            plan_width=cfg["plan_width"],
            candidate_cap=cfg["candidate_cap"],
            turn_width=cfg["turn_width"],
            score_mode=cfg["score_mode"],
            rollout_width=cfg["rollout_width"],
            candidate_mode=cfg["candidate_mode"],
            leaf_value_weight=cfg["leaf_value_weight"],
            counterfactual_credit=cfg["counterfactual_credit"],
            max_turns=cfg["max_turns"],
            enemy_hp_scale=cfg["enemy_hp_scale"],
            infinite_ego_resources=cfg["infinite_ego_resources"],
        ),
        ValueWeights(
            boss_hp=cfg["boss_hp_weight"],
            turn=cfg["turn_weight"],
            ally_damage=cfg["ally_damage_weight"],
            death=cfg["death_weight"],
        ),
    )
    scene = scenario(cfg["scenario"])
    stats, samples, replay = teacher.run_episode(
        seed,
        strict=scene.strict,
        enemies=list(scene.enemies),
        enemy_hp_scale=cfg["enemy_hp_scale"],
        max_turns=cfg["max_turns"],
    )
    out = Path(cfg["out"])
    arrays = encode_samples(
        encoder,
        samples,
        quality_weighting=cfg["quality_weighting"],
        quality_power=cfg["quality_power"],
        failure_weight=cfg["failure_weight"],
        min_quality=cfg["min_quality"],
        credit_weighting=cfg["counterfactual_credit"],
    )
    save_dataset(out / f"seed_{seed:05d}.npz", arrays)
    row = stats.to_row()
    row["search"] = dict(teacher.stats)
    if stats.won:
        (out / "replays").mkdir(parents=True, exist_ok=True)
        with (out / "replays" / f"teacher_seed_{seed:05d}.json").open(
            "w", encoding="utf-8"
        ) as handle:
            json.dump(replay, handle, ensure_ascii=False)
    return row


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenario", default="burst")
    parser.add_argument("--seed-start", type=int, default=1)
    parser.add_argument("--seed-count", type=int, default=40)
    parser.add_argument("--budget", choices=["custom", "t0", "t1", "t2"], default="custom")
    parser.add_argument("--horizon", type=int, default=None)
    parser.add_argument("--plan-width", type=int, default=None)
    parser.add_argument("--candidate-cap", type=int, default=None)
    parser.add_argument("--turn-width", type=int, default=None)
    parser.add_argument("--rollout-width", type=int, default=None)
    parser.add_argument("--score-mode", default="heuristic", choices=["heuristic", "rollout"])
    parser.add_argument("--candidate-mode", default="top", choices=["top", "diverse", "all"])
    parser.add_argument("--leaf-value-weight", type=float, default=0.0)
    parser.add_argument("--boss-hp-weight", type=float, default=100.0)
    parser.add_argument("--turn-weight", type=float, default=0.0)
    parser.add_argument("--ally-damage-weight", type=float, default=30.0)
    parser.add_argument("--death-weight", type=float, default=60.0)
    parser.add_argument(
        "--quality-weighting", action=argparse.BooleanOptionalAction, default=False,
        help="weight failed Teacher rows by generic final-state quality",
    )
    parser.add_argument("--quality-power", type=float, default=2.0)
    parser.add_argument("--failure-weight", type=float, default=0.1)
    parser.add_argument("--min-quality", type=float, default=0.0)
    parser.add_argument(
        "--counterfactual-credit", action=argparse.BooleanOptionalAction, default=False,
        help="compute generic one-actor replacement margins for BC weights",
    )
    parser.add_argument("--max-turns", type=int, default=None)
    parser.add_argument("--enemy-hp-scale", type=float, default=None,
                        help="overrides the scenario's own knobs when given")
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--out", default=str(ROOT / "data" / "teacher"))
    parser.add_argument(
        "--infinite-ego-resources", action=argparse.BooleanOptionalAction, default=None,
        help="override the scenario's E.G.O resource setting",
    )
    args = parser.parse_args()

    from lcb.scenarios import scenario as get_scenario

    from lcb.teacher import teacher_budget

    scene = get_scenario(args.scenario)
    preset = teacher_budget(args.budget) if args.budget != "custom" else {}
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    cfg = {
        "enemy_hp_scale": (
            args.enemy_hp_scale if args.enemy_hp_scale is not None else scene.enemy_hp_scale
        ),
        "scenario": args.scenario,
        "budget": args.budget,
        "horizon": args.horizon if args.horizon is not None else preset.get("horizon", 3),
        "plan_width": args.plan_width if args.plan_width is not None else preset.get("plan_width", 32),
        "candidate_cap": args.candidate_cap if args.candidate_cap is not None else preset.get("candidate_cap", 6),
        "turn_width": args.turn_width if args.turn_width is not None else preset.get("turn_width", 4),
        "rollout_width": args.rollout_width if args.rollout_width is not None else preset.get("rollout_width", 4),
        "score_mode": args.score_mode,
        "candidate_mode": args.candidate_mode,
        "leaf_value_weight": args.leaf_value_weight,
        "boss_hp_weight": args.boss_hp_weight,
        "turn_weight": args.turn_weight,
        "ally_damage_weight": args.ally_damage_weight,
        "death_weight": args.death_weight,
        "quality_weighting": args.quality_weighting,
        "quality_power": args.quality_power,
        "failure_weight": args.failure_weight,
        "min_quality": args.min_quality,
        "counterfactual_credit": args.counterfactual_credit,
        "max_turns": args.max_turns if args.max_turns is not None else scene.max_turns,
        "out": str(out),
        "infinite_ego_resources": (
            args.infinite_ego_resources
            if args.infinite_ego_resources is not None
            else scene.infinite_ego_resources
        ),
    }
    seeds = list(range(args.seed_start, args.seed_start + args.seed_count))
    started = time.time()
    rows: List[Dict[str, Any]] = []
    if args.jobs > 1:
        with ProcessPoolExecutor(
            max_workers=args.jobs, mp_context=multiprocessing.get_context("spawn")
        ) as pool:
            for row in pool.map(_run_one, [(seed, cfg) for seed in seeds]):
                rows.append(row)
                print(
                    f"seed {row['seed']}: won={row['won']} turns={row['turns']} "
                    f"damage={row['damage_to_enemies']:.0f}",
                    flush=True,
                )
    else:
        for seed in seeds:
            row = _run_one((seed, cfg))
            rows.append(row)
            print(
                f"seed {row['seed']}: won={row['won']} turns={row['turns']} "
                f"damage={row['damage_to_enemies']:.0f}",
                flush=True,
            )

    from lcb.evaluate import best_of_n, provenance, summarise

    index = {
        "provenance": provenance(scene, seeds, {"teacher": cfg}),
        "elapsed_seconds": round(time.time() - started, 1),
        "summary": summarise(rows),
        "best_of_n": best_of_n(rows, (1, 10, 50)),
        "episodes": rows,
    }
    with (out / "teacher_index.json").open("w", encoding="utf-8") as handle:
        json.dump(index, handle, ensure_ascii=False, indent=2)
    print(json.dumps(index["summary"], ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
