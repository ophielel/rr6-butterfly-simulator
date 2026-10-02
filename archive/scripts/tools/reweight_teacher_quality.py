#!/usr/bin/env python3
"""Apply generic episode-outcome weights to a Teacher corpus.

The score combines fraction of main-enemy HP removed with surviving ally
fraction. It reads only terminal outcome metadata, never status, skill, identity,
or E.G.O labels. The existing per-row behavior-cloning weights are preserved.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Dict

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python"))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--teacher-dir", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--quality-power", type=float, default=2.0)
    parser.add_argument("--failure-floor", type=float, default=0.1)
    args = parser.parse_args()

    import numpy as np
    from lcb.dataset import load_dataset, save_dataset

    source = Path(args.teacher_dir)
    target = Path(args.out)
    index_path = source / "teacher_index.json"
    if not index_path.exists():
        raise SystemExit(f"missing Teacher index: {index_path}")
    index = json.loads(index_path.read_text(encoding="utf-8"))
    episodes = {int(row["seed"]): row for row in index["episodes"]}
    target.mkdir(parents=True, exist_ok=True)
    rows = []
    for path in sorted(source.glob("seed_*.npz")):
        seed = int(path.stem.split("_")[-1])
        episode = episodes.get(seed)
        if episode is None:
            raise SystemExit(f"seed {seed} missing from Teacher index")
        data = load_dataset(path)
        count = len(data.get("weight", []))
        if count:
            won = bool(episode.get("won"))
            boss_start = float(episode.get("boss_hp_start") or 0.0)
            boss_left = float(episode.get("boss_hp_left") or 0.0)
            progress = 1.0 - boss_left / boss_start if boss_start > 0 else 0.0
            survivors = max(0, min(7, int(episode.get("survivors") or 0)))
            quality = 1.0 if won else max(
                0.0, min(1.0, progress * (0.75 + 0.25 * survivors / 7.0))
            )
            multiplier = 1.0 if won else max(args.failure_floor, quality**args.quality_power)
            data["weight"] = (data["weight"].astype(np.float32) * multiplier).astype(
                np.float32
            )
        else:
            quality = 0.0
            multiplier = 0.0
        save_dataset(target / path.name, data)
        rows.append(
            {
                "seed": seed,
                "won": bool(episode.get("won")),
                "quality": quality,
                "weight_multiplier": multiplier,
                "actor_rows": count,
            }
        )
    report: Dict[str, Any] = {
        "source": str(source),
        "output": str(target),
        "quality": "win=1; otherwise Boss HP progress * (0.75 + 0.25 * survivor fraction)",
        "quality_power": args.quality_power,
        "failure_floor": args.failure_floor,
        "episodes": len(rows),
        "nonempty_episodes": sum(row["actor_rows"] > 0 for row in rows),
        "actor_rows": sum(row["actor_rows"] for row in rows),
        "rows": rows,
    }
    (target / "quality_weights.json").write_text(
        json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps({k: v for k, v in report.items() if k != "rows"}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
