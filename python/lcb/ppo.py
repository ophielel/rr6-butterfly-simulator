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
from .plans import actor_order, group_candidates, probe_action_effects
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
    #: Optional Teacher replay constraint. Zero keeps pure PPO; formal
    #: scenario-specific runs use one small BC update per iteration.
    demo_updates_per_iteration: int = 0
    demo_batch_decisions: int = 32
    demo_decay: float = 1.0
    demo_min_updates: int = 0
    counterfactual_credit: bool = False
    counterfactual_credit_weight: float = 1.0
    counterfactual_horizon: int = 3
    action_effects: bool = False
    action_effect_cap: int = 8
    sinking_trigger_reward: float = 0.0


@dataclass
class PPOHistory:
    iteration: List[int] = field(default_factory=list)
    mean_return: List[float] = field(default_factory=list)
    win_rate: List[float] = field(default_factory=list)
    policy_loss: List[float] = field(default_factory=list)
    value_loss: List[float] = field(default_factory=list)
    demo_loss: List[float] = field(default_factory=list)
    ratio: List[float] = field(default_factory=list)
    clip_fraction: List[float] = field(default_factory=list)
    validation_win_rate: List[float] = field(default_factory=list)
    validation_kill_turn: List[float] = field(default_factory=list)
    demo_updates: List[int] = field(default_factory=list)


TurnRecord = Dict[str, Any]


def _counterfactual_actor_credits(
    env: LimbusEnv,
    before: Dict[str, Any],
    plan: Sequence[Any],
    actors: Sequence[str],
    groups: Dict[str, List[Any]],
    sinking_trigger_reward: float,
    weight: float,
    net: PolicyValueNet,
    encoder: Encoder,
    future_horizon: int,
    action_effects: bool = False,
    action_effect_cap: int = 8,
) -> Tuple[List[float], int]:
    """Scale each actor's policy gradient by a generic replacement margin."""
    if not plan or not actors or weight <= 0.0:
        return [1.0 for _ in actors], 0
    base_probe = env.clone_state()
    base_info = base_probe.step_turn(list(plan))
    if not base_info.get("ok"):
        return [1.0 for _ in actors], 0
    base_after = base_probe.observe()
    base_score = compute_reward(
        base_info,
        before,
        base_after,
        sinking_trigger_reward=sinking_trigger_reward,
    ) + _future_rollout_value(
        base_probe,
        net,
        encoder,
        future_horizon,
        sinking_trigger_reward,
        action_effects=action_effects,
        action_effect_cap=action_effect_cap,
    )
    credits: List[float] = []
    calls = 0
    for position, actor in enumerate(actors):
        chosen = plan[position] if position < len(plan) else None
        alternative = next(
            (
                option
                for option in groups.get(actor, [])
                if chosen is None or str(option.to_wire()) != str(chosen.to_wire())
            ),
            None,
        )
        if alternative is None:
            credits.append(1.0)
            continue
        counterfactual = list(plan)
        counterfactual[position] = alternative
        probe = env.clone_state()
        alternative_info = probe.step_turn(counterfactual)
        calls += 1
        if not alternative_info.get("ok"):
            credits.append(1.0)
            continue
        alternative_after = probe.observe()
        alternative_score = compute_reward(
            alternative_info,
            before,
            alternative_after,
            sinking_trigger_reward=sinking_trigger_reward,
        ) + _future_rollout_value(
            probe,
            net,
            encoder,
            future_horizon,
            sinking_trigger_reward,
            action_effects=action_effects,
            action_effect_cap=action_effect_cap,
        )
        margin = base_score - alternative_score
        credits.append(1.0 + weight * float(np.tanh(margin / 300.0)))
    return credits, calls


def _sample_plan(
    net: PolicyValueNet,
    encoder: Encoder,
    obs: Dict[str, Any],
    legal: Sequence[Any],
    rng: np.random.Generator,
    effects: Optional[Sequence[np.ndarray]] = None,
) -> Tuple[List[Any], List[Tuple[np.ndarray, int, float]], List[str]]:
    groups = group_candidates(legal)
    index = encoder.unit_slots(obs)[2]
    state_vec = encoder.encode_state(obs)
    plan: List[Any] = []
    actor_names: List[str] = []
    actor_records: List[Tuple[np.ndarray, int, float]] = []
    for position, actor in enumerate(actor_order(obs, legal)):
        options = groups.get(actor, [])
        if not options:
            continue
        actor_effects = None if effects is None else effects[position]
        matrix = encoder.action_matrix(
            obs, options, index, plan, effects=actor_effects
        )
        pick, logprob = net.sample(state_vec, matrix, rng)
        if pick < 0:
            continue
        actor_records.append((matrix, pick, logprob))
        actor_names.append(actor)
        plan.append(options[pick])
    return plan, actor_records, actor_names


