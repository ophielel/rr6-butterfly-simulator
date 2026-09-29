"""Baselines and the shared policy interface (TRAINING_PLAN.md §7).

```text
Random     - uniform over the legal actions of each actor
FirstLegal - the first legal action of each actor, in actor order
Greedy     - one-turn lookahead evaluated through the simulator itself
AI         - a behaviour-cloned / PPO-fine-tuned policy (`lcb.nn`)
```

Every policy returns a **complete plan**; the turn is resolved by
`LimbusEnv.step_turn`, so a policy can never observe or influence a mid-turn
result.
"""

from __future__ import annotations

import random
from dataclasses import dataclass
from typing import Any, Dict, List, Optional, Sequence

import numpy as np

from .env import LimbusEnv
from .features import Encoder, SkillTable
from .nn import VALUE_SCALE
from .plans import (
    PlanGenerator,
    actor_order,
    canonical,
    first_legal_plan,
    group_candidates,
    payload,
    probe_action_effects,
)
from .rewards import compute_reward
from .teacher import ValueWeights, imago, state_value


class Policy:
    """A policy maps the current state to one complete turn plan."""

    name: str = "policy"

    def plan(self, env: LimbusEnv, obs: Dict[str, Any], legal: Sequence[Any]) -> List[Any]:
        raise NotImplementedError

    #: some policies need an episode start to calibrate their value function
    def start_episode(self, obs: Dict[str, Any]) -> None:
        return None


class RandomPolicy(Policy):
    name = "Random"

    def __init__(self, seed: int = 0) -> None:
        self.rng = random.Random(seed)

    def plan(self, env: LimbusEnv, obs: Dict[str, Any], legal: Sequence[Any]) -> List[Any]:
        groups = group_candidates(legal)
        plan: List[Any] = []
        for actor in actor_order(obs, legal):
            options = groups.get(actor, [])
            if options:
                plan.append(self.rng.choice(options))
        return plan


class FirstLegalPolicy(Policy):
    name = "FirstLegal"

    def plan(self, env: LimbusEnv, obs: Dict[str, Any], legal: Sequence[Any]) -> List[Any]:
        return first_legal_plan(obs, legal)


class GreedyPolicy(Policy):
    """One-turn lookahead through the real simulator.

    For each actor, in the fixed actor order, every considered candidate is
    submitted into a **clone** and the turn is resolved with the remaining actors
    filled by the first-legal rule; the clone with the best
    `reward + value(state)` wins and is carried into the next actor's evaluation.
    `cap` bounds the candidates per actor by the analytic prior (0 = all of them),
    which is what makes the baseline affordable; the report states which was used.
    """

    name = "Greedy"

    def __init__(
        self,
        table: SkillTable,
        cap: int = 8,
        weights: Optional[ValueWeights] = None,
    ) -> None:
        self.table = table
        self.cap = cap
        self.weights = weights or ValueWeights()
        self._boss_hp_start = 1.0
        self._allies_hp_start = 1.0

    def start_episode(self, obs: Dict[str, Any]) -> None:
        boss = imago(obs)
        self._boss_hp_start = float((boss or {}).get("hp") or 1.0)
        self._allies_hp_start = (
            sum(
                float(u.get("hp") or 0)
                for u in obs.get("units", [])
                if u.get("kind") == "sinner"
            )
            or 1.0
        )

    def _score(self, info: Dict[str, Any], before: Dict[str, Any], after: Dict[str, Any]) -> float:
        return compute_reward(info, before, after) + state_value(
            after,
            self.table,
            self.weights,
            self._boss_hp_start,
            self._allies_hp_start,
            int(before.get("turn") or 0),
        )

    def plan(self, env: LimbusEnv, obs: Dict[str, Any], legal: Sequence[Any]) -> List[Any]:
        generator = PlanGenerator(self.table, width=1, cap=self.cap)
        chosen: List[Any] = []
        actors = actor_order(obs, legal)
        for position, actor in enumerate(actors):
            options = generator.candidate_actions(obs, legal, actor)
            if not options:
                continue
            best: Optional[tuple] = None
            for option in options:
                probe = env.clone_state()
                for action in chosen:
                    probe.submit(action)
                probe.submit(option)
                # The remaining actors are filled with the first-legal rule so
                # the turn can resolve at all.
                plan = chosen + [option]
                for other in actors[position + 1 :]:
                    rest = group_candidates(legal).get(other, [])
                    if rest:
                        plan.append(rest[0])
                info = probe.step_turn(plan)
                if not info.get("ok"):
                    continue
                score = self._score(info, obs, probe.observe())
                if best is None or score > best[0]:
                    best = (score, option)
            if best is not None:
                chosen.append(best[1])
        return chosen


