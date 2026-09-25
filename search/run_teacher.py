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
    from lcb.teacher import BeamTeacher, TeacherConfig, encode_samples

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
            max_turns=cfg["max_turns"],
            enemy_hp_scale=cfg["enemy_hp_scale"],
            infinite_ego_resources=cfg["infinite_ego_resources"],
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
    arrays = encode_samples(encoder, samples)
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
    parser.add_argument("--horizon", type=int, default=3)
    parser.add_argument("--plan-width", type=int, default=32)
    parser.add_argument("--candidate-cap", type=int, default=6)
    parser.add_argument("--turn-width", type=int, default=4)
    parser.add_argument("--rollout-width", type=int, default=4)
    parser.add_argument("--score-mode", default="heuristic", choices=["heuristic", "rollout"])
    parser.add_argument("--max-turns", type=int, default=12)
    parser.add_argument("--enemy-hp-scale", type=float, default=None,
                        help="overrides the scenario's own knobs when given")
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--out", default=str(ROOT / "data" / "teacher"))
    parser.add_argument("--infinite-ego-resources", action="store_true")
    args = parser.parse_args()

    from lcb.scenarios import scenario as get_scenario

    scene = get_scenario(args.scenario)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    cfg = {
        "enemy_hp_scale": (
            args.enemy_hp_scale if args.enemy_hp_scale is not None else scene.enemy_hp_scale
        ),
        "scenario": args.scenario,
        "horizon": args.horizon,
        "plan_width": args.plan_width,
        "candidate_cap": args.candidate_cap,
        "turn_width": args.turn_width,
        "rollout_width": args.rollout_width,
        "score_mode": args.score_mode,
        "max_turns": args.max_turns,
        "out": str(out),
        "infinite_ego_resources": args.infinite_ego_resources,
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
