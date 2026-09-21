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
from .plans import (
    PlanGenerator,
    actor_order,
    first_legal_plan,
    group_candidates,
    payload,
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
    ) -> None:
        self.net = net
        self.encoder = encoder
        self.sample = sample
        self.rng = np.random.default_rng(seed)
        if name:
            self.name = name
        self.last_logprobs: List[float] = []

    def plan(self, env: LimbusEnv, obs: Dict[str, Any], legal: Sequence[Any]) -> List[Any]:
        groups = group_candidates(legal)
        index = self.encoder.unit_slots(obs)[2]
        state_vec = self.encoder.encode_state(obs)
        plan: List[Any] = []
        self.last_logprobs = []
        for actor in actor_order(obs, legal):
            options = groups.get(actor, [])
            if not options:
                continue
            matrix = self.encoder.action_matrix(obs, options, index, plan)
            if self.sample:
                pick, logprob = self.net.sample(state_vec, matrix, self.rng)
            else:
                pick, logprob = self.net.argmax(state_vec, matrix), 0.0
            if pick < 0:
                continue
            plan.append(options[pick])
            self.last_logprobs.append(float(logprob))
        return plan


__all__ = [
    "Policy",
    "TeacherPolicy",
    "RandomPolicy",
    "FirstLegalPolicy",
    "GreedyPolicy",
    "NeuralPolicy",
]
