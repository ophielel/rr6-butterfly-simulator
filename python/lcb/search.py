"""Search layer.

The plan's ordering is explicit: random -> greedy -> beam -> MCTS -> learning,
and greedy must compare actions *through the real simulator*, never through a
hand-written damage estimate.  Every function here therefore clones the
environment, plays the action out, and reads the resulting state.
"""

from __future__ import annotations

import random
from typing import Any, Dict, List, Optional, Tuple

from .env import LimbusEnv, Action, EgoAction


def _default_action_for(env: LimbusEnv, actor: str, used: set) -> Optional[Any]:
    """Fallback assignment for slots the caller has not filled yet."""
    candidates = [
        a
        for a in env.legal_actions()
        if isinstance(a, (Action, EgoAction)) and a.actor == actor and actor not in used
    ]
    if not candidates:
        return None
    # attack the enemy with the lowest HP
    state = env.state()
    hp = {
        unit["id"]: unit["hp"]
        for unit in state["units"]
        if "Sinner" not in unit.get("kind", {})
    }
    return min(candidates, key=lambda a: hp.get(a.target, 1 << 30))


def _fill_and_commit(env: LimbusEnv, assigned: Optional[List[Any]] = None) -> Dict[str, Any]:
    """Submit what we have, top the rest up with the fallback, then commit."""
    assigned = assigned or []
    used = set()
    for action in assigned:
        env.step(action)
        used.add(action.actor)
    guard = 0
    while guard < 32:
        state = env.state()
        pending = [
            unit["id"]
            for unit in state["units"]
            if "Sinner" in unit.get("kind", {})
            and unit.get("alive")
            and unit["id"] not in used
        ]
        if not pending:
            break
        action = _default_action_for(env, pending[0], used)
        if action is None:
            break
        env.step(action)
        used.add(action.actor)
        guard += 1
    return env.commit()


def greedy_turn(env: LimbusEnv, verbose: bool = False) -> List[Any]:
    """Sequentially pick the action with the best simulated outcome.

    For every legal action of the current unit the environment is cloned, the
    turn is played to completion and the resulting state is scored.  The winner
    is applied to the real environment; the next unit is then evaluated with it
    already committed, which keeps the cost linear in the number of units.
    """
    chosen: List[Any] = []
    current = env.clone_state()
    evaluated = 0
    guard = 0
    while guard < 32:
        guard += 1
        state = current.state()
        pending = [
            unit["id"]
            for unit in state["units"]
            if "Sinner" in unit.get("kind", {})
            and unit.get("alive")
            and not unit.get("stagger", {}).get("turns_remaining", 0)
            and unit["id"] not in {a.actor for a in chosen}
        ]
        if not pending:
            break
        actor = pending[0]
        options = [
            a
            for a in current.legal_actions()
            if isinstance(a, (Action, EgoAction)) and a.actor == actor
        ]
        if not options:
            break
        best: Tuple[float, Optional[Any]] = (float("-inf"), None)
        for option in options:
            probe = current.clone_state()
            probe.step(option)
            _fill_and_commit(probe, [])
            evaluated += 1
            score = LimbusEnv.score(probe.state())
            if score > best[0]:
                best = (score, option)
        if best[1] is None:
            break
        chosen.append(best[1])
        if verbose:
            print(f"  {actor}: {best[1]} (score {best[0]:.0f})")
    return chosen


def random_turn(env: LimbusEnv, rng: Optional[random.Random] = None) -> List[Any]:
    """Uniformly sample one legal action per unit for this turn."""
    rng = rng or random.Random()
    chosen: List[Any] = []
    probe = env.clone_state()
    guard = 0
    while guard < 32:
        guard += 1
        options = [
            a
            for a in probe.legal_actions()
            if isinstance(a, (Action, EgoAction))
            and a.actor not in {c.actor for c in chosen}
        ]
        if not options:
            break
        action = rng.choice(options)
        chosen.append(action)
        probe.step(action)
    return chosen


def beam_turn(env: LimbusEnv, width: int = 3, horizon: int = 1) -> List[Any]:
    """Beam search over turn-level plans (stage 3 of the plan's search ladder).

    `horizon` turns are planned; each beam entry is a list of actions.  Only
    real simulations are used for scoring.
    """
    if horizon < 1:
        raise ValueError("horizon must be >= 1")
    beams: List[Tuple[float, List[Any], LimbusEnv]] = [(0.0, [], env.clone_state())]
    for _ in range(horizon):
        expanded: List[Tuple[float, List[Any], LimbusEnv]] = []
        for _, plan, node in beams:
            for action in node.legal_actions():
                if not isinstance(action, (Action, EgoAction)):
                    continue
                probe = node.clone_state()
                probe.step(action)
                expanded.append((0.0, plan + [action], probe))
        if not expanded:
            break
        scored: List[Tuple[float, List[Any], LimbusEnv]] = []
        for _, plan, node in expanded:
            probe = node.clone_state()
            _fill_and_commit(probe, [])
            scored.append((LimbusEnv.score(probe.state()), plan, node))
        scored.sort(key=lambda item: item[0], reverse=True)
        beams = scored[:width]
    return beams[0][1] if beams else []


def mcts_turn(*_args, **_kwargs):  # pragma: no cover - intentionally not done
    """Stage 4 of the plan's search ladder.

    Not implemented on purpose: the plan requires the simulator and the fixed
    content to be finished and verified first.  Implemented so far: random,
    greedy, beam.
    """
    raise NotImplementedError(
        "MCTS is intentionally not implemented yet (see docs/STATUS.md)"
    )