class TeacherPolicy(Policy):
    """The stage-A search teacher itself (the plan's reference policy).

    It is expensive - every decision expands a turn-level tree - so it is a
    reference, not the shipped policy; the shipped policy is the network trained
    to imitate it (`models/ai.npz`).
    """

    name = "Teacher"

    def __init__(self, table: SkillTable, encoder: Encoder, teacher=None) -> None:
        from .teacher import BeamTeacher, TeacherConfig

        self.table = table
        self.teacher = teacher or BeamTeacher(table, encoder, TeacherConfig())
        self._encoder = encoder

    def start_episode(self, obs: Dict[str, Any]) -> None:
        boss = imago(obs)
        self.teacher._boss_hp_start = float((boss or {}).get("hp") or 1.0)
        self.teacher._allies_hp_start = (
            sum(
                float(u.get("hp") or 0)
                for u in obs.get("units", [])
                if u.get("kind") == "sinner"
            )
            or 1.0
        )

    def plan(self, env: LimbusEnv, obs: Dict[str, Any], legal: Sequence[Any]) -> List[Any]:
        plan, _value, _groups, _actors = self.teacher.plan_turn(env, obs, legal)
        return plan


class NeuralPolicy(Policy):
    """Argmax (or sampling) plan from a `PolicyValueNet`."""

    name = "AI"

    def __init__(
        self,
        net,
        encoder: Encoder,
        sample: bool = False,
        seed: int = 0,
        name: Optional[str] = None,
        action_effects: bool = False,
        action_effect_cap: int = 8,
    ) -> None:
        self.net = net
        self.encoder = encoder
        self.sample = sample
        self.rng = np.random.default_rng(seed)
        if name:
            self.name = name
        self.action_effects = bool(action_effects) and bool(
            getattr(net, "uses_action_effects", lambda: True)()
        )
        self.action_effect_cap = int(action_effect_cap)
        self.last_logprobs: List[float] = []

    def _plan_once(
        self,
        obs: Dict[str, Any],
        legal: Sequence[Any],
        effects: Optional[Sequence[np.ndarray]] = None,
    ) -> List[Any]:
        groups = group_candidates(legal)
        index = self.encoder.unit_slots(obs)[2]
        state_vec = self.encoder.encode_state(obs)
        plan: List[Any] = []
        self.last_logprobs = []
        for position, actor in enumerate(actor_order(obs, legal)):
            options = groups.get(actor, [])
            if not options:
                continue
            actor_effects = None if effects is None else effects[position]
            matrix = self.encoder.action_matrix(
                obs, options, index, plan, effects=actor_effects
            )
            if self.sample:
                pick, logprob = self.net.sample(state_vec, matrix, self.rng)
            else:
                pick, logprob = self.net.argmax(state_vec, matrix), 0.0
            if pick < 0:
                continue
            plan.append(options[pick])
            self.last_logprobs.append(float(logprob))
        return plan

    def plan(self, env: LimbusEnv, obs: Dict[str, Any], legal: Sequence[Any]) -> List[Any]:
        if not self.action_effects or not self.encoder.effect_dim:
            return self._plan_once(obs, legal)
        groups = group_candidates(legal)
        index = self.encoder.unit_slots(obs)[2]
        state_vec = self.encoder.encode_state(obs)
        plan: List[Any] = []
        self.last_logprobs = []
        for actor in actor_order(obs, legal):
            options = groups.get(actor, [])
            if not options:
                continue
            effects = probe_action_effects(
                env,
                obs,
                legal,
                actor,
                options,
                plan,
                self.encoder.table,
                cap=self.action_effect_cap,
            )
            matrix = self.encoder.action_matrix(
                obs, options, index, plan, effects=effects
            )
            if self.sample:
                pick, logprob = self.net.sample(state_vec, matrix, self.rng)
            else:
                pick, logprob = self.net.argmax(state_vec, matrix), 0.0
            if pick < 0:
                continue
            plan.append(options[pick])
            self.last_logprobs.append(float(logprob))
        return plan


