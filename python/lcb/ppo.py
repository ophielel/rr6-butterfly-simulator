"""Stage C: PPO fine-tuning (TRAINING_PLAN.md §6).

The environment step is a whole turn, so the advantage/return unit is a turn and
the action log-probability is the **sum** of the seven actors' log-probabilities
(the joint plan).  Nothing else is special-cased: early Harmony, early Solemn
Lament or meaningless stacking are punished only by the terminal and process
results, exactly as the plan requires.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple

import numpy as np

from . import dataset as ds
from .baselines import NeuralPolicy
from .env import LimbusEnv
from .evaluate import Scenario, run_episode
from .features import Encoder
from .nn import VALUE_SCALE, PolicyValueNet
from .plans import actor_order, group_candidates
from .rewards import compute_reward
from .teacher import detect_axis, imago


@dataclass
class PPOConfig:
    iterations: int = 4
    episodes_per_iteration: int = 8
    epochs_per_iteration: int = 1
    learning_rate: float = 1e-3
    clip: float = 0.2
    value_coef: float = 0.5
    entropy_coef: float = 0.0
    seed: int = 0
    gamma: float = 1.0
    standardize_advantage: bool = True
    #: Seeds of the **validation** band used to pick the checkpoint to keep
    #: (0 = keep the last one).  The test band is never touched here.
    validation_seeds: Tuple[int, ...] = ()
    validation_episodes: int = 0


@dataclass
class PPOHistory:
    iteration: List[int] = field(default_factory=list)
    mean_return: List[float] = field(default_factory=list)
    win_rate: List[float] = field(default_factory=list)
    policy_loss: List[float] = field(default_factory=list)
    value_loss: List[float] = field(default_factory=list)
    ratio: List[float] = field(default_factory=list)
    clip_fraction: List[float] = field(default_factory=list)
    validation_win_rate: List[float] = field(default_factory=list)
    validation_kill_turn: List[float] = field(default_factory=list)


TurnRecord = Dict[str, Any]


def collect_episode(
    net: PolicyValueNet,
    encoder: Encoder,
    scenario: Scenario,
    seed: int,
    rng: np.random.Generator,
) -> Tuple[List[TurnRecord], Dict[str, Any]]:
    """One PPO rollout: sampling the plan, but resolving whole turns only."""
    env = LimbusEnv(strict=scenario.strict)
    env.reset(
        seed,
        enemies=list(scenario.enemies),
        max_turns=scenario.max_turns,
        enemy_hp_scale=scenario.enemy_hp_scale,
    )
    first = env.observe()
    turns: List[TurnRecord] = []
    replay: List[Dict[str, Any]] = []
    total_reward = 0.0
    while True:
        obs = env.observe()
        if obs.get("winner") or obs.get("phase") == "Finished":
            break
        legal = env.legal_actions()
        groups = group_candidates(legal)
        index = encoder.unit_slots(obs)[2]
        state_vec = encoder.encode_state(obs)
        plan: List[Any] = []
        actors: List[Tuple[np.ndarray, int, float]] = []
        for actor in actor_order(obs, legal):
            options = groups.get(actor, [])
            if not options:
                continue
            matrix = encoder.action_matrix(obs, options, index, plan)
            pick, logprob = net.sample(state_vec, matrix, rng)
            if pick < 0:
                continue
            actors.append((matrix, pick, logprob))
            plan.append(options[pick])
        info = env.step_turn(plan)
        if not info.get("ok"):
            raise RuntimeError(f"PPO rollout produced an illegal plan: {info.get('error')}")
        after = env.observe()
        reward = compute_reward(info, obs, after)
        total_reward += reward
        turns.append(
            {
                "state": state_vec,
                "actors": actors,
                "reward": reward,
                "obs": obs,
                "done": bool(after.get("winner")),
            }
        )
        replay.append(
            {
                "turn": info.get("turn"),
                "reward": reward,
                "stats": info.get("stats") or {},
                "state_hash_before": info.get("state_hash_before"),
                "transition_hash": info.get("transition_hash"),
            }
        )
    final = env.observe()
    summary = {
        "seed": seed,
        "return": total_reward,
        "winner": final.get("winner"),
        "won": final.get("winner") == "Sinners",
        "turns": len(turns),
        "boss_hp_left": float((imago(final) or {}).get("hp") or 0),
        "axis": detect_axis(replay),
    }
    return turns, summary


def returns_from_rewards(rewards: Sequence[float], gamma: float = 1.0) -> List[float]:
    out: List[float] = []
    running = 0.0
    for reward in reversed(list(rewards)):
        running = reward + gamma * running
        out.append(running)
    return list(reversed(out))


def train_ppo(
    net: PolicyValueNet,
    encoder: Encoder,
    scenario: Scenario,
    seeds: Sequence[int],
    config: Optional[PPOConfig] = None,
    log: Optional[List[str]] = None,
) -> Tuple[PolicyValueNet, PPOHistory, Dict[str, Any]]:
    config = config or PPOConfig()
    history = PPOHistory()
    rng = np.random.default_rng(config.seed)
    seed_pool = list(seeds)
    info: Dict[str, Any] = {"episodes": 0, "returns": [], "wins": 0}
    for iteration in range(config.iterations):
        rollouts: List[Tuple[List[TurnRecord], Dict[str, Any]]] = []
        for _ in range(config.episodes_per_iteration):
            seed = int(rng.choice(seed_pool))
            rollouts.append(collect_episode(net, encoder, scenario, seed, rng))
        info["episodes"] += len(rollouts)
        info["wins"] += sum(1 for _, summary in rollouts if summary["won"])
        episode_returns = [summary["return"] for _, summary in rollouts]
        info["returns"].extend(episode_returns)

        decisions: List[Tuple[np.ndarray, List[Tuple[np.ndarray, int, float]], float]] = []
        values: List[Tuple[np.ndarray, float]] = []
        for turns, _ in rollouts:
            # The value head is trained on normalised returns (VALUE_SCALE); the
            # advantages and the reported episode return stay in reward units.
            rewards = [turn["reward"] / VALUE_SCALE for turn in turns]
            returns = returns_from_rewards(rewards, config.gamma)
            for turn, ret in zip(turns, returns):
                advantage = ret - net.value(turn["state"])
                decisions.append(
                    (
                        turn["state"],
                        [(cand, action, logprob) for cand, action, logprob in turn["actors"]],
                        float(advantage),
                    )
                )
                values.append((turn["state"], float(ret)))
        if config.standardize_advantage and decisions:
            raw = np.asarray([d[2] for d in decisions], dtype=float)
            if raw.std() > 1e-6:
                mean, std = float(raw.mean()), float(raw.std())
                decisions = [(state, actors, (adv - mean) / std) for state, actors, adv in decisions]

        policy_stats = {"loss": 0.0, "ratio": 1.0, "clip_frac": 0.0}
        for _ in range(config.epochs_per_iteration):
            order = rng.permutation(len(decisions))
            for start in range(0, len(order), 32):
                batch = [decisions[i] for i in order[start : start + 32]]
                policy_stats = net.ppo_update(batch, clip=config.clip)
        value_loss = 0.0
        order = rng.permutation(len(values))
        for start in range(0, len(order), 32):
            batch = [values[i] for i in order[start : start + 32]]
            value_loss += net.value_update(batch, value_coef=config.value_coef)

        history.iteration.append(iteration + 1)
        history.mean_return.append(float(np.mean(episode_returns)))
        history.win_rate.append(
            sum(1 for _, summary in rollouts if summary["won"]) / len(rollouts)
        )
        history.policy_loss.append(float(policy_stats["loss"]))
        history.value_loss.append(float(value_loss))
        history.ratio.append(float(policy_stats["ratio"]))
        history.clip_fraction.append(float(policy_stats["clip_frac"]))
        if log is not None:
            log.append("")
        validation = {"win_rate": 0.0, "kill_turn": None}
        if config.validation_seeds and config.validation_episodes:
            validation = evaluate_argmax(
                net, encoder, scenario, config.validation_seeds, config.validation_episodes
            )
        history.validation_win_rate.append(validation["win_rate"])
        history.validation_kill_turn.append(
            float(validation["kill_turn"]) if validation["kill_turn"] else 0.0
        )
        if validation["win_rate"] > info.get("best_validation_win_rate", -1.0):
            info["best_validation_win_rate"] = validation["win_rate"]
            info["best_iteration"] = iteration + 1
            info["best_params"] = {k: v.copy() for k, v in net.params.items()}
        if log is not None and config.validation_seeds:
            log[-1] += (
                f" val_win={validation['win_rate']:.2f}"
                f" val_kill={validation['kill_turn']}"
            )
        if log is not None:
            line = (
                f"iteration {iteration + 1}/{config.iterations}: "
                f"return={history.mean_return[-1]:.2f} win_rate={history.win_rate[-1]:.2f} "
                f"policy_loss={history.policy_loss[-1]:.3f} value_loss={history.value_loss[-1]:.1f} "
                f"ratio={history.ratio[-1]:.3f} clip={history.clip_fraction[-1]:.2f}"
            )
            if config.validation_seeds:
                line += (
                    f" val_win={validation['win_rate']:.2f}"
                    f" val_kill={validation['kill_turn']}"
                )
            log[-1] = line
    info["win_rate"] = info["wins"] / info["episodes"] if info["episodes"] else 0.0
    info["mean_return"] = float(np.mean(info["returns"])) if info["returns"] else 0.0
    best = info.pop("best_params", None)
    if best is not None:
        net.params.update(best)
        info["selected_by_validation"] = True
    info.pop("returns", None)
    return net, history, info


def evaluate_argmax(
    net: PolicyValueNet,
    encoder: Encoder,
    scenario: Scenario,
    seeds: Sequence[int],
    episodes: int = 0,
) -> Dict[str, Any]:
    """Win rate and median kill turn of the greedy (argmax) policy."""
    used = list(seeds)[:episodes] if episodes else list(seeds)
    wins = 0
    kills: List[int] = []
    for seed in used:
        env = LimbusEnv(strict=scenario.strict)
        env.reset(
            seed,
            enemies=list(scenario.enemies),
            max_turns=scenario.max_turns,
            enemy_hp_scale=scenario.enemy_hp_scale,
        )
        turns = 0
        while True:
            obs = env.observe()
            if obs.get("winner") or obs.get("phase") == "Finished":
                break
            legal = env.legal_actions()
            groups = group_candidates(legal)
            index = encoder.unit_slots(obs)[2]
            state_vec = encoder.encode_state(obs)
            plan: List[Any] = []
            for actor in actor_order(obs, legal):
                options = groups.get(actor, [])
                if not options:
                    continue
                matrix = encoder.action_matrix(obs, options, index, plan)
                pick = net.argmax(state_vec, matrix)
                if pick < 0:
                    continue
                plan.append(options[pick])
            if not plan:
                break
            info = env.step_turn(plan)
            if not info.get("ok"):
                break
            turns = int(info.get("turn") or turns + 1)
            if turns >= scenario.max_turns:
                break
        final = env.observe()
        if final.get("winner") == "Sinners":
            wins += 1
            kills.append(turns)
    return {
        "win_rate": wins / len(used) if used else 0.0,
        "kill_turn": float(np.median(kills)) if kills else None,
        "episodes": len(used),
    }


__all__ = ["PPOConfig", "PPOHistory", "train_ppo", "collect_episode", "returns_from_rewards"]
