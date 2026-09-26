"""Evaluation protocol and metrics (TRAINING_PLAN.md §7).

Because a real fight may be lost to variance, the report never stops at the mean:

* win rate,
* mean / median `kill_turn` of the successful runs,
* P10 / P25 kill turn (the short-axis indicator),
* short-win rate `kill_turn <= T`, where `T` comes from the Greedy validation
  quantile or an explicit threshold,
* best-of-N restarts (N = 1, 10, 50) over independent seeds of the same scene,
* fastest kill, long-tail failure rate, average survivors,
* the Sinking / Harmony / Solemn Lament axis events and the trigger damage.

Everything a run reports is written to JSONL, with the code revision, the
simulator's data hash, the configuration, the seed range and the restart count -
so a reader can tell "the axis was found" from "one lucky restart".
"""

from __future__ import annotations

import hashlib
import json
import platform
import subprocess
import sys
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

import numpy as np

from .env import LimbusEnv, SECTION5_WAVE, TEAM
from .features import Encoder, SkillTable
from .plans import canonical
from .rewards import EpisodeStats, compute_reward
from .teacher import AXIS_EGO, boss_sinking, detect_axis, detect_strategy_labels, imago

REPO = Path(__file__).resolve().parents[2]


@dataclass
class Scenario:
    """One evaluation scene."""

    name: str = "section5"
    enemies: Tuple[str, ...] = tuple(SECTION5_WAVE)
    max_turns: int = 30
    #: **Scenario knob**: 1.0 is the encounter as the data describes it.
    enemy_hp_scale: float = 1.0
    strict: bool = True
    infinite_ego_resources: bool = False
    description: str = ""

    def to_dict(self) -> Dict[str, Any]:
        data = asdict(self)
        data["enemies"] = list(self.enemies)
        return data


@dataclass
class EpisodeRecord:
    """One evaluated episode (the replay is kept separately)."""

    policy: str
    scenario: str
    seed: int
    stats: Dict[str, Any]
    per_turn: List[Dict[str, Any]] = field(default_factory=list)
    replay: List[Dict[str, Any]] = field(default_factory=list)
    error: Optional[str] = None


def run_episode(
    policy,
    scenario: Scenario,
    seed: int,
    record_replay: bool = False,
    replay_limit: int = 400,
) -> EpisodeRecord:
    env = LimbusEnv(strict=scenario.strict)
    env.reset(
        seed,
        enemies=list(scenario.enemies),
        max_turns=scenario.max_turns,
        enemy_hp_scale=scenario.enemy_hp_scale,
        infinite_ego_resources=scenario.infinite_ego_resources,
    )
    first = env.observe()
    boss = imago(first)
    stats = EpisodeStats(
        seed=seed,
        boss_hp_start=float((boss or {}).get("hp") or 1.0),
    )
    policy.start_episode(first)
    replay: List[Dict[str, Any]] = []
    while True:
        obs = env.observe()
        if obs.get("winner") or obs.get("phase") == "Finished":
            break
        legal = env.legal_actions()
        try:
            plan = policy.plan(env, obs, legal)
        except Exception as exc:  # pragma: no cover - defensive
            return EpisodeRecord(policy.name, scenario.name, seed, stats.to_row(), error=repr(exc))
        if not plan:
            break
        info = env.step_turn(plan)
        if not info.get("ok"):
            return EpisodeRecord(
                policy.name,
                scenario.name,
                seed,
                stats.to_row(),
                error=str(info.get("error")),
            )
        after = env.observe()
        reward = compute_reward(info, obs, after)
        stats.per_turn_reward.append(reward)
        stats.turns = int(info.get("turn") or stats.turns + 1)
        turn_stats = info.get("stats") or {}
        stats.damage_to_enemies += float(turn_stats.get("damage_to_enemies") or 0)
        stats.damage_to_allies += float(turn_stats.get("damage_to_allies") or 0)
        stats.sinking_damage += float(turn_stats.get("sinking_damage") or 0)
        stats.sinking_sp_damage += float(turn_stats.get("sinking_sp_damage") or 0)
        stats.sinking_triggers += int(turn_stats.get("sinking_triggers") or 0)
        stats.deaths += sum(
            1
            for unit_id in turn_stats.get("deaths") or []
            if any(u["id"] == unit_id and u.get("kind") == "sinner" for u in obs["units"])
        )
        for ego in turn_stats.get("ego_uses") or []:
            if ego not in stats.ego_uses:
                stats.ego_uses.append(ego)
        stats.skill_uses.append(list(turn_stats.get("skill_uses") or []))
        if record_replay and len(replay) < replay_limit:
            replay.append(
                {
                    "turn": info.get("turn"),
                    "plan": [json.loads(canonical(a)) for a in plan],
                    "state_hash_before": info.get("state_hash_before"),
                    "state_hash_after": info.get("state_hash_after"),
                    "transition_hash": info.get("transition_hash"),
                    "reward": reward,
                    "stats": turn_stats,
                    "boss_sinking": boss_sinking(obs),
                }
            )

    final = env.observe()
    stats.winner = final.get("winner")
    stats.won = final.get("winner") == "Sinners"
    stats.kill_turn = stats.turns if stats.won else None
    stats.boss_hp_left = float((imago(final) or {}).get("hp") or 0)
    stats.survivors = sum(
        1 for u in final.get("units", []) if u.get("kind") == "sinner" and u.get("alive")
    )
    stats.axis = detect_axis(replay)
    stats.axis.update(detect_strategy_labels(replay))
    return EpisodeRecord(
        policy=policy.name,
        scenario=scenario.name,
        seed=seed,
        stats=stats.to_row(),
        replay=replay,
    )


