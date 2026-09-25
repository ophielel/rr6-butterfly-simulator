#!/usr/bin/env python3
"""Merge post-audit evaluation directories into one compact provenance report."""

from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def load(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def main() -> int:
    burst = load(ROOT / "reports/post_audit_burst_bc/evaluation.json")
    burst.update({"ppo": load(ROOT / "reports/post_audit_burst_ppo/evaluation.json")["policies"]})
    real = load(ROOT / "reports/post_audit_real_bc/evaluation.json")
    real.update({"ppo": load(ROOT / "reports/post_audit_real_ppo/evaluation.json")["policies"]})
    sweep = load(ROOT / "data/teacher_post_audit_sweep/sweep.json")
    payload = {
        "status": "post-audit; not legacy; compact aggregate of local runs",
        "data_fingerprint": burst["provenance"].get("data_fingerprint"),
        "git": burst["provenance"].get("git"),
        "training": {
            "teacher_sweep": sweep,
            "teacher_data": "data/teacher_post_audit_sweep",
            "bc_checkpoint": "models/bc_post_audit.npz",
            "bc_report": "models/bc_post_audit.json",
            "ppo_checkpoint": "models/ppo_post_audit.npz",
            "ppo_report": "models/ppo_post_audit.json",
            "human_setup_oracle": "reports/post_audit_human_setup_oracle.json",
        },
        "evaluation": {
            "burst": burst["policies"],
            "burst_ppo": burst["ppo"],
            "real": real["policies"],
            "real_ppo": real["ppo"],
            "raw": {
                "burst": ["reports/post_audit_burst_bc/evaluation.json", "reports/post_audit_burst_ppo/evaluation.json"],
                "real": ["reports/post_audit_real_bc/evaluation.json", "reports/post_audit_real_ppo/evaluation.json"],
            },
        },
        "caveat": "The real-HP scene did not clear within 50 restarts for any learned policy; burst is an explicitly scaled sanity scene, not the real-boss result.",
    }
    out = ROOT / "reports/post_audit_summary.json"
    out.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
