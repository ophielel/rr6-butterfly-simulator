#!/usr/bin/env python3
"""Summarise the critic-warmup / suffix-data ablation into one markdown table.

Reads the arm training reports (`models/exp_critic_*.json`), the baseline
checkpoint report and the paired evaluation report, and prints:

* the training-time diagnostics (entropy, value warm-up loss, ratio, clip);
* the per-arm holdout wins with Wilson intervals;
* the paired comparison against the reference arm.

```bash
python tools/report_critic_ablation.py \
    --paired reports/paired_critic_warmup_500.json \
    --baseline models/ppo_plan_E_suffix_trigger001.json
```
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any, Dict, List, Optional


def load(path: Path) -> Optional[Dict[str, Any]]:
    if not path.exists():
        return None
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--paired", default="reports/paired_critic_warmup_500.json")
    parser.add_argument("--baseline", default="models/ppo_plan_E_suffix_trigger001.json")
    parser.add_argument("--arms", nargs="*", default=["ctrl", "algo", "data", "algo_data"])
    args = parser.parse_args()

    lines: List[str] = []
    baseline = load(Path(args.baseline))
    if baseline:
        config = baseline.get("config", {})
        lines.append(
            f"baseline `{Path(args.baseline).stem}`: "
            f"warmup={config.get('value_warmup_epochs', 'historical')} "
            f"entropy={config.get('entropy_coef', 0.0)} "
            f"lr={config.get('learning_rate')} "
            f"episodes={config.get('seed_count')} "
            f"iterations={config.get('iterations')}x{config.get('episodes_per_iteration')}"
        )
        history = baseline.get("history", {})
        if history.get("win_rate"):
            lines.append(
                "  rollout win_rate: "
                + ", ".join(f"{value:.2f}" for value in history["win_rate"])
            )
    lines.append("")

    lines.append("| arm | warmup | entropy | suffix | rollout win_rate (last) | holdout wins | rate | 95% CI | net vs ref | McNemar p |")
    lines.append("|---|---:|---:|---|---:|---:|---:|---|---:|---:|")
    paired = load(Path(args.paired)) or {}
    arms = paired.get("arms", {})
    paired_block = paired.get("paired", {})
    for name in args.arms:
        report = load(Path("models") / f"exp_critic_{name}.json")
        warmup = entropy = suffix = None
        last_rate = None
        if report:
            config = report.get("config", {})
            warmup = config.get("value_warmup_epochs")
            entropy = config.get("entropy_coef")
            demo = config.get("demo_data") or []
            suffix = any("suffix_teacher" in str(entry) for entry in demo)
            history = report.get("history", {})
            if history.get("win_rate"):
                last_rate = history["win_rate"][-1]
        arm = arms.get(name, {})
        row = paired_block.get(name, {})
        lines.append(
            "| {name} | {warmup} | {entropy} | {suffix} | {last} | {wins} | {rate} | {ci} | {net} | {p} |".format(
                name=name,
                warmup="-" if warmup is None else warmup,
                entropy="-" if entropy is None else entropy,
                suffix="yes" if suffix else ("no" if suffix is not None else "-"),
                last="-" if last_rate is None else f"{last_rate:.2f}",
                wins=arm.get("wins", "-"),
                rate="-" if not arm else f"{arm.get('win_rate', 0.0):.3f}",
                ci=(
                    "-"
                    if not arm.get("wilson_95")
                    else "%.3f-%.3f" % tuple(arm["wilson_95"])
                ),
                net="-" if not row else f"{row.get('net_gain'):+d}",
                p="-" if not row else f"{row.get('mcnemar_exact_p'):.4f}",
            )
        )
    reference = paired.get("reference")
    if reference:
        ref_arm = arms.get(reference, {})
        lines.insert(
            0,
            f"reference arm `{reference}`: {ref_arm.get('wins', '?')}/"
            f"{ref_arm.get('episodes', '?')} on seeds "
            f"{paired.get('seed_start')}-"
            f"{(paired.get('seed_start') or 0) + (paired.get('seed_count') or 0) - 1}",
        )
    lines.append("")
    lines.append("Training diagnostics per arm (entropy / value warm-up loss):")
    for name in args.arms:
        report = load(Path("models") / f"exp_critic_{name}.json")
        if not report:
            continue
        history = report.get("history", {})
        entropy = history.get("entropy") or []
        warmup = history.get("value_warmup_loss") or []
        lines.append(
            f"* {name}: entropy {['%.3f' % value for value in entropy]}"
            f" | value_warmup_loss {['%.4f' % value for value in warmup]}"
        )
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