# ---------------------------------------------------------------------------
# Aggregation
# ---------------------------------------------------------------------------


def _percentile(values: Sequence[float], q: float) -> Optional[float]:
    if not values:
        return None
    return float(np.percentile(np.asarray(values, dtype=float), q))


def wilson_interval(successes: int, total: int, z: float = 1.96) -> Tuple[float, float]:
    """Two-sided Wilson score interval for a binomial rate."""
    if total <= 0:
        return (0.0, 1.0)
    p = min(1.0, max(0.0, successes / total))
    z2 = z * z
    denominator = 1.0 + z2 / total
    center = (p + z2 / (2.0 * total)) / denominator
    margin = z * np.sqrt(p * (1.0 - p) / total + z2 / (4.0 * total * total)) / denominator
    return (max(0.0, float(center - margin)), min(1.0, float(center + margin)))


def summarise(records: Sequence[Dict[str, Any]], short_turn: Optional[int] = None) -> Dict[str, Any]:
    """The §7 metric block for one policy on one scenario."""
    total = len(records)
    if total == 0:
        return {"episodes": 0}
    wins = [r for r in records if r.get("won")]
    kill_turns = sorted(float(r["kill_turn"]) for r in wins if r.get("kill_turn"))
    threshold = short_turn if short_turn is not None else 12
    short_wins = [r for r in records if r.get("won") and (r.get("kill_turn") or 999) <= threshold]
    damage = [float(r.get("damage_to_enemies") or 0) for r in records]
    hp_left = [float(r.get("boss_hp_left") or 0) for r in records]
    survivors = [float(r.get("survivors") or 0) for r in records]
    sinking = [float(r.get("sinking_damage") or 0) for r in records]
    axis_rows = [r for r in records if r.get("axis_ok")]
    strategy_rows = [r.get("axis") or {} for r in records]
    ego_turns: List[int] = []
    for row in records:
        for entry in row.get("ego_uses") or []:
            ego_turns.append(int(row.get("kill_turn") or row.get("turns") or 0))
    failures = [r for r in records if not r.get("won")]
    long_tail = [
        r
        for r in failures
        if float(r.get("boss_hp_left") or 0) > 0.75 * float(r.get("boss_hp_start") or 1)
    ]
    ci_low, ci_high = wilson_interval(len(wins), total)
    return {
        "episodes": total,
        "wins": len(wins),
        "win_rate": len(wins) / total,
        "win_rate_wilson_95": [ci_low, ci_high],
        "kill_turn_mean": float(np.mean(kill_turns)) if kill_turns else None,
        "kill_turn_median": float(np.median(kill_turns)) if kill_turns else None,
        "kill_turn_p10": _percentile(kill_turns, 10),
        "kill_turn_p25": _percentile(kill_turns, 25),
        "fastest_kill_turn": min(kill_turns) if kill_turns else None,
        "short_turn_threshold": threshold,
        "short_win_rate": len(short_wins) / total,
        "damage_to_enemies_mean": float(np.mean(damage)),
        "boss_hp_left_median": float(np.median(hp_left)),
        "survivors_mean": float(np.mean(survivors)),
        "sinking_damage_mean": float(np.mean(sinking)),
        "axis_ok_rate": len(axis_rows) / total,
        "axis_ok_rate_among_wins": (len(axis_rows) / len(wins)) if wins else 0.0,
        "known_setup_like_rate": sum(bool(row.get("known_setup_like")) for row in strategy_rows) / total,
        "direct_burst_rate": sum(bool(row.get("direct_burst")) for row in strategy_rows) / total,
        "peak_sinking_potency_mean": float(np.mean([float(row.get("peak_sinking_potency") or 0) for row in strategy_rows])),
        "peak_sinking_count_mean": float(np.mean([float(row.get("peak_sinking_count") or 0) for row in strategy_rows])),
        "long_tail_failure_rate": len(long_tail) / total,
        "ego_use_turns": sorted(ego_turns),
        "errors": sum(1 for r in records if r.get("error")),
    }


