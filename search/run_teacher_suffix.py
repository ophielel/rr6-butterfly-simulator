#!/usr/bin/env python3
"""Search successful suffixes from states reached by an existing policy.

The prefix policy only supplies a legal starting state. The BeamTeacher then
searches from that state to the normal terminal condition, and only the suffix
teacher decisions are written to the dataset.
"""

from __future__ import annotations

import argparse
import json
import multiprocessing
import sys
import time
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path
from typing import Any, Dict, List, Sequence, Tuple

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "python"))


def _add_turn_stats(stats: Any, turn_stats: Dict[str, Any], before: Dict[str, Any]) -> None:
    stats.damage_to_enemies += float(turn_stats.get("damage_to_enemies") or 0)
    stats.damage_to_allies += float(turn_stats.get("damage_to_allies") or 0)
    stats.sinking_damage += float(turn_stats.get("sinking_damage") or 0)
    stats.sinking_sp_damage += float(turn_stats.get("sinking_sp_damage") or 0)
    stats.sinking_triggers += int(turn_stats.get("sinking_triggers") or 0)
    stats.deaths += sum(
        1
        for unit_id in turn_stats.get("deaths") or []
        if any(
            unit["id"] == unit_id and unit.get("kind") == "sinner"
            for unit in before.get("units", [])
        )
    )
    for ego in turn_stats.get("ego_uses") or []:
        if ego not in stats.ego_uses:
            stats.ego_uses.append(ego)
    stats.skill_uses.append(list(turn_stats.get("skill_uses") or []))


def _replay_entry(
    info: Dict[str, Any], plan: Sequence[Any], reward: float, before: Dict[str, Any]
) -> Dict[str, Any]:
    from lcb.plans import canonical
    from lcb.teacher import boss_sinking

    return {
        "turn": info.get("turn"),
        "plan": [json.loads(canonical(action)) for action in plan],
        "state_hash_before": info.get("state_hash_before"),
        "state_hash_after": info.get("state_hash_after"),
        "transition_hash": info.get("transition_hash"),
        "reward": reward,
        "stats": info.get("stats") or {},
        "boss_sinking": boss_sinking(before),
    }


