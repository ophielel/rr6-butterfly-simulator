"""Environment wrapper around the Rust simulator.

The action space follows the plan: pick a unit -> pick a skill -> pick a target
-> commit.  Every mutation goes through Rust so that the rules cannot be
re-implemented (and therefore silently changed) on the Python side.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, List, Optional

try:  # pragma: no cover - import guard
    from . import lcb_sim  # type: ignore

    BACKEND = "pyo3"
    TEAM: List[str] = list(lcb_sim.TEAM)
    BOSS_IMAGO: str = lcb_sim.BOSS_IMAGO
    BOSS_PUPA: str = lcb_sim.BOSS_PUPA
    #: Line 6 Section 5 Wave 1: the Imago with its three Illusory Butterflies.
    SECTION5_WAVE: List[str] = list(lcb_sim.SECTION5_WAVE)
    EGO_LOADOUT: List[List[str]] = [list(pair) for pair in lcb_sim.EGO_LOADOUT]
except ImportError:  # pragma: no cover - fallback path
    from .stdio_client import StdioSimulator

    lcb_sim = None  # type: ignore
    BACKEND = "stdio"
    # Fixed content ids (kept in sync with sim/crates/lcb-core/src/setup.rs).
    TEAM = ["10110", "10414", "10813", "10913", "11004", "11114", "11214"]
    BOSS_IMAGO = "9567"
    BOSS_PUPA = "9563"
    SECTION5_WAVE = ["9567", "9572", "9573", "9574"]
    EGO_LOADOUT = [
        ["10110", "20109"],
        ["10110", "20106"],
        ["11214", "21207"],
        ["10813", "20807"],
        ["10813", "20810"],
        ["11004", "21009"],
        ["10913", "20903"],
    ]


@dataclass(frozen=True)
class Action:
    """One submitted action."""

    actor: str
    slot: int
    skill: str
    target: str

    def to_wire(self) -> str:
        return json.dumps(
            {
                "Assign": {
                    "actor": self.actor,
                    "slot": self.slot,
                    "skill": self.skill,
                    "target": self.target,
                }
            }
        )


@dataclass(frozen=True)
class EngageAction:
    """Chain a Skill to a specific enemy Skill Slot (focused encounters only)."""

    actor: str
    slot: int
    skill: str
    enemy_slot: int
    #: The enemy unit that owns the Slot (the wire format does not carry it, so
    #: the wrapper fills it from the current state for the search layer).
    target: str = ""

    def to_wire(self) -> str:
        return json.dumps(
            {
                "Engage": {
                    "actor": self.actor,
                    "slot": self.slot,
                    "skill": self.skill,
                    "enemy_slot": self.enemy_slot,
                }
            }
        )


@dataclass(frozen=True)
class EgoAction:
    """An E.G.O usage (awakening / corrosion / overclock)."""

    actor: str
    slot: int
    ego: str
    kind: str  # "Awakening" | "Corrosion" | "Overclock"
    target: str

    def to_wire(self) -> str:
        return json.dumps(
            {
                "UseEgo": {
                    "actor": self.actor,
                    "slot": self.slot,
                    "ego": self.ego,
                    "kind": self.kind,
                    "target": self.target,
                }
            }
        )


def _decode(action: Dict[str, Any]) -> Any:
    if "Assign" in action:
        payload = action["Assign"]
        return Action(
            actor=payload["actor"],
            slot=payload["slot"],
            skill=payload["skill"],
            target=payload["target"],
        )
    if "Engage" in action:
        payload = action["Engage"]
        return EngageAction(
            actor=payload["actor"],
            slot=payload["slot"],
            skill=payload["skill"],
            enemy_slot=payload["enemy_slot"],
        )
    if "UseEgo" in action:
        payload = action["UseEgo"]
        return EgoAction(
            actor=payload["actor"],
            slot=payload["slot"],
            ego=payload["ego"],
            kind=payload["kind"],
            target=payload["target"],
        )
    return action


class LimbusEnv:
    """Gym-like wrapper with clone/rollback support.

    >>> env = LimbusEnv()
    >>> env.reset(seed=1)
    >>> for action in env.legal_actions():
    ...     pass
    >>> env.commit()          # resolve the turn
    """

    def __init__(self, data_dir: Optional[str] = None, strict: bool = False) -> None:
        default = Path(__file__).resolve().parents[2] / "data"
        self._data_dir = str(data_dir or default)
        if BACKEND == "pyo3":
            self._sim = lcb_sim.PySimulator(self._data_dir)
        else:
            self._sim = StdioSimulator(self._data_dir)
        self.strict = strict
        self.turn = 0

    # -- lifecycle ---------------------------------------------------------
    def reset(
        self,
        seed: int,
        team: Optional[List[str]] = None,
        enemies: Optional[List[str]] = None,
        max_turns: Optional[int] = None,
        enemy_hp_scale: Optional[float] = None,
    ) -> str:
        """Start a fresh encounter (returns the initial state hash).

        `enemy_hp_scale` is a *scenario* knob for training/evaluation - it never
        changes a rule, only the boss's HP pool (1.0 == the real encounter).
        """
        return self._sim.reset(
            int(seed),
            list(team) if team else None,
            list(enemies) if enemies else None,
            bool(self.strict),
            int(max_turns) if max_turns is not None else None,
            float(enemy_hp_scale) if enemy_hp_scale is not None else None,
        )

    def clone_state(self) -> "LimbusEnv":
        clone = object.__new__(LimbusEnv)
        clone._sim = self._sim.clone_state()
        clone.strict = self.strict
        clone.turn = self.turn
        return clone

    def state_hash(self) -> str:
        return self._sim.state_hash()

    def state(self) -> Dict[str, Any]:
        return json.loads(self._sim.state_json())

    def transition_hash(self) -> Optional[str]:
        return self._last_transition

    # -- interaction -------------------------------------------------------
    def legal_actions(self) -> List[Any]:
        actions = [_decode(a) for a in json.loads(self._sim.legal_actions())]
        # `EngageAction` chains to an enemy Skill Slot; the enemy unit that owns
        # it comes from the state so callers can treat every action uniformly.
        enemy_ids = [
            unit["id"]
            for unit in self.state()["units"]
            if "Sinner" not in unit.get("kind", {})
        ]
        if enemy_ids:
            actions = [
                (
                    EngageAction(
                        actor=a.actor,
                        slot=a.slot,
                        skill=a.skill,
                        enemy_slot=a.enemy_slot,
                        target=enemy_ids[0],
                    )
                    if isinstance(a, EngageAction)
                    else a
                )
                for a in actions
            ]
        return actions

    def step(self, action: Any) -> Dict[str, Any]:
        wire: Optional[str]
        if action is None:
            wire = None
        elif isinstance(action, (Action, EngageAction, EgoAction)):
            wire = action.to_wire()
        else:
            wire = json.dumps(action)
        result = json.loads(self._sim.step(wire))
        self._last_transition = result.get("transition_hash")
        self.turn = result.get("turn", self.turn)
        return result

    def commit(self) -> Dict[str, Any]:
        return self.step(None)

    # -- the training interface (TRAINING_PLAN.md §1) ----------------------
    def submit(self, action: Any) -> Dict[str, Any]:
        """Submit one action without resolving the turn (no state clone/hash)."""
        wire = (
            action.to_wire()
            if isinstance(action, (Action, EngageAction, EgoAction))
            else json.dumps(action)
        )
        return json.loads(self._sim.submit(wire))

    def step_turn(self, plan: List[Any]) -> Dict[str, Any]:
        """Resolve one **complete** turn plan atomically.

        `plan` is a list of actions, one per unit that can act.  The simulator
        validates every action against its own legal-action list and rejects a
        partial or illegal plan without changing the state; the returned
        `info` carries the replay material (hashes, RNG continuation, battle log
        delta) and the turn's real statistics.
        """
        wire = [
            json.loads(action.to_wire())
            if isinstance(action, (Action, EngageAction, EgoAction))
            else action
            for action in plan
        ]
        result = json.loads(self._sim.step_turn(json.dumps(wire)))
        self._last_transition = result.get("transition_hash")
        if result.get("ok"):
            self.turn = result.get("turn", self.turn)
        return result

    def observe(self) -> Dict[str, Any]:
        """Compact observation: everything a decision may depend on, no log."""
        return json.loads(self._sim.observation_json())

    def search_key(self) -> str:
        """Transposition key (state hash without the log/warnings/statistics)."""
        return self._sim.search_key()

    # -- diagnostics -------------------------------------------------------
    def unknown_rules(self) -> List[str]:
        return list(self._sim.unknown_rules())

    def strict_blockers(self) -> List[str]:
        return list(self._sim.strict_blockers())

    # -- scoring (objective only; not a damage model) ----------------------
    @staticmethod
    def score(state: Dict[str, Any]) -> float:
        """Difference in remaining HP, weighted towards the enemy.

        This is the search objective.  It reads the *simulated* state, so it is
        never a hand-written damage estimate.
        """
        sinners = 0
        enemies = 0
        for unit in state.get("units", []):
            kind = unit.get("kind", {})
            alive = unit.get("alive", True)
            hp = unit.get("hp", 0) if alive else 0
            if "Sinner" in kind:
                sinners += hp
            else:
                enemies += hp
        weighted = 1.0 * sinners - 4.0 * enemies
        if state.get("winner") == "Sinners":
            weighted += 10_000
        elif state.get("winner") == "Enemies":
            weighted -= 10_000
        return weighted


__all__ = [
    "LimbusEnv",
    "Action",
    "EgoAction",
    "TEAM",
    "BOSS_IMAGO",
    "BOSS_PUPA",
    "BACKEND",
]
