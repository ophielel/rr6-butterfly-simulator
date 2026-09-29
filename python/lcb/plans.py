"""Whole-turn plan generation (TRAINING_PLAN.md §2.1, §4.2).

A *plan* is one action for every unit that can act, generated actor by actor in a
fixed order.  Normal generation never submits a partial plan to the simulator:
it uses the legal-action mask and resolves only complete plans.  The optional effect
probe also uses complete-turn clones while a plan is being assembled; it never treats
a partial submission as a game transition.  That invariant lives in one place.

The inner (actor-level) beam needs a score for a *partial* plan.  Two modes are
supported:

* `heuristic` - an analytic prior computed from the candidate skills' own numbers
  and the target's resistances.  It never touches the simulator, which keeps the
  search affordable; it is only a *prior*, the value of a completed plan always
  comes from a real simulation (see `lcb.teacher`).
* `rollout` - fill the partial plan with a fallback, resolve the turn in a clone
  and read the value.  Faithful but far more expensive.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Callable, Dict, Iterable, List, Optional, Sequence, Tuple

import numpy as np

from .env import Action, EngageAction, EgoAction, LimbusEnv
from .features import (
    ACTION_EFFECT_DIM,
    DAMAGE_TYPES,
    SINS,
    SkillTable,
    action_effect_fingerprint,
)

ACTION_TYPES = (Action, EngageAction, EgoAction)


def decode(action: Any) -> Any:
    """The wire form of an action, whichever action class it is.

    `Commit` is a unit variant and travels as the plain string `"Commit"`; it is
    never part of a plan (the plan itself resolves the turn), so it decodes to
    the string and every helper below treats it as "no payload".
    """
    if isinstance(action, str):
        return action
    if isinstance(action, dict):
        return action
    return json.loads(action.to_wire())


def payload(action: Any) -> Dict[str, Any]:
    wire = decode(action)
    if not isinstance(wire, dict):
        return {}
    kind = next(iter(wire.keys()), None)
    if kind is None:
        return {}
    value = wire[kind]
    return value if isinstance(value, dict) else {}


def action_actor(action: Any) -> str:
    return str(payload(action).get("actor") or "")


def canonical(action: Any) -> str:
    """Stable string identity of an action (used for masks and replays)."""
    wire = decode(action)
    if not isinstance(wire, dict):
        return str(wire)
    return json.dumps(wire, sort_keys=True, separators=(",", ":"))


def actor_order(obs: Dict[str, Any], legal: Sequence[Any]) -> List[str]:
    """The units that must act this turn, in the simulator's own order.

    `legal_actions` is the authority: a Staggered unit offers nothing and is not
    part of the plan.
    """
    actors: List[str] = []
    for action in legal:
        actor = action_actor(action)
        if actor and actor not in actors:
            actors.append(actor)
    order = {unit["id"]: position for position, unit in enumerate(obs.get("units", []))}
    return sorted(actors, key=lambda actor: order.get(actor, 1 << 30))


def group_candidates(legal: Sequence[Any]) -> Dict[str, List[Any]]:
    grouped: Dict[str, List[Any]] = {}
    seen: Dict[str, set] = {}
    for action in legal:
        if not isinstance(action, ACTION_TYPES):
            continue
        if action_actor(action) == "":
            continue
        actor = action_actor(action)
        key = canonical(action)
        bucket = grouped.setdefault(actor, [])
        if seen.setdefault(actor, set()).__contains__(key):
            continue
        seen[actor].add(key)
        bucket.append(action)
    return grouped


def probe_action_effects(
    env: LimbusEnv,
    obs: Dict[str, Any],
    legal: Sequence[Any],
    actor: str,
    options: Sequence[Any],
    chosen: Sequence[Any],
    table: SkillTable,
    cap: int = 8,
) -> List[np.ndarray]:
    """Probe a diverse subset of candidates by resolving complete turn clones."""
    effects = [np.zeros(ACTION_EFFECT_DIM, dtype=np.float32) for _ in options]
    if not options:
        return effects
    actor_positions = actor_order(obs, legal)
    if actor not in actor_positions:
        return effects
    selected = PlanGenerator(
        table=table,
        width=1,
        cap=max(1, cap),
        candidate_mode="diverse",
    )
    probe_options = (
        list(options)
        if cap <= 0
        else selected.select_candidates(obs, legal, actor, options)
    )
    option_indices = {canonical(option): position for position, option in enumerate(options)}
    groups = group_candidates(legal)
    chosen_actors = {action_actor(action) for action in chosen}
    for option in probe_options:
        position = option_indices.get(canonical(option))
        if position is None:
            continue
        plan = list(chosen) + [option]
        selected_now = chosen_actors | {actor}
        for remaining in actor_positions:
            if remaining in selected_now:
                continue
            candidates = groups.get(remaining, [])
            if candidates:
                plan.append(candidates[0])
        probe = env.clone_state()
        info = probe.step_turn(plan)
        if info.get("ok"):
            effects[position] = action_effect_fingerprint(
                obs,
                probe.observe(),
                info,
                valid=True,
                target_id=str(payload(option).get("target") or "") or None,
            )
        else:
            effects[position] = action_effect_fingerprint(
                obs,
                obs,
                info,
                valid=False,
                target_id=str(payload(option).get("target") or "") or None,
            )
    return effects


# ---------------------------------------------------------------------------
# Analytic prior
# ---------------------------------------------------------------------------

#: Chance a Coin lands Heads: `H = 50 + SP` percent (wiki.gg `Sanity`).
def heads_chance(sp: Optional[int]) -> float:
    if sp is None:
        return 0.5
    return min(max((50 + sp) / 100.0, 0.05), 1.0)


def estimate_action(obs: Dict[str, Any], action: Any, table: SkillTable) -> float:
    """Cheap expected-value prior of a single action.

    It is deliberately simple: expected Coin damage against the target's
    resistances, discounted when the target is a 1 HP illusion (half of that
    damage is transferred to the Imago, the rest is wasted), plus a small bonus
    for setting up [Sinking] and a penalty for spending E.G.O resources.
    """
    wire = payload(action)
    kind = next(iter(decode(action).keys()), "Assign") if isinstance(decode(action), dict) else "Commit"
    props = table.props_for_wire(wire, kind)
    target_id = wire.get("target")
    target = None
    if target_id:
        target = next((u for u in obs.get("units", []) if u["id"] == target_id), None)
    if target is not None and target.get("kind") == "sinner":
        target = None

    if props.is_defense:
        # A Guard/Evade is worth its shield/evade value; kept small so that
        # attacking stays attractive while the team is healthy.
        return 1.5

    actor_id = str(wire.get("actor") or "")
    actor = next((u for u in obs.get("units", []) if u["id"] == actor_id), None)
    chance = heads_chance(actor.get("sp") if actor else None)
    coins = max(1.0, props.coins)
    expected = 0.0
    for index in range(int(coins)):
        expected += props.base_power + props.coin_power * index * chance
    expected *= 1.0 + 0.02 * (props.offense_level_mod)
    if props.damage_type >= 0 and target is not None:
        resist = float((target.get("resist_physical") or {}).get(DAMAGE_TYPES[props.damage_type], 1.0))
        expected *= resist
    if props.sin >= 0 and target is not None:
        resist = float((target.get("resist_sin") or {}).get(SINS[props.sin], 1.0))
        expected *= resist
    if target is not None:
        hp = float(target.get("hp") or 0)
        if hp <= 1:
            # 1 HP Illusory Butterfly: Origination transfers half of the damage
            # to the Imago, the rest cannot reduce its HP below 1.
            expected *= 0.5
        else:
            expected = min(expected, hp)
        if target.get("staggered"):
            expected *= 1.2
    else:
        expected *= 0.1  # untargeted ("self") skills do not deal damage
    if props.is_ego:
        expected = expected * 1.35 - 0.4 * props.ego_cost
    if kind == "Engage":
        expected *= 0.9
    return float(expected)


# ---------------------------------------------------------------------------
# Plan generation
# ---------------------------------------------------------------------------

#: Value of a partial plan plus a rollout value callback.
RolloutFn = Callable[[LimbusEnv, List[Any]], float]

@dataclass
class PlanGenerator:
    """Beam over actor-ordered partial plans.

    `width` is the number of partial plans kept after each actor, `cap` bounds
    the candidates considered per actor, and `rollout` scores a partial plan by
    completing it with legal fallback actions and resolving the whole turn.
    ``candidate_mode=\"diverse\"`` prevents the analytic prior from removing
    every defense, E.G.O, low-output, or target-diverse option before search can
    inspect it. The categories are structural and numeric; they do not name a
    status or route.
    """

    table: SkillTable
    width: int = 8
    cap: int = 6
    rollout: Optional[RolloutFn] = None
    candidate_mode: str = "top"  # "top" | "diverse" | "all"
    scores: List[float] = field(default_factory=list)
    selection_stats: Dict[str, int] = field(default_factory=dict)
    first_candidates: Dict[str, List[Any]] = field(default_factory=dict)

    def __post_init__(self) -> None:
        if self.candidate_mode not in {"top", "diverse", "all"}:
            raise ValueError(f"unknown candidate mode: {self.candidate_mode}")

    def _bucket(self, action: Any, score: float) -> str:
        wire = payload(action)
        kind = next(iter(decode(action).keys()), "Assign") if isinstance(decode(action), dict) else "Assign"
        props = self.table.props_for_wire(wire, kind)
        if kind == "UseEgo" or props.is_ego:
            return "ego"
        if props.is_defense:
            return "defense"
        raw_power = props.base_power + props.coin_power * max(0.0, props.coins)
        if raw_power <= 0.0 or score <= 0.25:
            return "low_output"
        return "attack"

    @staticmethod
    def _target_key(action: Any) -> str:
        wire = payload(action)
        return str(wire.get("target") or wire.get("enemy") or "<none>")

    def _record_selection(
        self, ranked: Sequence[Tuple[float, Any]], selected: Sequence[Any]
    ) -> None:
        stats = self.selection_stats
        stats["candidate_calls"] = stats.get("candidate_calls", 0) + 1
        stats["candidate_available"] = stats.get("candidate_available", 0) + len(ranked)
        stats["candidate_kept"] = stats.get("candidate_kept", 0) + len(selected)
        selected_keys = {canonical(action) for action in selected}
        top_score = ranked[0][0] if ranked else 0.0
        low_available = 0
        low_kept = 0
        available_buckets = set()
        kept_buckets = set()
        for score, action in ranked:
            bucket = self._bucket(action, score)
            available_buckets.add(bucket)
            if score <= top_score * 0.5:
                low_available += 1
            if canonical(action) in selected_keys:
                kept_buckets.add(bucket)
                if score <= top_score * 0.5:
                    low_kept += 1
        stats["low_output_available"] = stats.get("low_output_available", 0) + (
            1 if "low_output" in available_buckets else 0
        )
        stats["low_output_kept"] = stats.get("low_output_kept", 0) + (
            1 if "low_output" in kept_buckets else 0
        )
        stats["low_prior_available"] = stats.get("low_prior_available", 0) + low_available
        stats["low_prior_kept"] = stats.get("low_prior_kept", 0) + low_kept
        for bucket in available_buckets:
            key = f"bucket_{bucket}_available"
            stats[key] = stats.get(key, 0) + 1
        for bucket in kept_buckets:
            key = f"bucket_{bucket}_kept"
            stats[key] = stats.get(key, 0) + 1

    def candidate_actions(
        self, obs: Dict[str, Any], legal: Sequence[Any], actor: str
    ) -> List[Any]:
        grouped = group_candidates(legal)
        return self.select_candidates(obs, legal, actor, grouped.get(actor, []))

    def select_candidates(
        self,
        obs: Dict[str, Any],
        legal: Sequence[Any],
        actor: str,
        options: Sequence[Any],
    ) -> List[Any]:
        """`candidate_actions` for an already-grouped option list.

        The grouping and the analytic prior cost O(#legal) each call, and the
        probe below asks for the same actor's candidates once per probe, so the
        grouped list is passed in instead of being rebuilt every time.
        """
        ranked = sorted(
            options,
            key=lambda action: estimate_action(obs, action, self.table),
            reverse=True,
        )
        if self.candidate_mode == "all" or not self.cap or len(ranked) <= self.cap:
            selected = ranked
        elif self.candidate_mode == "top":
            selected = ranked[: self.cap]
        else:
            selected = []
            selected_keys = set()

            def add(action: Any) -> None:
                key = canonical(action)
                if key not in selected_keys and len(selected) < self.cap:
                    selected_keys.add(key)
                    selected.append(action)

            # Keep a representative from every structural action family first.
            for bucket in ("attack", "defense", "ego", "low_output"):
                for action in ranked:
                    score = estimate_action(obs, action, self.table)
                    if self._bucket(action, score) == bucket:
                        add(action)
                        break
            # Then keep target diversity, which is important even when all
            # candidates belong to the same numeric family.
            seen_targets = set()
            for action in ranked:
                target = self._target_key(action)
                if target not in seen_targets:
                    seen_targets.add(target)
                    add(action)
            for action in ranked:
                add(action)
        scored = [
            (estimate_action(obs, action, self.table), action) for action in ranked
        ]
        self._record_selection(scored, selected)
        return selected

    def generate(
        self, env: LimbusEnv, obs: Dict[str, Any], legal: Sequence[Any]
    ) -> List[Tuple[float, List[Any]]]:
        """Every plan the beam keeps, best first (score = prior of the prefix)."""
        actors = actor_order(obs, legal)
        beams: List[Tuple[float, List[Any]]] = [(0.0, [])]
        self.first_candidates = {}
        capture_root = True
        for actor in actors:
            options = self.candidate_actions(obs, legal, actor)
            if capture_root:
                self.first_candidates[actor] = list(options)
            if not options:
                continue
            expanded: List[Tuple[float, List[Any]]] = []
            for score, plan in beams:
                for option in options:
                    prior = estimate_action(obs, option, self.table)
                    expanded.append((score + prior, plan + [option]))
            # Prefix scores stay cheap so every actor can expand a useful beam.
            # Running a full simulator rollout for each partial prefix is both
            # wasteful and biased by the arbitrary fallback actions. Score only
            # complete plans with the simulator after the actor beam is finished.
            expanded.sort(key=lambda item: item[0], reverse=True)
            beams = expanded[: self.width]
        if self.rollout is not None:
            beams = [(self.rollout(env, plan), plan) for _prior, plan in beams]
            beams.sort(key=lambda item: item[0], reverse=True)
        self.scores = [score for score, _ in beams]
        return beams

    def best_plan(self, env: LimbusEnv, obs: Dict[str, Any], legal: Sequence[Any]) -> List[Any]:
        plans = self.generate(env, obs, legal)
        return plans[0][1] if plans else []


def first_legal_plan(obs: Dict[str, Any], legal: Sequence[Any]) -> List[Any]:
    """One action per actor, taking the first legal option of each (baseline)."""
    plan: List[Any] = []
    for actor in actor_order(obs, legal):
        grouped = group_candidates(legal)
        options = grouped.get(actor, [])
        if options:
            plan.append(options[0])
    return plan
