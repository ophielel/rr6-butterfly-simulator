#!/usr/bin/env python3
"""Stage C runner: PPO fine-tuning on top of the BC checkpoint (§6).

```bash
python training/train_ppo.py --checkpoint models/bc.npz --out models/ppo.npz \
    --scenario burst --seed-start 8001 --seed-count 24 --iterations 6
```

PPO only starts once the BC policy produces legal plans (the runner checks the
checkpoint's validation accuracy from the BC report and refuses a checkpoint that
has none), and the training seeds are never used for evaluation.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python"))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkpoint", default=str(ROOT / "models" / "bc.npz"))
    parser.add_argument("--out", default=str(ROOT / "models" / "ppo.npz"))
    parser.add_argument("--scenario", default="burst")
    parser.add_argument("--seed-start", type=int, default=8001)
    parser.add_argument("--seed-count", type=int, default=24)
    parser.add_argument("--iterations", type=int, default=6)
    parser.add_argument("--episodes-per-iteration", type=int, default=8)
    parser.add_argument("--epochs-per-iteration", type=int, default=1)
    parser.add_argument("--learning-rate", type=float, default=1e-3)
    parser.add_argument("--clip", type=float, default=0.2)
    parser.add_argument("--value-coef", type=float, default=0.5)
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--val-seed-start", type=int, default=8001)
    parser.add_argument("--val-seed-count", type=int, default=12)
    parser.add_argument("--require-bc-accuracy", type=float, default=0.2)
    parser.add_argument(
        "--allow-non-bc-checkpoint", action="store_true",
        help="allow curriculum fine-tuning from an earlier PPO checkpoint",
    )
    parser.add_argument(
        "--demo-data", nargs="+", default=[],
        help="scenario-matched Teacher dataset directories/files for replay constraint",
    )
    parser.add_argument("--demo-updates-per-iteration", type=int, default=0)
    parser.add_argument("--demo-batch-decisions", type=int, default=32)
    args = parser.parse_args()

    from lcb import dataset as ds
    from lcb.evaluate import write_json
    from lcb.features import Encoder
    from lcb.nn import PolicyValueNet
    from lcb.ppo import PPOConfig, train_ppo
    from lcb.scenarios import scenario

    report_path = Path(args.checkpoint).with_suffix(".json")
    if report_path.exists():
        with report_path.open(encoding="utf-8") as handle:
            bc_report = json.load(handle)
        accuracy = bc_report.get("info", {}).get("val_accuracy", 0.0)
        if accuracy < args.require_bc_accuracy and not args.allow_non_bc_checkpoint:
            raise SystemExit(
                f"BC validation accuracy {accuracy:.3f} < {args.require_bc_accuracy}; "
                "the plan requires PPO to start from a policy that imitates the teacher"
            )
    elif not args.allow_non_bc_checkpoint:
        raise SystemExit(
            f"missing BC report for {args.checkpoint}; use --allow-non-bc-checkpoint "
            "only for an explicit curriculum continuation"
        )
    encoder = Encoder()
    net = PolicyValueNet.load(args.checkpoint)
    seeds = list(range(args.seed_start, args.seed_start + args.seed_count))
    demo_files = []
    for entry in args.demo_data:
        path = Path(entry)
        found = sorted(path.glob("seed_*.npz")) if path.is_dir() else [path]
        if not found:
            raise SystemExit(f"no Teacher .npz files in {entry}")
        demo_files.extend(found)
    demonstrations = []
    if demo_files:
        demo_arrays = ds.merge([ds.load_dataset(path) for path in demo_files])
        demonstrations = list(ds.iter_decisions(demo_arrays))
    log = []
    started = time.time()
    net, history, info = train_ppo(
        net,
        encoder,
        scenario(args.scenario),
        seeds,
        PPOConfig(
            iterations=args.iterations,
            episodes_per_iteration=args.episodes_per_iteration,
            epochs_per_iteration=args.epochs_per_iteration,
            learning_rate=args.learning_rate,
            clip=args.clip,
            value_coef=args.value_coef,
            seed=args.seed,
            validation_seeds=tuple(
                range(args.val_seed_start, args.val_seed_start + args.val_seed_count)
            ),
            validation_episodes=args.val_seed_count,
            demo_updates_per_iteration=args.demo_updates_per_iteration,
            demo_batch_decisions=args.demo_batch_decisions,
        ),
        log=log,
        demonstrations=demonstrations,
    )
    net.save(args.out)
    write_json(
        Path(args.out).with_suffix(".json"),
        {
            "scenario": args.scenario,
            "scenario_config": scenario(args.scenario).to_dict(),
            "demonstrations": {
                "files": len(demo_files),
                "decisions": len(demonstrations),
                "updates_per_iteration": args.demo_updates_per_iteration,
            },
            "seeds": {"first": min(seeds), "last": max(seeds), "count": len(seeds)},
            "config": vars(args),
            "history": {
                "iteration": history.iteration,
                "mean_return": history.mean_return,
                "win_rate": history.win_rate,
                "policy_loss": history.policy_loss,
                "value_loss": history.value_loss,
                "demo_loss": history.demo_loss,
                "ratio": history.ratio,
                "clip_fraction": history.clip_fraction,
                "validation_win_rate": history.validation_win_rate,
                "validation_kill_turn": history.validation_kill_turn,
            },
            "info": {k: v for k, v in info.items() if k != "returns"},
            "mean_return": info["mean_return"],
            "win_rate": info["win_rate"],
            "elapsed_seconds": round(time.time() - started, 1),
            "log": log,
        },
    )
    for line in log:
        print(line)
    print(f"saved {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
