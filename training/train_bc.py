#!/usr/bin/env python3
"""Stage B runner: behaviour cloning from the teacher data (TRAINING_PLAN.md §5).

```bash
python training/train_bc.py --data data/teacher --out models/bc.npz \
    --epochs 8 --batch-decisions 32
```
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path
from typing import Any, Dict, List

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python"))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--data",
        nargs="+",
        default=[str(ROOT / "data" / "teacher_tree")],
        help="one or more directories of teacher .npz files (they are merged)",
    )
    parser.add_argument("--out", default=str(ROOT / "models" / "bc.npz"))
    parser.add_argument("--epochs", type=int, default=8)
    parser.add_argument("--batch-decisions", type=int, default=32)
    parser.add_argument("--learning-rate", type=float, default=3e-3)
    parser.add_argument("--hidden", type=int, default=64)
    parser.add_argument("--validation-fraction", type=float, default=0.25)
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--limit", type=int, default=0, help="use only the first N .npz files")
    args = parser.parse_args()

    from lcb import dataset as ds
    from lcb.bc import BCConfig, train_bc
    from lcb.evaluate import write_json
    from lcb.features import Encoder

    files = []
    for entry in args.data:
        found = sorted(Path(entry).glob("seed_*.npz"))
        if not found:
            raise SystemExit(f"no teacher data in {entry}")
        files.extend(found)
    if args.limit:
        files = files[: args.limit]
    encoder = Encoder()
    merged: Dict[str, Any] = ds.merge([ds.load_dataset(path) for path in files])
    log: List[str] = []
    started = time.time()
    net, history, info = train_bc(
        merged,
        encoder,
        BCConfig(
            epochs=args.epochs,
            batch_decisions=args.batch_decisions,
            learning_rate=args.learning_rate,
            hidden=args.hidden,
            validation_fraction=args.validation_fraction,
            seed=args.seed,
        ),
        log=log,
    )
    net.save(args.out)
    report = {
        "files": len(files),
        "epochs": args.epochs,
        "elapsed_seconds": round(time.time() - started, 1),
        "history": {
            "train_loss": history.train_loss,
            "val_loss": history.val_loss,
            "val_accuracy": history.val_accuracy,
            "val_accuracy_weighted": history.val_accuracy_weighted,
        },
        "info": info,
        "log": log,
    }
    write_json(Path(args.out).with_suffix(".json"), report)
    for line in log:
        print(line)
    print(f"saved {args.out} (val accuracy {info['val_accuracy']:.3f})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
