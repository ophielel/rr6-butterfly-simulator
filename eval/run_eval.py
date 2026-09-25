#!/usr/bin/env python3
"""Stage D runner: the evaluation protocol (TRAINING_PLAN.md §7).

```bash
python eval/run_eval.py --scenario burst --seed-start 9001 --seed-count 60 \
    --policies Random FirstLegal Greedy AI --checkpoint models/ppo.npz --jobs 8
```

Writes `reports/<scenario>_<policy>.jsonl` (one row per episode), a per-policy
summary, `reports/evaluation.json` (all policies together, with provenance) and
the replay of the fastest win of every policy into `replays/`.
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


def _worker(args: Tuple[str, Dict[str, Any], int]) -> Dict[str, Any]:
    policy_name, cfg, seed = args
    from lcb.baselines import (
        FirstLegalPolicy,
        GreedyPolicy,
        NeuralPolicy,
        RandomPolicy,
        TeacherPolicy,
    )
    from lcb.evaluate import run_episode
    from lcb.features import Encoder
    from lcb.scenarios import scenario

    encoder = Encoder()
    if policy_name == "Random":
        policy = RandomPolicy(seed=cfg["seed"])
    elif policy_name == "FirstLegal":
        policy = FirstLegalPolicy()
    elif policy_name == "Greedy":
        policy = GreedyPolicy(encoder.table, cap=cfg["greedy_cap"])
    elif policy_name == "Teacher":
        from lcb.teacher import BeamTeacher, TeacherConfig

        scene = scenario(cfg["scenario"])
        teacher = BeamTeacher(
            encoder.table,
            encoder,
            TeacherConfig(
                horizon=cfg["teacher_horizon"],
                plan_width=cfg["teacher_plan_width"],
                candidate_cap=cfg["teacher_candidate_cap"],
                turn_width=cfg["teacher_turn_width"],
                max_turns=scene.max_turns,
                enemy_hp_scale=scene.enemy_hp_scale,
                infinite_ego_resources=scene.infinite_ego_resources,
            ),
        )
        policy = TeacherPolicy(encoder.table, encoder, teacher)
    elif policy_name in ("AI", "BC", "PPO"):
        from lcb.nn import PolicyValueNet

        net = PolicyValueNet.load(cfg["checkpoint"])
        policy = NeuralPolicy(net, encoder, sample=cfg["sample"], seed=seed, name=policy_name)
    else:  # pragma: no cover - guarded by argparse
        raise ValueError(policy_name)

    record = run_episode(policy, scenario(cfg["scenario"]), seed, record_replay=True)
    return {
        "policy": record.policy,
        "seed": seed,
        "scenario_config": scenario(cfg["scenario"]).to_dict(),
        "stats": record.stats,
        "replay": record.replay,
        "error": record.error,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenario", default="burst")
    parser.add_argument("--seed-start", type=int, default=9001)
    parser.add_argument("--seed-count", type=int, default=40)
    parser.add_argument(
        "--policies", nargs="+",
        default=["Random", "FirstLegal", "Greedy", "Teacher", "BC", "PPO"],
        choices=["Random", "FirstLegal", "Greedy", "Teacher", "AI", "BC", "PPO"],
    )
    parser.add_argument("--checkpoint", default=str(ROOT / "models" / "ppo.npz"))
    parser.add_argument("--greedy-cap", type=int, default=6)
    parser.add_argument("--teacher-horizon", type=int, default=3)
    parser.add_argument("--teacher-plan-width", type=int, default=32)
    parser.add_argument("--teacher-candidate-cap", type=int, default=6)
    parser.add_argument("--teacher-turn-width", type=int, default=4)
    parser.add_argument("--short-turn", type=int, default=0, help="0 = take the Greedy median")
    parser.add_argument("--sample", action="store_true", help="sample instead of argmax")
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--out", default=str(ROOT / "reports"))
    parser.add_argument("--replays", default=str(ROOT / "replays"))
    args = parser.parse_args()

    from lcb.evaluate import best_of_n, provenance, replay_index, restart_aware, summarise
    from lcb.scenarios import scenario

    scene = scenario(args.scenario)
    seeds = list(range(args.seed_start, args.seed_start + args.seed_count))
    cfg = {
        "scenario": args.scenario,
        "greedy_cap": args.greedy_cap,
        "checkpoint": args.checkpoint,
        "sample": args.sample,
        "seed": args.seed,
        "teacher_horizon": args.teacher_horizon,
        "teacher_plan_width": args.teacher_plan_width,
        "teacher_candidate_cap": args.teacher_candidate_cap,
        "teacher_turn_width": args.teacher_turn_width,
    }
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    replays_dir = Path(args.replays)
    replays_dir.mkdir(parents=True, exist_ok=True)

    results: Dict[str, List[Dict[str, Any]]] = {}
    replays: Dict[str, Dict[str, Any]] = {}
    started = time.time()
    for policy_name in args.policies:
        rows: List[Dict[str, Any]] = []
        jobs = [(policy_name, cfg, seed) for seed in seeds]
        if args.jobs > 1:
            with ProcessPoolExecutor(
                max_workers=args.jobs,
                mp_context=multiprocessing.get_context("spawn"),
            ) as pool:
                for result in pool.map(_worker, jobs):
                    rows.append(result)
        else:
            for job in jobs:
                rows.append(_worker(job))
        rows.sort(key=lambda row: row["seed"])
        star = policy_name + ("*" if args.sample else "")
        results[policy_name] = rows
        with (out / f"{args.scenario}_{policy_name}.jsonl").open("w", encoding="utf-8") as handle:
            for row in rows:
                handle.write(json.dumps(row["stats"], ensure_ascii=False) + "\n")
        wins = [row for row in rows if row["stats"].get("won")]
        if wins:
            best = min(wins, key=lambda row: row["stats"]["kill_turn"] or 10**6)
            replays[policy_name] = best
            with (replays_dir / f"{args.scenario}_{policy_name}_seed{best['seed']}.json").open(
                "w", encoding="utf-8"
            ) as handle:
                json.dump(
                    {
                        "policy": policy_name,
                        "scenario": args.scenario,
                        "scenario_config": best["scenario_config"],
                        "seed": best["seed"],
                        "stats": best["stats"],
                        "turns": best["replay"],
                    },
                    handle,
                    ensure_ascii=False,
                    indent=1,
                )
        print(
            f"{star}: {len(wins)}/{len(rows)} wins, "
            f"damage={sum(r['stats']['damage_to_enemies'] for r in rows)/max(1,len(rows)):.0f}",
            flush=True,
        )

    short_turn = args.short_turn or 0
    if not short_turn:
        greedy_rows = [row["stats"] for row in results.get("Greedy", [])]
        greedy_summary = summarise(greedy_rows) if greedy_rows else {}
        median = greedy_summary.get("kill_turn_median")
        short_turn = int(median) if median else 12

    payload: Dict[str, Any] = {
        "provenance": provenance(
            scene,
            seeds,
            {
                "elapsed_seconds": round(time.time() - started, 1),
                "restarts": 1,
                "best_of_n": [1, 10, 50],
                "policies": args.policies,
                "checkpoint": args.checkpoint,
                "greedy_cap": args.greedy_cap,
            },
        ),
        "short_turn_threshold": short_turn,
        "policies": {},
    }
    for policy_name, rows in results.items():
        stats = [row["stats"] for row in rows]
        payload["policies"][policy_name] = {
            "summary": summarise(stats, short_turn=short_turn),
            "best_of_n": best_of_n(stats, (1, 10, 50)),
            "restart_aware": restart_aware(stats, turn_threshold=8),
            "fastest_kill_turn": min(
                (s["kill_turn"] for s in stats if s.get("kill_turn")), default=None
            ),
            "ego_uses": sorted({ego for s in stats for ego in (s.get("ego_uses") or [])}),
            "errors": sum(1 for row in rows if row["error"]),
        }
    payload["replays"] = replay_index(replays_dir)
    with (out / "evaluation.json").open("w", encoding="utf-8") as handle:
        json.dump(payload, handle, ensure_ascii=False, indent=2)
    with (replays_dir / "index.json").open("w", encoding="utf-8") as handle:
        json.dump(payload["replays"], handle, ensure_ascii=False, indent=1)
    print(json.dumps(payload["policies"], ensure_ascii=False, indent=2)[:4000])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
