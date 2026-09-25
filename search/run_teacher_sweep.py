#!/usr/bin/env python3
"""Run the Teacher across HP scales and aggregate the generic combat outcomes.

The sweep is deliberately a wrapper around `run_teacher.py`: each scale gets its
own output directory, seed list, dataset and provenance.  Sinking/E.G.O route
labels are read after the search from replay/stat rows, never used as search
features or objective terms.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any, Dict, List

ROOT = Path(__file__).resolve().parents[1]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scales", nargs="+", type=float,
                        default=[0.08, 0.15, 0.30, 0.50, 0.75, 1.0])
    parser.add_argument("--seed-start", type=int, default=1)
    parser.add_argument("--seed-count", type=int, default=40)
    parser.add_argument("--max-turns", type=int, default=30)
    parser.add_argument("--horizon", type=int, default=3)
    parser.add_argument("--plan-width", type=int, default=32)
    parser.add_argument("--candidate-cap", type=int, default=6)
    parser.add_argument("--turn-width", type=int, default=4)
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--out", type=Path, default=ROOT / "reports" / "teacher_sweep")
    parser.add_argument("--infinite-ego-resources", action="store_true")
    args = parser.parse_args()

    args.out.mkdir(parents=True, exist_ok=True)
    rows: List[Dict[str, Any]] = []
    for scale in args.scales:
        label = f"hp_{scale:g}".replace(".", "_")
        target = args.out / label
        command = [
            sys.executable,
            str(ROOT / "search" / "run_teacher.py"),
            "--scenario", "real",
            "--seed-start", str(args.seed_start),
            "--seed-count", str(args.seed_count),
            "--max-turns", str(args.max_turns),
            "--horizon", str(args.horizon),
            "--plan-width", str(args.plan_width),
            "--candidate-cap", str(args.candidate_cap),
            "--turn-width", str(args.turn_width),
            "--jobs", str(args.jobs),
            "--enemy-hp-scale", str(scale),
            "--out", str(target),
        ]
        if args.infinite_ego_resources:
            command.append("--infinite-ego-resources")
        subprocess.run(command, cwd=ROOT, check=True)
        index = json.loads((target / "teacher_index.json").read_text(encoding="utf-8"))
        summary = dict(index["summary"])
        episodes = index.get("episodes", [])
        summary.update({
            "enemy_hp_scale": scale,
            "dataset": str(target),
            "known_setup_like_rate": sum(bool((row.get("axis") or {}).get("known_setup_like")) for row in episodes) / len(episodes) if episodes else 0.0,
            "direct_burst_rate": sum(bool((row.get("axis") or {}).get("direct_burst")) for row in episodes) / len(episodes) if episodes else 0.0,
            "peak_sinking_potency_max": max((int((row.get("axis") or {}).get("peak_sinking_potency") or 0) for row in episodes), default=0),
            "peak_sinking_count_max": max((int((row.get("axis") or {}).get("peak_sinking_count") or 0) for row in episodes), default=0),
        })
        rows.append(summary)

    payload = {
        "experiment": "teacher_hp_scale_sweep",
        "objective": "generic combat outcome; route labels are post-hoc only",
        "scales": rows,
        "seed_range": {"first": args.seed_start, "count": args.seed_count},
        "infinite_ego_resources": args.infinite_ego_resources,
        "budget": {
            "horizon": args.horizon,
            "plan_width": args.plan_width,
            "candidate_cap": args.candidate_cap,
            "turn_width": args.turn_width,
            "max_turns": args.max_turns,
        },
    }
    (args.out / "sweep.json").write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(rows, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
