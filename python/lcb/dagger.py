"""DAgger-lite: label the states the *policy* reaches (§5's imitation gap).

Plain behaviour cloning only sees the states the teacher visits.  Once the policy
deviates, it lands in states the teacher never labelled and the error compounds
over the seven actors of a turn.  This module plays episodes where the teacher
decides some turns and the current policy decides the rest, and asks the teacher
for a label on **every** visited state - the standard fix, and the only change to
the pipeline (the labels still come from the same stage-A teacher).

`policy_prob` is the probability that the policy, not the teacher, chooses the
turn; every visited state is labelled by the teacher either way.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Dict, List, Optional, Sequence, Tuple

import numpy as np

from .env import LimbusEnv
from .features import Encoder
from .plans import actor_order, canonical, group_candidates
from .rewards import EpisodeStats, compute_reward
from .teacher import BeamTeacher, TeacherSample, boss_sinking, detect_axis, imago


def collect_episode(
    policy,
    teacher: BeamTeacher,
    encoder: Encoder,
    seed: int,
    scenario,
    policy_prob: float = 0.5,
    rng: Optional[np.random.Generator] = None,
) -> Tuple[EpisodeStats, List[TeacherSample], List[Dict[str, Any]]]:
    rng = rng or np.random.default_rng(seed)
    env = LimbusEnv(strict=scenario.strict)
    env.reset(
        seed,
        enemies=list(scenario.enemies),
        max_turns=scenario.max_turns,
        enemy_hp_scale=scenario.enemy_hp_scale,
    )
    first = env.observe()
    boss = imago(first)
    teacher._boss_hp_start = float((boss or {}).get("hp") or 1.0)
    teacher._allies_hp_start = (
        sum(
            float(u.get("hp") or 0)
            for u in first.get("units", [])
            if u.get("kind") == "sinner"
        )
        or 1.0
    )
    stats = EpisodeStats(seed=seed, boss_hp_start=teacher._boss_hp_start)
    samples: List[TeacherSample] = []
    replay: List[Dict[str, Any]] = []
    while True:
        obs = env.observe()
        if obs.get("winner") or obs.get("phase") == "Finished":
            break
        legal = env.legal_actions()
        if not legal:
            break
        plan, value, groups, actors = teacher.plan_turn(env, obs, legal)
        if not plan:
            break
        samples.append(
            TeacherSample(
                turn=int(obs.get("turn") or 0),
                obs=obs,
                groups=groups,
                plan=plan,
                actors=actors,
                value=value,
                seed=seed,
            )
        )
        if rng.random() < policy_prob:
            # Deviate: the policy drives into its own distribution.
            try:
                alternative = policy.plan(env, obs, legal)
            except Exception:  # pragma: no cover - defensive
                alternative = []
            if alternative:
                plan = alternative
        info = env.step_turn(plan)
        if not info.get("ok"):
            raise RuntimeError(f"dagger plan rejected: {info.get('error')}")
        after = env.observe()
        reward = compute_reward(info, obs, after)
        stats.per_turn_reward.append(reward)
        stats.turns = int(info.get("turn") or stats.turns + 1)
        turn_stats = info.get("stats") or {}
        stats.damage_to_enemies += float(turn_stats.get("damage_to_enemies") or 0)
        stats.sinking_damage += float(turn_stats.get("sinking_damage") or 0)
        stats.sinking_triggers += int(turn_stats.get("sinking_triggers") or 0)
        for ego in turn_stats.get("ego_uses") or []:
            if ego not in stats.ego_uses:
                stats.ego_uses.append(ego)
        replay.append(
            {
                "turn": info.get("turn"),
                "reward": reward,
                "stats": turn_stats,
                "boss_sinking": boss_sinking(obs),
                "transition_hash": info.get("transition_hash"),
            }
        )
        if stats.turns >= scenario.max_turns:
            break
    final = env.observe()
    stats.winner = final.get("winner")
    stats.won = final.get("winner") == "Sinners"
    stats.kill_turn = stats.turns if stats.won else None
    stats.boss_hp_left = float((imago(final) or {}).get("hp") or 0)
    stats.survivors = sum(
        1 for u in final.get("units", []) if u.get("kind") == "sinner" and u.get("alive")
    )
    stats.axis = detect_axis(replay)
    for sample in samples:
        sample.episode_won = stats.won
        sample.kill_turn = stats.kill_turn
        sample.episode_axis = stats.axis
    return stats, samples, replay


__all__ = ["collect_episode"]