def _effect_aware_sample_plan(
    env: LimbusEnv,
    net: PolicyValueNet,
    encoder: Encoder,
    obs: Dict[str, Any],
    legal: Sequence[Any],
    rng: np.random.Generator,
    cap: int,
) -> Tuple[List[Any], List[Tuple[np.ndarray, int, float]], List[str], List[np.ndarray]]:
    groups = group_candidates(legal)
    index = encoder.unit_slots(obs)[2]
    state_vec = encoder.encode_state(obs)
    plan: List[Any] = []
    records: List[Tuple[np.ndarray, int, float]] = []
    actors: List[str] = []
    effects_by_actor: List[np.ndarray] = []
    for actor in actor_order(obs, legal):
        options = groups.get(actor, [])
        if not options:
            continue
        effects = np.asarray(
            probe_action_effects(
                env,
                obs,
                legal,
                actor,
                options,
                plan,
                encoder.table,
                cap=cap,
            ),
            dtype=np.float32,
        )
        matrix = encoder.action_matrix(obs, options, index, plan, effects=effects)
        pick, logprob = net.sample(state_vec, matrix, rng)
        if pick < 0:
            continue
        plan.append(options[pick])
        records.append((matrix, pick, logprob))
        actors.append(actor)
        effects_by_actor.append(effects)
    return plan, records, actors, effects_by_actor


def _argmax_plan(
    net: PolicyValueNet,
    encoder: Encoder,
    obs: Dict[str, Any],
    legal: Sequence[Any],
    env: Optional[LimbusEnv] = None,
    action_effects: bool = False,
    action_effect_cap: int = 8,
) -> List[Any]:
    groups = group_candidates(legal)
    index = encoder.unit_slots(obs)[2]
    state_vec = encoder.encode_state(obs)
    plan: List[Any] = []
    for actor in actor_order(obs, legal):
        options = groups.get(actor, [])
        if not options:
            continue
        effects = None
        if action_effects and env is not None:
            effects = probe_action_effects(
                env, obs, legal, actor, options, plan, encoder.table, cap=action_effect_cap
            )
        matrix = encoder.action_matrix(obs, options, index, plan, effects=effects)
        pick = net.argmax(state_vec, matrix)
        if pick >= 0:
            plan.append(options[pick])
    return plan


def _future_rollout_value(
    env: LimbusEnv,
    net: PolicyValueNet,
    encoder: Encoder,
    horizon: int,
    sinking_trigger_reward: float,
    action_effects: bool = False,
    action_effect_cap: int = 8,
) -> float:
    probe = env.clone_state()
    total = 0.0
    for _ in range(max(0, horizon)):
        before = probe.observe()
        if before.get("winner") or before.get("phase") == "Finished":
            break
        legal = probe.legal_actions()
        plan = _argmax_plan(
            net,
            encoder,
            before,
            legal,
            env=probe,
            action_effects=action_effects,
            action_effect_cap=action_effect_cap,
        )
        if not plan:
            break
        info = probe.step_turn(plan)
        if not info.get("ok"):
            break
        after = probe.observe()
        total += compute_reward(
            info,
            before,
            after,
            sinking_trigger_reward=sinking_trigger_reward,
        )
    return total