def restart_aware(
    records: Sequence[Dict[str, Any]],
    turn_threshold: int = 8,
    n_values: Sequence[int] = (1, 5, 10, 20, 50),
) -> Dict[str, Any]:
    """Report both single-attempt and restart-budget clear probabilities."""
    rows = sorted(records, key=lambda r: int(r["seed"]))
    qualifying_rows = [
        r for r in rows
        if r.get("won") and r.get("kill_turn") and int(r["kill_turn"]) <= turn_threshold
    ]
    out: Dict[str, Any] = {
        "turn_threshold": turn_threshold,
        "attempts": len(rows),
        "clear_attempts": len(qualifying_rows),
        "clear_rate_per_attempt": len(qualifying_rows) / len(rows) if rows else 0.0,
    }
    for n in n_values:
        if n <= 0 or len(rows) < n:
            continue
        window_clear: List[bool] = []
        best_kills: List[int] = []
        attempts_until: List[int] = []
        for start in range(0, len(rows) - n + 1):
            window = rows[start : start + n]
            qualifying = [
                int(row["kill_turn"])
                for row in window
                if row.get("won") and row.get("kill_turn")
                and int(row["kill_turn"]) <= turn_threshold
            ]
            window_clear.append(bool(qualifying))
            if qualifying:
                best_kills.append(min(qualifying))
                attempts_until.append(next(
                    index + 1 for index, row in enumerate(window)
                    if row.get("won") and row.get("kill_turn")
                    and int(row["kill_turn"]) <= turn_threshold
                ))
        out[f"n{n}"] = {
            "windows": len(window_clear),
            "clear_within_n_rate": sum(window_clear) / len(window_clear) if window_clear else 0.0,
            "best_kill_turn_min": min(best_kills) if best_kills else None,
            "best_kill_turn_median": float(np.median(best_kills)) if best_kills else None,
            "median_attempts_until_clear": float(np.median(attempts_until)) if attempts_until else None,
        }
    return out


def best_of_n(
    records: Sequence[Dict[str, Any]], n_values: Sequence[int] = (1, 10, 50)
) -> Dict[str, Any]:
    """Best result out of N independent restarts of the same scene.

    Seeds are independent attempts at the same scene, so a window of N records is
    exactly one "best of N restarts" experiment (the plan's §7 requirement).
    """
    rows = sorted(records, key=lambda r: int(r["seed"]))
    out: Dict[str, Any] = {}
    for n in n_values:
        if n <= 0 or len(rows) < n:
            continue
        kills: List[Optional[int]] = []
        wins = 0
        for start in range(0, len(rows) - n + 1):
            window = rows[start : start + n]
            window_kills = [
                int(r["kill_turn"]) for r in window if r.get("won") and r.get("kill_turn")
            ]
            if window_kills:
                wins += 1
                kills.append(min(window_kills))
            else:
                kills.append(None)
        won = [k for k in kills if k is not None]
        out[f"n{n}"] = {
            "windows": len(kills),
            "window_win_rate": wins / len(kills) if kills else 0.0,
            "best_kill_turn_min": min(won) if won else None,
            "best_kill_turn_median": float(np.median(won)) if won else None,
        }
    return out


