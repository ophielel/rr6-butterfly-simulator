"""Whole-turn plan generation (TRAINING_PLAN.md §2.1, §4.2).

A *plan* is one action for every unit that can act, generated actor by actor in a
fixed order.  Nothing is submitted to the simulator while a plan is still being
built: the only simulator calls are the action mask (`legal_actions`) and - once
the plan is complete - the turn resolution.  That is the rule the whole training
stack rests on, so it lives in one place.

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

from .env import Action, EngageAction, EgoAction, LimbusEnv
from .features import DAMAGE_TYPES, SINS, SkillTable

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

    `width` is the number of partial plans kept after each actor (the plan's
    baseline is 32), `cap` bounds the candidates considered per actor (they are
    pre-sorted by the analytic prior), and `rollout` marks a function that scores
    a partial plan with the real simulator.
    """

    table: SkillTable
    width: int = 8
    cap: int = 6
    rollout: Optional[RolloutFn] = None
    scores: List[float] = field(default_factory=list)

    def candidate_actions(
        self, obs: Dict[str, Any], legal: Sequence[Any], actor: str
    ) -> List[Any]:
        grouped = group_candidates(legal)
        options = grouped.get(actor, [])
        if self.cap and len(options) > self.cap:
            scored = sorted(
                options,
                key=lambda action: estimate_action(obs, action, self.table),
                reverse=True,
            )
            options = scored[: self.cap]
        return options

    def generate(
        self, env: LimbusEnv, obs: Dict[str, Any], legal: Sequence[Any]
    ) -> List[Tuple[float, List[Any]]]:
        """Every plan the beam keeps, best first (score = prior of the prefix)."""
        actors = actor_order(obs, legal)
        beams: List[Tuple[float, List[Any]]] = [(0.0, [])]
        for actor in actors:
            options = self.candidate_actions(obs, legal, actor)
            if not options:
                continue
            expanded: List[Tuple[float, List[Any]]] = []
            for score, plan in beams:
                for option in options:
                    prior = estimate_action(obs, option, self.table)
                    expanded.append((score + prior, plan + [option]))
            if self.rollout is not None:
                rescored = [
                    (self.rollout(env, plan), plan) for _, plan in expanded
                ]
                expanded = rescored
            expanded.sort(key=lambda item: item[0], reverse=True)
            beams = expanded[: self.width]
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