def collect_episode(
    net: PolicyValueNet,
    encoder: Encoder,
    scenario: Scenario,
    seed: int,
    rng: np.random.Generator,
    sinking_trigger_reward: float = 0.0,
    counterfactual_credit: bool = False,
    counterfactual_credit_weight: float = 1.0,
    counterfactual_horizon: int = 3,
    action_effects: bool = False,
    action_effect_cap: int = 8,
) -> Tuple[List[TurnRecord], Dict[str, Any]]:
    """One PPO rollout: sampling the plan, but resolving whole turns only."""
    env = LimbusEnv(strict=scenario.strict)
    env.reset(
        seed,
        enemies=list(scenario.enemies),
        max_turns=scenario.max_turns,
        enemy_hp_scale=scenario.enemy_hp_scale,
        infinite_ego_resources=scenario.infinite_ego_resources,
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
        state_vec = encoder.encode_state(obs)
        if action_effects:
            plan, actor_records, actor_names, effect_rows = _effect_aware_sample_plan(
                env, net, encoder, obs, legal, rng, action_effect_cap
            )
        else:
            plan, actor_records, actor_names = _sample_plan(
                net, encoder, obs, legal, rng
            )
            effect_rows = None
        if counterfactual_credit:
            credits, counterfactual_calls = _counterfactual_actor_credits(
                env,
                obs,
                plan,
                actor_names,
                groups,
                sinking_trigger_reward,
                counterfactual_credit_weight,
                net,
                encoder,
                counterfactual_horizon,
                action_effects=action_effects,
                action_effect_cap=action_effect_cap,
            )
        else:
            credits, counterfactual_calls = [1.0] * len(actor_records), 0
        actors = [
            (matrix, pick, logprob, credit)
            for (matrix, pick, logprob), credit in zip(actor_records, credits)
        ]
        info = env.step_turn(plan)
        if not info.get("ok"):
            raise RuntimeError(f"PPO rollout produced an illegal plan: {info.get('error')}")
        after = env.observe()
        reward = compute_reward(
            info, obs, after, sinking_trigger_reward=sinking_trigger_reward
        )
        total_reward += reward
        turns.append(
            {
                "state": state_vec,
                "actors": actors,
                "action_effects": effect_rows,
                "reward": reward,
                "counterfactual_calls": counterfactual_calls,
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
        "counterfactual_calls": sum(
            int(turn.get("counterfactual_calls") or 0) for turn in turns
        ),
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
    demonstrations: Sequence[ds.Decision] = (),
) -> Tuple[PolicyValueNet, PPOHistory, Dict[str, Any]]:
    config = config or PPOConfig()
    history = PPOHistory()
    rng = np.random.default_rng(config.seed)
    seed_pool = list(seeds)
    if not seed_pool:
        raise ValueError("PPO needs at least one training seed")
    seed_order: List[int] = []
    seed_cursor = 0
    # The CLI learning-rate flag must affect a loaded BC checkpoint too.
    net.lr = config.learning_rate
    net.value_lr = config.learning_rate
    info: Dict[str, Any] = {
        "episodes": 0,
        "returns": [],
        "wins": 0,
        "counterfactual_calls": 0,
        "demonstration_decisions": len(demonstrations),
    }
    for iteration in range(config.iterations):
        rollouts: List[Tuple[List[TurnRecord], Dict[str, Any]]] = []
        for _ in range(config.episodes_per_iteration):
            if seed_cursor >= len(seed_order):
                seed_order = list(seed_pool)
                rng.shuffle(seed_order)
                seed_cursor = 0
            seed = seed_order[seed_cursor]
            seed_cursor += 1
            rollouts.append(
                collect_episode(
                    net,
                    encoder,
                    scenario,
                    seed,
                    rng,
                    sinking_trigger_reward=config.sinking_trigger_reward,
                    counterfactual_credit=config.counterfactual_credit,
                    counterfactual_credit_weight=config.counterfactual_credit_weight,
                    counterfactual_horizon=config.counterfactual_horizon,
                    action_effects=config.action_effects,
                    action_effect_cap=config.action_effect_cap,
                )
            )
        info["episodes"] += len(rollouts)
        info["wins"] += sum(1 for _, summary in rollouts if summary["won"])
        info["counterfactual_calls"] += sum(
            int(summary.get("counterfactual_calls") or 0) for _, summary in rollouts
        )
        episode_returns = [summary["return"] for _, summary in rollouts]
        info["returns"].extend(episode_returns)

        decisions: List[Tuple[np.ndarray, List[Tuple], float]] = []
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
                        list(turn["actors"]),
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
        demo_loss = 0.0
        demo_updates = 0
        if demonstrations and config.demo_updates_per_iteration > 0:
            decay = max(0.0, float(config.demo_decay))
            demo_updates = max(
                config.demo_min_updates,
                int(round(config.demo_updates_per_iteration * (decay ** iteration))),
            )
            demo_losses: List[float] = []
            for _ in range(demo_updates):
                if len(demonstrations) <= config.demo_batch_decisions:
                    demo_batch = list(demonstrations)
                else:
                    demo_indices = rng.choice(
                        len(demonstrations), config.demo_batch_decisions, replace=False
                    )
                    demo_batch = [demonstrations[int(index)] for index in demo_indices]
                demo_losses.append(net.bc_update(demo_batch))
            demo_loss = float(np.mean(demo_losses)) if demo_losses else 0.0

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
        history.demo_loss.append(demo_loss)
        history.ratio.append(float(policy_stats["ratio"]))
        history.clip_fraction.append(float(policy_stats["clip_frac"]))
        if log is not None:
            log.append("")
        validation = {"win_rate": 0.0, "kill_turn": None}
        if config.validation_seeds and config.validation_episodes:
            validation = evaluate_argmax(
                net,
                encoder,
                scenario,
                config.validation_seeds,
                config.validation_episodes,
                action_effects=config.action_effects,
                action_effect_cap=config.action_effect_cap,
            )
        history.validation_win_rate.append(validation["win_rate"])
        history.validation_kill_turn.append(
            float(validation["kill_turn"]) if validation["kill_turn"] else 0.0
        )
        history.demo_updates.append(demo_updates)
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
                f"demo_loss={history.demo_loss[-1]:.3f} "
                f"demo_updates={demo_updates} "
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
    action_effects: bool = False,
    action_effect_cap: int = 8,
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
            infinite_ego_resources=scenario.infinite_ego_resources,
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
                effects = None
                if action_effects:
                    effects = probe_action_effects(
                        env,
                        obs,
                        legal,
                        actor,
                        options,
                        plan,
                        encoder.table,
                        cap=action_effect_cap,
                    )
                matrix = encoder.action_matrix(
                    obs, options, index, plan, effects=effects
                )
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


__all__ = [
    "PPOConfig",
    "PPOHistory",
    "train_ppo",
    "collect_episode",
    "returns_from_rewards",
]
