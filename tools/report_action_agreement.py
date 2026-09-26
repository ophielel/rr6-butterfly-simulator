#!/usr/bin/env python3
"""Measure Teacher agreement on seed-disjoint training and validation samples."""

from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any, Dict, List

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python"))


def _score(data: Dict[str, np.ndarray], net) -> Dict[str, Any]:
    buckets: Dict[str, Dict[str, float]] = defaultdict(
        lambda: {"rows": 0.0, "correct": 0.0, "top3": 0.0}
    )
    offsets = data["offsets"]
    decisions = data["decision"]
    order = np.argsort(decisions, kind="stable")
    cursor = 0
    position = 0
    while position < len(order):
        decision = decisions[order[position]]
        decision_rows: List[int] = []
        while position < len(order) and decisions[order[position]] == decision:
            decision_rows.append(int(order[position]))
            position += 1
        state = data["state"][decision_rows[0]]
        for row in decision_rows:
            count = int(offsets[row])
            candidates = data["cand"][cursor : cursor + count]
            cursor += count
            label = int(data["label"][row])
            if count == 0 or label < 0:
                continue
            logits = net.logits(net.embed(state), candidates)
            ranking = np.argsort(logits)[::-1]
            keys = [
                "overall",
                f"actor_{int(data['actor'][row])}",
                f"turn_{int(data['turn'][row])}" if "turn" in data else "turn_unknown",
            ]
            for key in keys:
                bucket = buckets[key]
                bucket["rows"] += 1
                bucket["correct"] += float(int(ranking[0]) == label)
                bucket["top3"] += float(label in ranking[: min(3, count)])

    def finish(value: Dict[str, float]) -> Dict[str, Any]:
        total = max(1.0, value["rows"])
        return {
            "rows": int(value["rows"]),
            "top1_accuracy": value["correct"] / total,
            "top3_accuracy": value["top3"] / total,
        }

    return {
        "overall": finish(buckets["overall"]),
        "by_actor_position": {
            key: finish(value)
            for key, value in sorted(buckets.items())
            if key.startswith("actor_")
        },
        "by_turn": {
            key: finish(value)
            for key, value in sorted(buckets.items())
            if key.startswith("turn_")
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", nargs="+", required=True)
    parser.add_argument("--checkpoint", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--split-seed", type=int, default=0)
    parser.add_argument("--validation-fraction", type=float, default=0.25)
    args = parser.parse_args()

    from lcb import dataset as ds
    from lcb.features import Encoder
    from lcb.nn import PolicyValueNet

    files: List[Path] = []
    for entry in args.data:
        path = Path(entry)
        files.extend(sorted(path.glob("seed_*.npz")) if path.is_dir() else [path])
    if not files:
        raise SystemExit("no dataset files")
    data = ds.merge([ds.load_dataset(path) for path in files])
    _train, validation = ds.seed_split(
        data, validation_fraction=args.validation_fraction, seed=args.split_seed
    )
    net = PolicyValueNet.load(args.checkpoint)
    report = {
        "checkpoint": args.checkpoint,
        "files": len(files),
        "split": {
            "method": "episode seed",
            "seed": args.split_seed,
            "validation_fraction": args.validation_fraction,
            "training_seeds": int(len(np.unique(_train["seed"]))),
            "validation_seeds": int(len(np.unique(validation["seed"]))),
        },
        "train": _score(_train, net),
        "validation": _score(validation, net),
    }
    Path(args.out).parent.mkdir(parents=True, exist_ok=True)
    Path(args.out).write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