# ---------------------------------------------------------------------------
# Meta / provenance
# ---------------------------------------------------------------------------


def data_fingerprint(data_dir: Optional[str] = None) -> str:
    """Hash of the generated library, so a report pins the simulator's data."""
    root = Path(data_dir or REPO / "data")
    digest = hashlib.sha256()
    for path in sorted(root.rglob("*.json")):
        if "_raw" in path.parts:
            continue
        digest.update(path.relative_to(root).as_posix().encode())
        digest.update(path.read_bytes())
    return digest.hexdigest()[:16]


def git_revision() -> Dict[str, Any]:
    def run(args: Sequence[str]) -> str:
        try:
            return subprocess.run(
                ["git", *args], cwd=REPO, capture_output=True, text=True, check=False
            ).stdout.strip()
        except OSError:  # pragma: no cover - git missing
            return ""

    return {
        "commit": run(["rev-parse", "HEAD"]),
        "dirty": bool(run(["status", "--porcelain"])),
        "branch": run(["rev-parse", "--abbrev-ref", "HEAD"]),
    }


def provenance(scenario: Scenario, seeds: Sequence[int], extra: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:
    meta = {
        "generated_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "python": sys.version.split()[0],
        "numpy": np.__version__,
        "platform": platform.platform(),
        "git": git_revision(),
        "data_fingerprint": data_fingerprint(),
        "scenario": scenario.to_dict(),
        "seeds": {"first": min(seeds) if seeds else None, "last": max(seeds) if seeds else None, "count": len(seeds)},
        "team": list(TEAM),
    }
    if extra:
        meta.update(extra)
    return meta


def write_jsonl(path: Path, rows: Iterable[Dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(row, ensure_ascii=False) + "\n")


def replay_index(directory: Path) -> Dict[str, Any]:
    """Index of the replay files a run wrote (`replays/index.json`).

    Each entry is enough to find and re-run the fight: the policy, the scenario,
    the seed, the kill turn and the transition hashes of every turn.
    """
    entries: List[Dict[str, Any]] = []
    for path in sorted(Path(directory).glob("*.json")):
        if path.name == "index.json":
            continue
        try:
            with path.open(encoding="utf-8") as handle:
                payload = json.load(handle)
        except (OSError, json.JSONDecodeError):  # pragma: no cover - defensive
            continue
        turns = payload.get("turns") or []
        entries.append(
            {
                "file": path.name,
                "policy": payload.get("policy"),
                "scenario": payload.get("scenario"),
                "scenario_config": payload.get("scenario_config"),
                "seed": payload.get("seed"),
                "won": (payload.get("stats") or {}).get("won"),
                "kill_turn": (payload.get("stats") or {}).get("kill_turn"),
                "turns": len(turns),
                "transition_hashes": [t.get("transition_hash") for t in turns],
                "state_hash_before": turns[0].get("state_hash_before") if turns else None,
                "state_hash_after": turns[-1].get("state_hash_after") if turns else None,
                "replayable": bool(turns)
                and all(
                    t.get("state_hash_before") and t.get("state_hash_after") for t in turns
                ),
            }
        )
    entries.sort(key=lambda entry: (str(entry.get("policy")), entry.get("seed") or 0))
    return {"count": len(entries), "entries": entries}


def write_json(path: Path, payload: Dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        json.dump(payload, handle, ensure_ascii=False, indent=2)
        handle.write("\n")


__all__ = [
    "replay_index",
    "Scenario",
    "EpisodeRecord",
    "run_episode",
    "summarise",
    "wilson_interval",
    "best_of_n",
    "restart_aware",
    "provenance",
    "data_fingerprint",
    "git_revision",
    "write_json",
    "write_jsonl",
    "AXIS_EGO",
]
