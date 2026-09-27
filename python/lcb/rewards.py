"""Reward and episode bookkeeping (TRAINING_PLAN.md §3).

The reward is computed from the simulator's own `StepResult` plus the two
observations around it - never from a hand-written damage model:

```text
Boss kill                     +1000
every turn                    -30
ally death                    -30
team wipe                     -1000
Boss HP actually lost         +0.01 * Imago HP delta
winning turn <= 6              +400
winning turn == 7               +0
winning turn > 7                -200 per extra turn
```

Death and kill speed dominate; the process terms only help credit assignment,
which is why "raise Sinking Potency" is deliberately *not* rewarded on its own
(it is visible through `sinking_readiness` in the teacher's value function and
through the terminal terms once the stacks are cashed in).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional

WIN_REWARD = 1000.0
# A turn is a meaningful opportunity cost: a nine-turn win now loses 270 reward
# to time, while the +1000 terminal win still dominates.  This remains a
# generic time cost; it does not reward or name any particular status route.
TURN_PENALTY = -30.0
DEATH_PENALTY = -30.0
WIPE_PENALTY = -1000.0
DAMAGE_REWARD = 0.01
# Generic terminal speed shaping.  A win by turn 6 (or earlier) gets a sizeable
# bonus; turn 7 is neutral; each later winning turn pays a larger cost.  These
# constants deliberately mention no status, identity, skill, or E.G.O.
FAST_KILL_TURN = 6
NEUTRAL_KILL_TURN = 7
FAST_KILL_BONUS = 400.0
LATE_KILL_PENALTY_PER_TURN = -200.0


def _unit_map(obs: Dict[str, Any]) -> Dict[str, Dict[str, Any]]:
    return {unit["id"]: unit for unit in obs.get("units", [])}


def _boss_hp(obs: Dict[str, Any]) -> float:
    """HP of the main enemy, selected by the largest max-HP unit."""
    enemies = [unit for unit in obs.get("units", []) if unit.get("kind") != "sinner"]
    boss = max(enemies, key=lambda unit: float(unit.get("max_hp") or 0), default=None)
    return float((boss or {}).get("hp") or 0.0)


def kill_speed_reward(after: Dict[str, Any]) -> float:
    """Return generic terminal reward based on the winning turn."""
    if after.get("winner") != "Sinners":
        return 0.0
    turn = int(after.get("turn") or 0)
    if turn <= 0:
        return 0.0
    if turn <= FAST_KILL_TURN:
        return FAST_KILL_BONUS
    if turn > NEUTRAL_KILL_TURN:
        return LATE_KILL_PENALTY_PER_TURN * (turn - NEUTRAL_KILL_TURN)
    return 0.0


def compute_reward(
    info: Dict[str, Any], before: Dict[str, Any], after: Optional[Dict[str, Any]] = None
) -> float:
    """Reward of one resolved turn."""
    stats = info.get("stats") or {}
    reward = TURN_PENALTY
    if after is None:
        return reward
    boss_delta = max(0.0, _boss_hp(before) - _boss_hp(after))
    reward += DAMAGE_REWARD * boss_delta
    # Sinking damage remains an evaluation statistic, but is not a shaping
    # term.  The policy must receive credit through the actual Imago HP delta.
    before_units = _unit_map(before)
    for unit_id in stats.get("deaths") or []:
        unit = before_units.get(unit_id)
        if unit is not None and unit.get("kind") == "sinner":
            reward += DEATH_PENALTY
    if after.get("winner") == "Sinners":
        reward += WIN_REWARD + kill_speed_reward(after)
    elif after.get("winner") in ("Enemies",):
        reward += WIPE_PENALTY
    return reward


def terminal_reward(after: Dict[str, Any]) -> float:
    """Return the terminal victory/defeat reward, including generic speed."""
    if after.get("winner") == "Sinners":
        return WIN_REWARD + kill_speed_reward(after)
    if after.get("winner") == "Enemies":
        return WIPE_PENALTY
    return 0.0


@dataclass
class EpisodeStats:
    """Everything the evaluation reports for one episode (§7)."""

    seed: int = 0
    turns: int = 0
    won: bool = False
    winner: Optional[str] = None
    kill_turn: Optional[int] = None
    boss_hp_left: float = 0.0
    boss_hp_start: float = 0.0
    damage_to_enemies: float = 0.0
    damage_to_allies: float = 0.0
    sinking_damage: float = 0.0
    sinking_sp_damage: float = 0.0
    sinking_triggers: int = 0
    deaths: int = 0
    survivors: int = 0
    ego_uses: List[str] = field(default_factory=list)
    skill_uses: List[List[str]] = field(default_factory=list)
    per_turn_reward: List[float] = field(default_factory=list)
    axis: Dict[str, Any] = field(default_factory=dict)
    replays: List[Dict[str, Any]] = field(default_factory=list)

    @property
    def damage_ratio(self) -> float:
        if self.boss_hp_start <= 0:
            return 0.0
        return 1.0 - self.boss_hp_left / self.boss_hp_start

    def to_row(self) -> Dict[str, Any]:
        return {
            "seed": self.seed,
            "won": self.won,
            "winner": self.winner,
            "kill_turn": self.kill_turn,
            "turns": self.turns,
            "boss_hp_left": self.boss_hp_left,
            "boss_hp_start": self.boss_hp_start,
            "damage_to_enemies": self.damage_to_enemies,
            "damage_to_allies": self.damage_to_allies,
            "sinking_damage": self.sinking_damage,
            "sinking_triggers": self.sinking_triggers,
            "deaths": self.deaths,
            "survivors": self.survivors,
            "ego_uses": self.ego_uses,
            "axis_ok": self.axis.get("axis_ok"),
            "axis": self.axis,
        }


__all__ = [
    "EpisodeStats",
    "compute_reward",
    "terminal_reward",
    "WIN_REWARD",
    "TURN_PENALTY",
    "DEATH_PENALTY",
    "WIPE_PENALTY",
    "DAMAGE_REWARD",
    "FAST_KILL_TURN",
    "NEUTRAL_KILL_TURN",
    "FAST_KILL_BONUS",
    "LATE_KILL_PENALTY_PER_TURN",
    "kill_speed_reward",
]
