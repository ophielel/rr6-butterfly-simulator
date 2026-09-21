#!/usr/bin/env python3
"""Label the states the policy reaches, with the stage-A teacher (see lcb.dagger).

```bash
python search/run_dagger.py --checkpoint models/bc_tree.npz --out data/dagger1 \
    --scenario burst --seed-start 2001 --seed-count 100 --policy-prob 0.5 --jobs 8
```
"""

from __future__ import annotations

import argparse
import json
import multiprocessing
import sys
import time
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path
from typing import Any, Dict, Tuple

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python"))


def _run_one(args: Tuple[int, Dict[str, Any]]) -> Dict[str, Any]:
    seed, cfg = args
    from lcb.baselines import NeuralPolicy
    from lcb.dagger import collect_episode
    from lcb.dataset import save_dataset
    from lcb.features import Encoder
    from lcb.nn import PolicyValueNet
    from lcb.scenarios import scenario
    from lcb.teacher import BeamTeacher, TeacherConfig, encode_samples

    encoder = Encoder()
    net = PolicyValueNet.load(cfg["checkpoint"])
    policy = NeuralPolicy(net, encoder, sample=True, seed=seed, name="BC")
    teacher = BeamTeacher(
        encoder.table,
        encoder,
        TeacherConfig(
            horizon=cfg["horizon"],
            plan_width=cfg["plan_width"],
            candidate_cap=cfg["candidate_cap"],
            turn_width=cfg["turn_width"],
            max_turns=cfg["max_turns"],
            enemy_hp_scale=cfg["enemy_hp_scale"],
        ),
    )
    scene = scenario(cfg["scenario"])
    stats, samples, _replay = collect_episode(
        policy, teacher, encoder, seed, scene, policy_prob=cfg["policy_prob"]
    )
    out = Path(cfg["out"])
    save_dataset(out / f"seed_{seed:05d}.npz", encode_samples(encoder, samples))
    return stats.to_row()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkpoint", default=str(ROOT / "models" / "bc_tree.npz"))
    parser.add_argument("--scenario", default="burst")
    parser.add_argument("--seed-start", type=int, default=2001)
    parser.add_argument("--seed-count", type=int, default=100)
    parser.add_argument("--policy-prob", type=float, default=0.5)
    parser.add_argument("--horizon", type=int, default=3)
    parser.add_argument("--plan-width", type=int, default=32)
    parser.add_argument("--candidate-cap", type=int, default=6)
    parser.add_argument("--turn-width", type=int, default=4)
    parser.add_argument("--max-turns", type=int, default=12)
    parser.add_argument("--enemy-hp-scale", type=float, default=None)
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--out", default=str(ROOT / "data" / "dagger1"))
    args = parser.parse_args()

    from lcb.evaluate import best_of_n, provenance, summarise
    from lcb.scenarios import scenario

    scene = scenario(args.scenario)
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    cfg = {
        "checkpoint": args.checkpoint,
        "scenario": args.scenario,
        "policy_prob": args.policy_prob,
        "horizon": args.horizon,
        "plan_width": args.plan_width,
        "candidate_cap": args.candidate_cap,
        "turn_width": args.turn_width,
        "max_turns": args.max_turns,
        "enemy_hp_scale": (
            args.enemy_hp_scale if args.enemy_hp_scale is not None else scene.enemy_hp_scale
        ),
        "out": str(out),
    }
    seeds = list(range(args.seed_start, args.seed_start + args.seed_count))
    started = time.time()
    if args.jobs > 1:
        with ProcessPoolExecutor(
            max_workers=args.jobs, mp_context=multiprocessing.get_context("spawn")
        ) as pool:
            rows = list(pool.map(_run_one, [(seed, cfg) for seed in seeds]))
    else:
        rows = [_run_one((seed, cfg)) for seed in seeds]
    index = {
        "provenance": provenance(scene, seeds, {"dagger": cfg}),
        "elapsed_seconds": round(time.time() - started, 1),
        "summary": summarise(rows),
        "best_of_n": best_of_n(rows, (1, 10, 50)),
        "episodes": rows,
    }
    with (out / "dagger_index.json").open("w", encoding="utf-8") as handle:
        json.dump(index, handle, ensure_ascii=False, indent=2)
    print(json.dumps(index["summary"], ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
