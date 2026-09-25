#!/usr/bin/env python3
"""Assemble `reports/evaluation.json` from the per-policy JSONL files.

`eval/run_eval.py` already writes the JSON it was invoked for; this script reads
**every** `<scenario>_<policy>.jsonl` (so policies evaluated in separate runs are
reported together) and rebuilds the combined report - including the §7.1
acceptance-criteria table against the Greedy baseline.

```bash
python eval/summarise_reports.py --scenario burst
```
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Dict, List

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python"))


def load_rows(path: Path) -> List[Dict[str, Any]]:
    rows: List[Dict[str, Any]] = []
    with path.open(encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


def acceptance(
    baseline: Dict[str, Any], candidate: Dict[str, Any], short_turn: int
) -> Dict[str, Any]:
    """The plan's §7.1 criteria, evaluated as literally as the numbers allow."""
    win_gap = candidate["win_rate"] - baseline["win_rate"]
    base_kill = baseline.get("kill_turn_median")
    cand_kill = candidate.get("kill_turn_median")
    improvement = None
    if base_kill and cand_kill:
        improvement = (base_kill - cand_kill) / base_kill
    return {
        "win_rate_baseline": baseline["win_rate"],
        "win_rate_candidate": candidate["win_rate"],
        "win_rate_gap_points": round(win_gap * 100, 2),
        "win_rate_ok": win_gap >= -0.02,
        "kill_turn_median_baseline": base_kill,
        "kill_turn_median_candidate": cand_kill,
        "kill_turn_improvement": improvement,
        "kill_turn_ok": bool(
            improvement is not None and improvement >= 0.15 and (base_kill - cand_kill) >= 1
        ),
        "short_win_rate_baseline": baseline["short_win_rate"],
        "short_win_rate_candidate": candidate["short_win_rate"],
        "short_win_ok": candidate["short_win_rate"] > baseline["short_win_rate"],
        "axis_rate_among_wins_baseline": baseline["axis_ok_rate_among_wins"],
        "axis_rate_among_wins_candidate": candidate["axis_ok_rate_among_wins"],
        "short_turn_threshold": short_turn,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenario", default="burst")
    parser.add_argument("--reports", default=str(ROOT / "reports"))
    parser.add_argument("--replays", default=str(ROOT / "replays"))
    parser.add_argument("--baseline", default="Greedy")
    parser.add_argument("--candidates", nargs="*", default=["AI", "Teacher"])
    parser.add_argument("--short-turn", type=int, default=0)
    args = parser.parse_args()

    from lcb.evaluate import best_of_n, provenance, replay_index, restart_aware, summarise
    from lcb.scenarios import scenario

    reports = Path(args.reports)
    payload: Dict[str, Any] = {}
    rows_by_policy: Dict[str, List[Dict[str, Any]]] = {}
    for path in sorted(reports.glob(f"{args.scenario}_*.jsonl")):
        policy = path.stem.split(f"{args.scenario}_", 1)[1]
        rows_by_policy[policy] = load_rows(path)

    if not rows_by_policy:
        raise SystemExit(f"no {args.scenario}_*.jsonl in {reports}")

    greedy = rows_by_policy.get(args.baseline, [])
    greedy_summary = summarise(greedy) if greedy else {}
    short_turn = args.short_turn or int(greedy_summary.get("kill_turn_median") or 12)

    policies: Dict[str, Any] = {}
    for policy, rows in rows_by_policy.items():
        policies[policy] = {
            "summary": summarise(rows, short_turn=short_turn),
            "best_of_n": best_of_n(rows, (1, 10, 50)),
            "restart_aware": restart_aware(rows, turn_threshold=8),
            "fastest_kill_turn": min(
                (r["kill_turn"] for r in rows if r.get("kill_turn")), default=None
            ),
            "ego_uses": sorted({ego for r in rows for ego in (r.get("ego_uses") or [])}),
            "files": 1,
        }

    scene = scenario(args.scenario)
    seeds = sorted({int(r["seed"]) for rows in rows_by_policy.values() for r in rows})
    payload = {
        "provenance": provenance(
            scene,
            seeds,
            {
                "assembled_by": "eval/summarise_reports.py",
                "policies": sorted(rows_by_policy),
                "note": (
                    "Each policy is an independent run over the same test seeds; "
                    "the AI checkpoint is models/ai.npz unless stated otherwise."
                ),
            },
        ),
        "short_turn_threshold": short_turn,
        "policies": policies,
        "acceptance": {},
        "replays": replay_index(Path(args.replays)),
    }
    if args.baseline in policies:
        baseline = policies[args.baseline]["summary"]
        for candidate in args.candidates:
            if candidate in policies:
                payload["acceptance"][candidate] = acceptance(
                    baseline, policies[candidate]["summary"], short_turn
                )
    with (reports / "evaluation.json").open("w", encoding="utf-8") as handle:
        json.dump(payload, handle, ensure_ascii=False, indent=2)
    with (Path(args.replays) / "index.json").open("w", encoding="utf-8") as handle:
        json.dump(payload["replays"], handle, ensure_ascii=False, indent=1)
    for policy, data in policies.items():
        summary = data["summary"]
        print(
            f"{policy:11s} n={summary['episodes']:3d} win={summary['win_rate']:.2f} "
            f"kill_med={summary['kill_turn_median']} p25={summary['kill_turn_p25']} "
            f"short={summary['short_win_rate']:.2f} axis={summary['axis_ok_rate']:.2f} "
            f"bon10={data['best_of_n'].get('n10', {}).get('window_win_rate')}"
        )
    for candidate, table in payload["acceptance"].items():
        print(f"acceptance[{candidate}] = {json.dumps(table, ensure_ascii=False)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