class NeuralLookaheadPolicy(Policy):
    """The network plus a bounded, simulator-anchored lookahead.

    This is the classic "learned prior + search" combination, not a different
    game: the candidate actions are the network's own top-`branch` choices taken
    from the simulator's legal set, the outcomes come from real `step_turn`
    clones, and the leaf score is the network's value head (the critic PPO
    trains).  Nothing here names a skill, a status or a route.

    Budget per resolved turn:

    ```text
    complete plans considered   <= beam
    simulator turns evaluated   <= beam * (1 + horizon)
    ```

    so the cost is a stated constant, unlike `GreedyPolicy`, whose cost grows
    with the number of actors and candidates.  The plan the network would have
    produced alone is always one of the candidates, so this can only change the
    decision when the lookahead actively prefers another legal plan.
    """

    name = "AI+search"

    def __init__(
        self,
        net,
        encoder: Encoder,
        beam: int = 8,
        branch: int = 4,
        horizon: int = 1,
        name: Optional[str] = None,
        action_effects: bool = False,
        action_effect_cap: int = 8,
    ) -> None:
        self.net = net
        self.encoder = encoder
        self.beam = max(1, int(beam))
        self.branch = max(1, int(branch))
        self.horizon = max(0, int(horizon))
        self.action_effects = bool(action_effects) and bool(
            getattr(net, "uses_action_effects", lambda: True)()
        )
        self.action_effect_cap = int(action_effect_cap)
        if name:
            self.name = name
        self.last_plan_source = "policy"
        self._boss_hp_start = 1.0
        self._allies_hp_start = 1.0

    # -- candidate plans ---------------------------------------------------
    def _candidate_plans(
        self, env: LimbusEnv, obs: Dict[str, Any], legal: Sequence[Any]
    ) -> List[List[Any]]:
        """Beam over the network's own top-`branch` actions per actor."""
        groups = group_candidates(legal)
        index = self.encoder.unit_slots(obs)[2]
        # `logits`/`probs` take the *embedded* state (`net.embed`), not the raw
        # encoder vector.
        embedded = self.net.embed(self.encoder.encode_state(obs))
        beams: List[List[Any]] = [[]]
        for actor in actor_order(obs, legal):
            options = groups.get(actor, [])
            if not options:
                continue
            expanded: List[List[Any]] = []
            for plan in beams:
                effects = None
                if self.action_effects:
                    effects = probe_action_effects(
                        env,
                        obs,
                        legal,
                        actor,
                        options,
                        plan,
                        self.encoder.table,
                        cap=self.action_effect_cap,
                    )
                matrix = self.encoder.action_matrix(
                    obs, options, index, plan, effects=effects
                )
                scores = np.asarray(self.net.logits(embedded, matrix)).ravel()
                top = np.argsort(scores)[::-1][: self.branch]
                for pick in top:
                    expanded.append(plan + [options[int(pick)]])
            if not expanded:
                continue
            # Keeping the best `beam` by the network's *own* score means the
            # argmax plan is always retained as a candidate.
            expanded.sort(
                key=lambda plan: -self._plan_logprob(embedded, index, obs, groups, plan)
            )
            beams = expanded[: self.beam]
        return [plan for plan in beams if plan]

    def _plan_logprob(
        self,
        embedded: np.ndarray,
        index: Dict[str, int],
        obs: Dict[str, Any],
        groups: Dict[str, List[Any]],
        plan: Sequence[Any],
    ) -> float:
        """Sum of the network's log-probs over the actors of one plan."""
        total = 0.0
        prefix: List[Any] = []
        for position, action in enumerate(plan):
            actor = str(payload(action).get("actor") or "")
            options = groups.get(actor, [])
            if not options:
                continue
            matrix = self.encoder.action_matrix(obs, options, index, prefix)
            logits = self.net.logits(embedded, matrix)
            probs = self.net._softmax(np.asarray(logits))
            if not np.all(np.isfinite(probs)) or probs.sum() <= 0:
                return total
            try:
                label = next(
                    i for i, option in enumerate(options) if canonical(option) == canonical(action)
                )
            except StopIteration:  # pragma: no cover - defensive
                return total
            total += float(np.log(max(probs[label], 1e-12)))
            prefix.append(action)
        return total

    # -- scoring -----------------------------------------------------------
    def _score_plan(
        self, env: LimbusEnv, plan: Sequence[Any], obs: Dict[str, Any]
    ) -> float:
        probe = env.clone_state()
        before = probe.observe()
        info = probe.step_turn(list(plan))
        if not info.get("ok"):
            return -1e9
        after = probe.observe()
        score = compute_reward(info, before, after)
        # The leaf value has to be `V` of the state *after* the simulated turns,
        # otherwise the bootstrap value is counted on top of the reward that was
        # already collected for reaching it.
        raw_state = self.encoder.encode_state(after)
        for _ in range(self.horizon):
            if after.get("winner") or after.get("phase") == "Finished":
                break
            follow_legal = probe.legal_actions()
            follow = self._argmax_plan(probe, after, follow_legal)
            if not follow:
                break
            follow_info = probe.step_turn(follow)
            if not follow_info.get("ok"):
                break
            follow_after = probe.observe()
            score += compute_reward(follow_info, after, follow_after)
            after = follow_after
            raw_state = self.encoder.encode_state(after)
        # `value` takes the raw encoder vector; `logits` takes the embedded one.
        return score + float(self.net.value(raw_state)) * VALUE_SCALE

    def _argmax_plan(
        self, env: LimbusEnv, obs: Dict[str, Any], legal: Sequence[Any]
    ) -> List[Any]:
        groups = group_candidates(legal)
        index = self.encoder.unit_slots(obs)[2]
        raw_state = self.encoder.encode_state(obs)
        plan: List[Any] = []
        for actor in actor_order(obs, legal):
            options = groups.get(actor, [])
            if not options:
                continue
            effects = None
            if self.action_effects:
                effects = probe_action_effects(
                    env,
                    obs,
                    legal,
                    actor,
                    options,
                    plan,
                    self.encoder.table,
                    cap=self.action_effect_cap,
                )
            matrix = self.encoder.action_matrix(obs, options, index, plan, effects=effects)
            # `argmax`/`probs`/`value` all take the raw state and embed it, while
            # `logits` takes the embedded state; keep the two apart explicitly.
            pick = self.net.argmax(raw_state, matrix)
            if pick >= 0:
                plan.append(options[pick])
        return plan

    def start_episode(self, obs: Dict[str, Any]) -> None:
        boss = imago(obs)
        self._boss_hp_start = float((boss or {}).get("hp") or 1.0)
        self._allies_hp_start = (
            sum(
                float(unit.get("hp") or 0)
                for unit in obs.get("units", [])
                if unit.get("kind") == "sinner"
            )
            or 1.0
        )

    def plan(self, env: LimbusEnv, obs: Dict[str, Any], legal: Sequence[Any]) -> List[Any]:
        candidates = self._candidate_plans(env, obs, legal)
        reference = self._argmax_plan(env, obs, legal)
        if not candidates:
            return reference
        best: Optional[List[Any]] = None
        best_score = -float("inf")
        for plan in candidates:
            score = self._score_plan(env, plan, obs)
            if score > best_score:
                best_score, best = score, plan
        if best is None:
            return reference
        same = len(best) == len(reference) and all(
            canonical(left) == canonical(right)
            for left, right in zip(best, reference)
        )
        self.last_plan_source = "policy" if same else "lookahead"
        return best


__all__ = [
    "Policy",
    "TeacherPolicy",
    "RandomPolicy",
    "FirstLegalPolicy",
    "GreedyPolicy",
    "NeuralPolicy",
]