def _run_one(args: Tuple[int, Dict[str, Any]]) -> Dict[str, Any]:
    seed, cfg = args
    from lcb.baselines import NeuralPolicy
    from lcb.dataset import save_dataset
    from lcb.env import LimbusEnv
    from lcb.features import Encoder
    from lcb.nn import PolicyValueNet
    from lcb.rewards import EpisodeStats, compute_reward
    from lcb.scenarios import scenario
    from lcb.teacher import (
        BeamTeacher,
        TeacherConfig,
        TeacherSample,
        ValueWeights,
        detect_axis,
        detect_strategy_labels,
        encode_samples,
        imago,
    )

    encoder = Encoder()
    net = PolicyValueNet.load(cfg["checkpoint"])
    prefix_policy = NeuralPolicy(net, encoder, sample=False, seed=seed)
    scene = scenario(cfg["scenario"])
    env = LimbusEnv(strict=scene.strict)
    env.reset(
        seed,
        enemies=list(scene.enemies),
        max_turns=cfg["max_turns"],
        enemy_hp_scale=cfg["enemy_hp_scale"],
        infinite_ego_resources=cfg["infinite_ego_resources"],
    )
    initial = env.observe()
    initial_boss = imago(initial)
    boss_hp_start = float((initial_boss or {}).get("hp") or 1.0)
    stats = EpisodeStats(seed=seed, boss_hp_start=boss_hp_start)
    replay: List[Dict[str, Any]] = []

    prefix_turns = 0
    prefix_won = False
    while prefix_turns < cfg["prefix_turns"]:
        before = env.observe()
        if before.get("winner") or before.get("phase") == "Finished":
            prefix_won = before.get("winner") == "Sinners"
            break
        legal = env.legal_actions()
        plan = prefix_policy.plan(env, before, legal)
        if not plan:
            break
        info = env.step_turn(plan)
        if not info.get("ok"):
            break
        reward = compute_reward(info, before, env.observe())
        stats.per_turn_reward.append(reward)
        stats.turns = int(info.get("turn") or stats.turns + 1)
        _add_turn_stats(stats, info.get("stats") or {}, before)
        replay.append(_replay_entry(info, plan, reward, before))
        prefix_turns += 1

    teacher = BeamTeacher(
        encoder.table,
        encoder,
        TeacherConfig(
            horizon=cfg["horizon"],
            plan_width=cfg["plan_width"],
            candidate_cap=cfg["candidate_cap"],
            turn_width=cfg["turn_width"],
            score_mode=cfg["score_mode"],
            rollout_width=cfg["rollout_width"],
            candidate_mode=cfg["candidate_mode"],
            leaf_value_weight=cfg["leaf_value_weight"],
            counterfactual_credit=cfg["counterfactual_credit"],
            sinking_trigger_reward=cfg["sinking_trigger_reward"],
            max_turns=cfg["max_turns"],
            enemy_hp_scale=cfg["enemy_hp_scale"],
            infinite_ego_resources=cfg["infinite_ego_resources"],
        ),
        ValueWeights(
            boss_hp=cfg["boss_hp_weight"],
            turn=cfg["turn_weight"],
            ally_damage=cfg["ally_damage_weight"],
            death=cfg["death_weight"],
        ),
    )
    teacher._boss_hp_start = boss_hp_start
    teacher._allies_hp_start = sum(
        float(unit.get("hp") or 0)
        for unit in initial.get("units", [])
        if unit.get("kind") == "sinner"
    ) or 1.0
    samples = []
    suffix_start_turn = int(env.observe().get("turn") or 0)
    while True:
        before = env.observe()
        if before.get("winner") or before.get("phase") == "Finished":
            break
        if int(before.get("turn") or 0) >= cfg["max_turns"]:
            break
        legal = env.legal_actions()
        plan, value, groups, actors = teacher.plan_turn(env, before, legal)
        if not plan:
            break
        actor_credit = (
            teacher._counterfactual_actor_credit(env, before, plan, groups, actors)
            if cfg["counterfactual_credit"]
            else None
        )
        info = env.step_turn(plan)
        if not info.get("ok"):
            raise RuntimeError(f"suffix plan rejected: {info.get('error')}")
        after = env.observe()
        reward = teacher._reward(info, before, after)
        stats.per_turn_reward.append(reward)
        stats.turns = int(info.get("turn") or stats.turns + 1)
        _add_turn_stats(stats, info.get("stats") or {}, before)
        replay.append(_replay_entry(info, plan, reward, before))
        samples.append(
            TeacherSample(
                turn=int(before.get("turn") or 0),
                obs=before,
                groups=groups,
                plan=plan,
                actors=actors,
                value=value,
                seed=seed,
                actor_credit=actor_credit,
            )
        )

    final = env.observe()
    stats.winner = final.get("winner")
    stats.won = stats.winner == "Sinners"
    stats.kill_turn = stats.turns if stats.won else None
    stats.boss_hp_left = float((imago(final) or {}).get("hp") or 0)
    stats.survivors = sum(
        1
        for unit in final.get("units", [])
        if unit.get("kind") == "sinner" and unit.get("alive")
    )
    stats.axis = detect_axis(replay, samples, table=encoder.table)
    stats.axis.update(detect_strategy_labels(replay))
    boss_progress = 1.0 - stats.boss_hp_left / stats.boss_hp_start if stats.boss_hp_start else 0.0
    quality = 1.0 if stats.won else max(
        0.0, min(1.0, boss_progress * (0.75 + 0.25 * stats.survivors / 7.0))
    )
    for sample in samples:
        sample.episode_won = stats.won
        sample.kill_turn = stats.kill_turn
        sample.episode_axis = stats.axis
        sample.episode_quality = quality

    out = Path(cfg["out"])
    out.mkdir(parents=True, exist_ok=True)
    arrays = encode_samples(
        encoder,
        samples,
        quality_weighting=cfg["quality_weighting"],
        quality_power=cfg["quality_power"],
        failure_weight=cfg["failure_weight"],
        min_quality=cfg["min_quality"],
        credit_weighting=cfg["counterfactual_credit"],
    )
    save_dataset(out / f"seed_{seed:05d}.npz", arrays)
    if stats.won:
        replay_dir = out / "replays"
        replay_dir.mkdir(parents=True, exist_ok=True)
        with (replay_dir / f"teacher_seed_{seed:05d}.json").open("w", encoding="utf-8") as handle:
            json.dump(replay, handle, ensure_ascii=False)
    row = stats.to_row()
    row.update(
        {
            "prefix_turns": prefix_turns,
            "prefix_won": prefix_won,
            "suffix_start_turn": suffix_start_turn,
            "suffix_samples": len(samples),
            "search": dict(teacher.stats),
        }
    )
    return row


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenario", default="real")
    parser.add_argument("--checkpoint", required=True)
    parser.add_argument("--seed-start", type=int, default=100001)
    parser.add_argument("--seed-count", type=int, default=100)
    parser.add_argument("--prefix-turns", type=int, default=3)
    parser.add_argument("--horizon", type=int, default=5)
    parser.add_argument("--plan-width", type=int, default=64)
    parser.add_argument("--candidate-cap", type=int, default=8)
    parser.add_argument("--turn-width", type=int, default=5)
    parser.add_argument("--rollout-width", type=int, default=6)
    parser.add_argument("--score-mode", choices=["heuristic", "rollout"], default="rollout")
    parser.add_argument("--candidate-mode", choices=["top", "diverse", "all"], default="diverse")
    parser.add_argument("--leaf-value-weight", type=float, default=5.0)
    parser.add_argument("--boss-hp-weight", type=float, default=100.0)
    parser.add_argument("--turn-weight", type=float, default=0.0)
    parser.add_argument("--ally-damage-weight", type=float, default=100.0)
    parser.add_argument("--death-weight", type=float, default=200.0)
    parser.add_argument("--quality-weighting", action=argparse.BooleanOptionalAction, default=True)
    parser.add_argument("--quality-power", type=float, default=2.0)
    parser.add_argument("--failure-weight", type=float, default=0.1)
    parser.add_argument("--min-quality", type=float, default=0.0)
    parser.add_argument("--counterfactual-credit", action=argparse.BooleanOptionalAction, default=False)
    parser.add_argument("--sinking-trigger-reward", type=float, default=0.0)
    parser.add_argument("--max-turns", type=int, default=None)
    parser.add_argument("--enemy-hp-scale", type=float, default=None)
    parser.add_argument("--infinite-ego-resources", action=argparse.BooleanOptionalAction, default=None)
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    from lcb.evaluate import best_of_n, provenance, summarise
    from lcb.scenarios import scenario

    scene = scenario(args.scenario)
    max_turns = args.max_turns if args.max_turns is not None else scene.max_turns
    enemy_hp_scale = args.enemy_hp_scale if args.enemy_hp_scale is not None else scene.enemy_hp_scale
    infinite = (
        args.infinite_ego_resources
        if args.infinite_ego_resources is not None
        else scene.infinite_ego_resources
    )
    cfg = vars(args).copy()
    cfg.update(
        {
            "max_turns": max_turns,
            "enemy_hp_scale": enemy_hp_scale,
            "infinite_ego_resources": infinite,
        }
    )
    seeds = list(range(args.seed_start, args.seed_start + args.seed_count))
    started = time.time()
    rows: List[Dict[str, Any]] = []
    if args.jobs > 1:
        with ProcessPoolExecutor(
            max_workers=args.jobs, mp_context=multiprocessing.get_context("spawn")
        ) as pool:
            for row in pool.map(_run_one, [(seed, cfg) for seed in seeds]):
                rows.append(row)
                print(
                    f"seed {row['seed']}: won={row['won']} prefix={row['prefix_turns']} "
                    f"samples={row['suffix_samples']}",
                    flush=True,
                )
    else:
        for seed in seeds:
            row = _run_one((seed, cfg))
            rows.append(row)
            print(
                f"seed {row['seed']}: won={row['won']} prefix={row['prefix_turns']} "
                f"samples={row['suffix_samples']}",
                flush=True,
            )
    rows.sort(key=lambda row: row["seed"])
    index = {
        "provenance": provenance(scene, seeds, {"suffix_teacher": cfg}),
        "elapsed_seconds": round(time.time() - started, 1),
        "prefix": {"checkpoint": args.checkpoint, "turns": args.prefix_turns},
        "summary": summarise(rows),
        "best_of_n": best_of_n(rows, (1, 10, 50)),
        "episodes": rows,
    }
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    with (out / "teacher_index.json").open("w", encoding="utf-8") as handle:
        json.dump(index, handle, ensure_ascii=False, indent=2)
    print(json.dumps(index["summary"], ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
