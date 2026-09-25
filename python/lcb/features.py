"""Stable discrete encoders for the training layer (TRAINING_PLAN.md §2.2).

The observation is built from `PySimulator.observation_json()`, which is a
*compact* view of `BattleState` (no battle log, so the length of a fight cannot
leak into the features).  Everything here is deterministic and versioned: the
dimensions and the order of every block are fixed by this file, so a checkpoint
trained yesterday still describes the same numbers today.

Nothing in this module re-implements a game rule.  Skill properties are read from
the generated library JSON (`data/identities`, `data/ego`, `data/enemies`) - the
same records the simulator itself loads, at the same Uptie (IV).
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

import numpy as np

#: Sins in the game's own order (`Sin::ALL`).
SINS: Tuple[str, ...] = (
    "Wrath",
    "Lust",
    "Sloth",
    "Gluttony",
    "Gloom",
    "Pride",
    "Envy",
)

#: Damage types.
DAMAGE_TYPES: Tuple[str, ...] = ("Slash", "Pierce", "Blunt")

#: E.G.O skill kinds.
EGO_KINDS: Tuple[str, ...] = ("Awakening", "Corrosion", "Overclock")

#: Statuses that the fixed content actually uses.  Anything outside this list is
#: folded into a single "other" bucket (magnitude only), so the encoding stays
#: fixed-size without dropping information silently.
STATUS_VOCAB: Tuple[str, ...] = (
    "Sinking",
    "Butterfly",
    "Rupture",
    "Tremor",
    "Tremor Burst",
    "Bleed",
    "Burn",
    "Poise",
    "Haste",
    "Fragile",
    "Attack Power Up",
    "Attack Power Down",
    "Offense Level Up",
    "Offense Level Down",
    "Defense Level Up",
    "Defense Level Down",
    "Plus Coin Boost",
    "Minus Coin Drop",
    "Damage Up",
    "Damage Down",
    "Protection",
    "Bind",
    "Paralyze",
    "Deep Tears",
    "Petals",
    "Faint Aroma",
    "Bright -光-",
    "HanafudaCombo",
    "The Living & The Departed",
    "Bullet - Solitude",
    "LCA Fracture Round",
    "In the Past",
    "In the Present",
    "In the Future",
    "Temporal Disjunction",
)

#: Statuses whose amounts are encoded with the full (potency, count, stack)
#: triple; the rest use a single magnitude.
TRIPLE_STATUSES = frozenset(STATUS_VOCAB)

#: 7 Sinners plus up to 5 enemy units (the Section 5 wave is 4; the Pupa is 6
#: Parts, so the encoder tolerates more than the wave needs).
MAX_SINNERS = 7
MAX_ENEMIES = 6
MAX_UNITS = MAX_SINNERS + MAX_ENEMIES

#: Scale constants (the largest values the fixed content reaches, so features
#: land in [-1, 1]).
HP_SCALE = 30_000.0
MP_SCALE = 512.0
SP_SCALE = 45.0
SPEED_SCALE = 16.0
STACK_SCALE = 20.0
POWER_SCALE = 30.0
DAMAGE_SCALE = 200.0


def _clamp(value: float, lo: float = -3.0, hi: float = 3.0) -> float:
    return float(min(max(value, lo), hi))


@dataclass(frozen=True)
class SkillProps:
    """The numbers a decision can depend on (never the skill's name text)."""

    skill_id: str
    base_power: float
    coin_power: float
    coins: float
    attack_weight: float
    offense_level_mod: float
    sin: int  # index into SINS, -1 == unknown
    damage_type: int  # index into DAMAGE_TYPES, -1 == unknown
    is_ego: bool
    is_defense: bool
    is_unclashable: bool
    slot: int  # 0..2 for skills, 3 for defense
    ego_kind: int  # index into EGO_KINDS, -1 == not E.G.O
    ego_id: str = ""
    ego_cost: float = 0.0
    ego_sp: float = 0.0


EMPTY_PROPS = SkillProps(
    skill_id="",
    base_power=0.0,
    coin_power=0.0,
    coins=0.0,
    attack_weight=1.0,
    offense_level_mod=0.0,
    sin=-1,
    damage_type=-1,
    is_ego=False,
    is_defense=False,
    is_unclashable=False,
    slot=0,
    ego_kind=-1,
)


class SkillTable:
    """Skill properties for every Skill the fixed content can offer."""

    def __init__(self, data_dir: Optional[str] = None, uptie: str = "4") -> None:
        self.data_dir = Path(data_dir or Path(__file__).resolve().parents[2] / "data")
        self.uptie = uptie
        self._skills: Dict[str, SkillProps] = {}
        self._ego_of: Dict[str, str] = {}
        self._ego_costs: Dict[str, Dict[str, int]] = {}
        self._load()

    # -- loading -----------------------------------------------------------
    def _load(self) -> None:
        for record in self._records("identities"):
            for skill in record.get("skills", []):
                self._add_identity_skill(skill)
        for record in self._records("ego"):
            self._add_ego(record)
        for record in self._records("enemies"):
            for skill in record.get("skills", []):
                self._add_enemy_skill(skill)

    def _records(self, folder: str) -> Iterable[Dict[str, Any]]:
        for path in sorted((self.data_dir / folder).glob("*.json")):
            with path.open(encoding="utf-8") as handle:
                yield json.load(handle)

    def _uptie_block(self, skill: Dict[str, Any]) -> Dict[str, Any]:
        upties = skill.get("upties") or {}
        if self.uptie in upties:
            return upties[self.uptie]
        if upties:
            return upties[sorted(upties, key=lambda key: int(key))[-1]]
        return skill

    def _add_identity_skill(self, skill: Dict[str, Any]) -> None:
        block = self._uptie_block(skill)
        slot_name = str(skill.get("slot", "skill1"))
        if slot_name.startswith("skill"):
            slot = max(0, min(2, int(slot_name.replace("skill", "") or "1") - 1))
        elif slot_name.startswith("defense"):
            slot = 3
        else:
            slot = 0
        skills = [skill.get("id")]
        for extra in skill.get("extra_slots", []) or []:
            skills.append(extra.get("id"))
        for skill_id in skills:
            if not skill_id:
                continue
            self._skills[str(skill_id)] = SkillProps(
                skill_id=str(skill_id),
                base_power=float(block.get("base_power") or 0),
                coin_power=float(block.get("coin_power") or 0),
                coins=float(block.get("coins") or 0),
                attack_weight=float(block.get("attack_weight") or 1),
                offense_level_mod=float(block.get("offense_level_mod") or 0),
                sin=_index(SINS, block.get("sin_affinity")),
                damage_type=_index(DAMAGE_TYPES, skill.get("type")),
                is_ego=False,
                is_defense=slot == 3,
                is_unclashable="Unclashable" in (block.get("tags") or []),
                slot=slot,
                ego_kind=-1,
            )

    def _add_enemy_skill(self, skill: Dict[str, Any]) -> None:
        block = self._uptie_block(skill) if "upties" in skill else skill
        skill_id = str(skill.get("id") or "")
        if not skill_id:
            return
        tags = block.get("tags") or skill.get("tags") or []
        self._skills[skill_id] = SkillProps(
            skill_id=skill_id,
            base_power=float(block.get("base_power") or 0),
            coin_power=float(block.get("coin_power") or 0),
            coins=float(block.get("coins") or 0),
            attack_weight=float(block.get("attack_weight") or 1),
            offense_level_mod=float(block.get("offense_level_mod") or 0),
            sin=_index(SINS, block.get("sin_affinity") or block.get("sin")),
            damage_type=_index(DAMAGE_TYPES, block.get("type")),
            is_ego=False,
            is_defense=str(skill.get("slot", "")).startswith("defense"),
            is_unclashable="Unclashable" in tags,
            slot=int(skill.get("slot_index") or 0),
            ego_kind=-1,
        )

    def _add_ego(self, record: Dict[str, Any]) -> None:
        ego_id = str(record.get("id"))
        costs = record.get("resource_cost") or {}
        self._ego_costs[ego_id] = {str(k): int(v) for k, v in costs.items()}
        for kind, block in (("Awakening", record.get("awakening")), ("Corrosion", record.get("corrosion"))):
            if not block:
                continue
            skill_id = f"{ego_id}.{kind.lower()}"
            self._skills[skill_id] = SkillProps(
                skill_id=skill_id,
                base_power=float(block.get("base_power") or 0),
                coin_power=float(block.get("coin_power") or 0),
                coins=float(block.get("coins") or 0),
                attack_weight=float(block.get("attack_weight") or 1),
                offense_level_mod=float(block.get("offense_level_mod") or 0),
                sin=_index(SINS, block.get("sin") or record.get("sin_affinity")),
                damage_type=_index(DAMAGE_TYPES, block.get("type")),
                is_ego=True,
                is_defense=False,
                is_unclashable=False,
                slot=0,
                ego_kind=_index(EGO_KINDS, kind),
                ego_id=ego_id,
                ego_cost=float(sum(costs.values())),
                ego_sp=float(
                    record.get("awakening_sp" if kind == "Awakening" else "corrosion_sp") or 0
                ),
            )
            self._ego_of[skill_id] = ego_id

    # -- lookups -----------------------------------------------------------
    def props(self, skill_id: str) -> SkillProps:
        return self._skills.get(str(skill_id), EMPTY_PROPS)

    def ego_id(self, skill_id: str) -> str:
        return self._ego_of.get(str(skill_id), "")

    def props_for_wire(self, wire: Dict[str, Any], kind: str = "Assign") -> SkillProps:
        """Properties of the Skill an action would use.

        A `UseEgo` action carries `ego` + `kind` instead of a Skill id (the
        simulator keys E.G.O Skills as `<ego id>.awakening` / `.corrosion`, and
        Overclock is the Corrosion Skill at 1.5x cost), so it has to be resolved
        here or the E.G.O would look like a 0-damage action.
        """
        if kind == "UseEgo":
            ego = str(wire.get("ego") or "")
            ego_kind = str(wire.get("kind") or "Awakening")
            suffix = "corrosion" if ego_kind in ("Corrosion", "Overclock") else "awakening"
            props = self.props(f"{ego}.{suffix}")
            if props is EMPTY_PROPS or not props.ego_id:
                return SkillProps(
                    skill_id=f"{ego}.{suffix}",
                    base_power=props.base_power,
                    coin_power=props.coin_power,
                    coins=props.coins,
                    attack_weight=props.attack_weight,
                    offense_level_mod=props.offense_level_mod,
                    sin=props.sin,
                    damage_type=props.damage_type,
                    is_ego=False,
                    is_defense=False,
                    is_unclashable=False,
                    slot=0,
                    ego_kind=-1,
                )
            return props
        return self.props(str(wire.get("skill") or ""))

    def ego_cost(self, ego_id: str) -> Dict[str, int]:
        return self._ego_costs.get(str(ego_id), {})

    def ego_affordable(self, ego_id: str, resources: Dict[str, Any]) -> bool:
        """True when the team's E.G.O resources already cover this E.G.O."""
        cost = self.ego_cost(ego_id)
        if not cost:
            return False
        return all(float(resources.get(sin, 0)) >= amount for sin, amount in cost.items())

    def __len__(self) -> int:
        return len(self._skills)


def _index(values: Sequence[str], value: Any) -> int:
    if value is None:
        return -1
    text = str(value)
    for position, candidate in enumerate(values):
        if candidate.lower() == text.lower():
            return position
    return -1


class Encoder:
    """Turns observations and actions into fixed-size float vectors."""

    def __init__(self, table: Optional[SkillTable] = None, data_dir: Optional[str] = None) -> None:
        self.table = table or SkillTable(data_dir)
        self.n_status = len(STATUS_VOCAB)
        # per-unit block: 18 scalars + resistances + 3 per status + "other" (3)
        self.unit_dim = 18 + len(DAMAGE_TYPES) + len(SINS) + 3 * self.n_status + 3
        # state: global scalars + 2 x MAX_UNITS blocks + resonance level table
        self.global_dim = 12 + 2 * len(SINS)
        self.state_dim = self.global_dim + MAX_UNITS * self.unit_dim
        # action: skill block + target block + ego block + context block
        self.skill_dim = 11 + len(SINS) + len(DAMAGE_TYPES) + len(EGO_KINDS)
        self.target_dim = 1 + 4 + 2 + 4
        # Context: which actor is choosing, how far the plan has come and what
        # the earlier actors of this turn already picked.  This is what makes the
        # model autoregressive over the fixed actor order (§5, §6.4) without
        # giving every actor its own head.
        self.context_dim = 6
        self.action_dim = self.skill_dim + self.target_dim + 3 + self.context_dim

    # -- helpers -----------------------------------------------------------
    @staticmethod
    def unit_slots(obs: Dict[str, Any]) -> Tuple[List[Dict[str, Any]], List[Dict[str, Any]], Dict[str, int]]:
        """(sinners, enemies, id -> block index) in a stable order.

        The order is the deployment order the simulator uses (Sinners first), so
        a block index means the same thing in every step of a fight.
        """
        units = obs.get("units", [])
        sinners = [u for u in units if u.get("kind") == "sinner"]
        enemies = [u for u in units if u.get("kind") != "sinner"]
        order = (sinners[:MAX_SINNERS] + enemies[:MAX_ENEMIES])
        index = {unit["id"]: position for position, unit in enumerate(order)}
        return sinners, enemies, index

    def _unit_block(self, unit: Optional[Dict[str, Any]]) -> np.ndarray:
        out = np.zeros(self.unit_dim, dtype=np.float32)
        if unit is None:
            return out
        max_hp = max(1.0, float(unit.get("max_hp") or 1))
        hp = float(unit.get("hp") or 0)
        sp = unit.get("sp")
        thresholds = [float(t) for t in (unit.get("stagger_thresholds") or [])]
        consumed = int(unit.get("stagger_level") or 0)
        next_threshold = thresholds[0] if thresholds else 0.0
        for position, threshold in enumerate(sorted(thresholds)):
            if threshold < hp:
                next_threshold = threshold
                break
        values = [
            1.0,
            1.0 if unit.get("kind") == "sinner" else 0.0,
            1.0 if unit.get("alive") else 0.0,
            hp / max_hp,
            hp / HP_SCALE,
            max_hp / HP_SCALE,
            float(unit.get("shield") or 0) / MP_SCALE,
            -1.0 if sp is None else float(sp) / SP_SCALE,
            0.0 if sp is None else 1.0,
            float(unit.get("speed") or 0) / SPEED_SCALE,
            float((unit.get("speed_range") or [0, 0])[0]) / SPEED_SCALE,
            float((unit.get("speed_range") or [0, 0])[1]) / SPEED_SCALE,
            1.0 if unit.get("staggered") else 0.0,
            float(unit.get("stagger_turns") or 0) / 3.0,
            consumed / 3.0,
            next_threshold / HP_SCALE,
            1.0 if unit.get("low_morale") else 0.0,
            (1.0 if unit.get("panicked") else 0.0) + (1.0 if unit.get("corroded") else 0.0),
        ]
        out[:18] = values
        # Resistances: the same Skill is a different decision against the Imago
        # (0.75 sloth / 1.25 wrath) than against a Sinner.
        resist_sin = unit.get("resist_sin") or {}
        resist_physical = unit.get("resist_physical") or {}
        cursor = 18
        for damage_type in DAMAGE_TYPES:
            out[cursor] = _clamp(float(resist_physical.get(damage_type, 1.0)) - 1.0)
            cursor += 1
        for sin in SINS:
            out[cursor] = _clamp(float(resist_sin.get(sin, 1.0)) - 1.0)
            cursor += 1
        statuses = unit.get("statuses") or {}
        # Scale statuses by their own Potency / Count /
        # Stack columns (see MECHANICS.md rules 44-62).
        other = 0.0
        for name in STATUS_VOCAB:
            instance = statuses.get(name)
            if not instance:
                cursor += 3
                continue
            out[cursor] = _clamp(float(instance.get("potency") or 0) / STACK_SCALE)
            out[cursor + 1] = _clamp(float(instance.get("count") or 0) / 10.0)
            out[cursor + 2] = _clamp(float(instance.get("stack") or 0) / STACK_SCALE)
            cursor += 3
        for name, instance in statuses.items():
            if name in TRIPLE_STATUSES:
                continue
            other += abs(float(instance.get("potency") or 0))
            other += abs(float(instance.get("count") or 0))
            other += abs(float(instance.get("stack") or 0))
        out[cursor : cursor + 3] = [
            _clamp(other / (STACK_SCALE * 4)),
            1.0 if other > 0 else 0.0,
            0.0,
        ]
        return out

    # -- state -------------------------------------------------------------
    def encode_state(self, obs: Dict[str, Any]) -> np.ndarray:
        out = np.zeros(self.state_dim, dtype=np.float32)
        max_turns = max(1.0, float(obs.get("max_turns") or 30))
        resources = obs.get("ego_resources") or {}
        resonance = obs.get("resonance") or {}
        a_resonance = obs.get("a_resonance") or {}
        campaign = obs.get("campaign") or {}
        phase = str(obs.get("phase") or "")
        winner = obs.get("winner")
        globals_ = [
            float(obs.get("turn") or 0) / max_turns,
            min(float(obs.get("turn") or 0), 12.0) / 12.0,
            1.0 if phase == "AwaitingActions" else 0.0,
            1.0 if phase == "Finished" else 0.0,
            1.0 if winner == "Sinners" else 0.0,
            1.0 if winner == "Enemies" else 0.0,
            1.0 if winner == "Draw" else 0.0,
            1.0 if winner == "EncounterEnded" else 0.0,
            1.0 if obs.get("encounter_ended") else 0.0,
            float(campaign.get("station") or 0) / 8.0,
            (float(campaign.get("pupa_hp")) / HP_SCALE) if campaign.get("pupa_hp") else 0.0,
            float(((obs.get("rng") or {}).get("draw_count")) or 0) / 1000.0,
        ]
        out[:12] = globals_
        cursor = 12
        for sin in SINS:
            out[cursor] = min(float(resources.get(sin.lower()) or 0), 8.0) / 8.0
            cursor += 1
        for sin in SINS:
            out[cursor] = min(float(resonance.get(sin.lower()) or resonance.get(sin) or 0), 5.0) / 5.0
            cursor += 1
        units = obs.get("units", [])
        sinners = [u for u in units if u.get("kind") == "sinner"]
        enemies = [u for u in units if u.get("kind") != "sinner"]
        for position in range(MAX_UNITS):
            if position < MAX_SINNERS:
                unit = sinners[position] if position < len(sinners) else None
            else:
                offset = position - MAX_SINNERS
                unit = enemies[offset] if offset < len(enemies) else None
            out[cursor : cursor + self.unit_dim] = self._unit_block(unit)
            cursor += self.unit_dim
        return out

    # -- actions -----------------------------------------------------------
    def encode_action(
        self,
        obs: Dict[str, Any],
        action: Any,
        index: Optional[Dict[str, int]] = None,
        chosen: Sequence[Any] = (),
    ) -> np.ndarray:
        """Encode one candidate action.

        Every candidate carries the skill's numbers, its target's block, which
        actor is choosing and what the earlier actors of this turn already picked
        (`chosen`), so candidates of the same actor are directly comparable and
        the joint plan distribution is autoregressive.
        """
        _, _, index = (self.unit_slots(obs) if index is None else (None, None, index))
        out = np.zeros(self.action_dim, dtype=np.float32)
        payload = action
        kind = "Assign"
        if isinstance(action, dict):
            kind = next(iter(action.keys()), "Assign")
            payload = action[kind]
        elif hasattr(action, "to_wire"):
            decoded = json.loads(action.to_wire())
            kind = next(iter(decoded.keys()))
            payload = decoded[kind]
        else:  # pragma: no cover - defensive
            payload = {}

        skill_id = str(payload.get("skill") or "")
        props = self.table.props_for_wire(payload, kind)
        if not skill_id and props.ego_id:
            skill_id = props.skill_id
        ego_id = str(payload.get("ego") or props.ego_id or "")
        ego_kind = payload.get("kind") or (
            EGO_KINDS[props.ego_kind] if props.ego_kind >= 0 else None
        )
        is_ego = bool(payload.get("is_ego")) or kind == "UseEgo" or props.is_ego
        skill_block = [
            props.base_power / POWER_SCALE,
            props.coin_power / POWER_SCALE,
            props.coins / 5.0,
            props.attack_weight / 3.0,
            props.offense_level_mod / 10.0,
            (props.base_power + props.coin_power * props.coins) / POWER_SCALE,
            props.coins * props.coin_power / POWER_SCALE,
            1.0 if props.is_defense else 0.0,
            1.0 if props.is_unclashable else 0.0,
            props.slot / 3.0,
            1.0 if is_ego else 0.0,
        ]
        skill_block += [1.0 if props.sin == position else 0.0 for position in range(len(SINS))]
        skill_block += [
            1.0 if props.damage_type == position else 0.0 for position in range(len(DAMAGE_TYPES))
        ]
        skill_block += [
            1.0 if _index(EGO_KINDS, ego_kind) == position else 0.0
            for position in range(len(EGO_KINDS))
        ]
        out[: self.skill_dim] = skill_block

        target_id = payload.get("target") if kind != "Engage" else payload.get("enemy")
        position = index.get(str(target_id), -1) if target_id else -1
        target = None
        if target_id:
            target = next((u for u in obs.get("units", []) if u["id"] == target_id), None)
        cursor = self.skill_dim
        out[cursor] = 1.0 if target_id else 0.0
        if position >= 0:
            out[cursor + 1 : cursor + 5] = _bucket(position, MAX_UNITS)
        if target is not None and target.get("kind") == "sinner":
            target = None  # allies as targets are only for support clauses
        if target is not None:
            max_hp = max(1.0, float(target.get("max_hp") or 1))
            statuses = target.get("statuses") or {}
            sinking = statuses.get("Sinking") or {}
            time_stack = max(
                float((statuses.get(name) or {}).get("stack") or 0)
                for name in ("In the Past", "In the Present", "In the Future")
            )
            values = [
                float(target.get("hp") or 0) / max_hp,
                float(target.get("hp") or 0) / HP_SCALE,
                1.0 if target.get("alive") else 0.0,
                1.0 if target.get("staggered") else 0.0,
                1.0 if target.get("kind") != "sinner" else 0.0,
                float(sinking.get("potency") or 0) / STACK_SCALE,
                float(sinking.get("count") or 0) / 10.0,
                time_stack / STACK_SCALE,
                1.0 if target.get("max_hp") == 1 else 0.0,
                1.0 if (statuses.get("Butterfly") or {}).get("potency") else 0.0,
            ]
            out[cursor + 1 : cursor + 1 + min(len(values), self.target_dim - 1)] = values[
                : self.target_dim - 1
            ]
        cursor += self.target_dim
        resources = obs.get("ego_resources") or {}
        ego_cost = 0.0
        affordable = 0.0
        if is_ego and ego_id:
            record = self.table.props(f"{ego_id}.awakening")
            ego_cost = record.ego_cost
            affordable = 1.0
        out[cursor] = ego_cost / 10.0
        out[cursor + 1] = affordable
        out[cursor + 2] = (
            float(payload.get("enemy_slot") or 0) / 5.0 if kind == "Engage" else 0.0
        )
        cursor += 3
        actor_id = str(payload.get("actor") or "")
        actor_position = index.get(actor_id, -1)
        same_target = 0
        same_skill = 0
        prior_ego = 0
        for earlier in chosen:
            earlier_payload = payload_of(earlier)
            if target_id and earlier_payload.get("target") == target_id:
                same_target += 1
            if earlier_payload.get("skill") == skill_id:
                same_skill += 1
            if earlier_payload.get("ego") or earlier_payload.get("is_ego"):
                prior_ego += 1
        out[cursor : cursor + self.context_dim] = [
            (actor_position + 1) / MAX_UNITS if actor_position >= 0 else 0.0,
            (actor_position % 4) / 4.0 if actor_position >= 0 else 0.0,
            1.0 if actor_position >= MAX_SINNERS else 0.0,
            len(chosen) / float(MAX_SINNERS),
            same_target / 4.0,
            (same_skill + prior_ego) / 4.0,
        ]
        return out

    def action_matrix(
        self,
        obs: Dict[str, Any],
        actions: Sequence[Any],
        index: Optional[Dict[str, int]] = None,
        chosen: Sequence[Any] = (),
    ) -> np.ndarray:
        index = index or self.unit_slots(obs)[2]
        if not actions:
            return np.zeros((0, self.action_dim), dtype=np.float32)
        return np.stack(
            [self.encode_action(obs, action, index, chosen) for action in actions]
        )


def payload_of(action: Any) -> Dict[str, Any]:
    """The wire payload of an action (accepts the action classes and dicts)."""
    if isinstance(action, dict):
        wire = action
    elif hasattr(action, "to_wire"):
        wire = json.loads(action.to_wire())
    else:  # pragma: no cover - defensive
        return {}
    if not isinstance(wire, dict):
        return {}
    kind = next(iter(wire.keys()), None)
    return wire.get(kind, {}) if kind else {}


def _bucket(position: int, size: int) -> List[float]:
    """Compress a unit index into 4 normalised numbers (no huge one-hots)."""
    if position < 0:
        return [0.0, 0.0, 0.0, 0.0]
    return [
        position / size,
        (position % 4) / 4.0,
        1.0 if position >= MAX_SINNERS else 0.0,
        1.0,
    ]
