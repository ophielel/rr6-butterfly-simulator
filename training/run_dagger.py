#!/usr/bin/env python3
"""Collect DAgger corrections on states actually visited by a policy.

The policy acts in the real environment.  At each decision the Teacher is
queried on a clone of that exact state, and only the Teacher label is recorded;
the environment then continues with the policy's original plan.  This keeps
DAgger honest about covariate shift instead of replaying fresh initial states.
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


def _plan_agreement(left: Sequence[Any], right: Sequence[Any]) -> Dict[str, Any]:
    from lcb.plans import canonical

    matches = [canonical(a) == canonical(b) for a, b in zip(left, right)]
    first_difference = next((index for index, match in enumerate(matches) if not match), None)
    if first_difference is None and len(left) != len(right):
        first_difference = min(len(left), len(right))
    return {
        "exact_plan": len(left) == len(right) and all(matches),
        "actor_matches": sum(matches),
        "actor_decisions": max(len(left), len(right)),
        "actor_agreement": sum(matches) / max(len(left), len(right), 1),
        "first_disagreement_actor": first_difference,
    }


def _update_stats(stats, info: Dict[str, Any], before: Dict[str, Any], after: Dict[str, Any]) -> float:
    from lcb.rewards import compute_reward

    reward = compute_reward(info, before, after)
    stats.per_turn_reward.append(reward)
    stats.turns = int(info.get("turn") or stats.turns + 1)
    turn_stats = info.get("stats") or {}
    stats.damage_to_enemies += float(turn_stats.get("damage_to_enemies") or 0)
    stats.damage_to_allies += float(turn_stats.get("damage_to_allies") or 0)
    stats.sinking_damage += float(turn_stats.get("sinking_damage") or 0)
    stats.sinking_sp_damage += float(turn_stats.get("sinking_sp_damage") or 0)
    stats.sinking_triggers += int(turn_stats.get("sinking_triggers") or 0)
    stats.deaths += sum(
        1
        for unit_id in turn_stats.get("deaths") or []
        if any(u["id"] == unit_id and u.get("kind") == "sinner" for u in before.get("units", []))
    )
    for ego in turn_stats.get("ego_uses") or []:
        if ego not in stats.ego_uses:
            stats.ego_uses.append(ego)
    stats.skill_uses.append(list(turn_stats.get("skill_uses") or []))
    return reward


def collect_one(seed: int, cfg: Dict[str, Any]):
    from lcb.baselines import NeuralPolicy
    from lcb.env import LimbusEnv
    from lcb.features import Encoder
    from lcb.nn import PolicyValueNet
    from lcb.rewards import EpisodeStats
    from lcb.scenarios import scenario
    from lcb.teacher import (
        BeamTeacher,
        TeacherConfig,
        TeacherSample,
        detect_axis,
        detect_strategy_labels,
        encode_samples,
        imago,
        boss_sinking,
        teacher_budget,
    )
    from lcb.plans import canonical

    scene = scenario(cfg["scenario"])
    encoder = Encoder()
    budget = teacher_budget(cfg["teacher_budget"])
    teacher = BeamTeacher(
        encoder.table,
        encoder,
        TeacherConfig(
            **budget,
            max_turns=scene.max_turns,
            enemy_hp_scale=scene.enemy_hp_scale,
            infinite_ego_resources=scene.infinite_ego_resources,
        ),
    )
    net = PolicyValueNet.load(cfg["checkpoint"])
    policy = NeuralPolicy(net, encoder, sample=cfg["sample"], seed=seed, name="DAggerPolicy")
    env = LimbusEnv(strict=scene.strict)
    env.reset(
        seed,
        enemies=list(scene.enemies),
        max_turns=scene.max_turns,
        enemy_hp_scale=scene.enemy_hp_scale,
        infinite_ego_resources=scene.infinite_ego_resources,
    )
    first = env.observe()
    boss = imago(first)
    stats = EpisodeStats(seed=seed, boss_hp_start=float((boss or {}).get("hp") or 1.0))
    allies_hp = sum(float(u.get("hp") or 0) for u in first.get("units", []) if u.get("kind") == "sinner")
    teacher._boss_hp_start = stats.boss_hp_start
    teacher._allies_hp_start = allies_hp or 1.0
    samples: List[TeacherSample] = []
    replay: List[Dict[str, Any]] = []
    disagreement_count = 0
    exact_plan_count = 0
    actor_matches = 0
    actor_decisions = 0
    first_disagreement_turn = None
    first_disagreement_actor_counts: Dict[str, int] = {}

    while True:
        before = env.observe()
        if before.get("winner") or before.get("phase") == "Finished":
            break
        legal = env.legal_actions()
        teacher_env = env.clone_state()
        teacher_plan, value, groups, actors = teacher.plan_turn(teacher_env, before, legal)
        if not teacher_plan:
            break
        policy_plan = policy.plan(env, before, legal)
        agreement = _plan_agreement(teacher_plan, policy_plan)
        disagreement = not agreement["exact_plan"]
        disagreement_count += int(disagreement)
        exact_plan_count += int(agreement["exact_plan"])
        actor_matches += int(agreement["actor_matches"])
        actor_decisions += int(agreement["actor_decisions"])
        first_actor = agreement["first_disagreement_actor"]
        if first_actor is not None:
            key = str(first_actor)
            first_disagreement_actor_counts[key] = first_disagreement_actor_counts.get(key, 0) + 1
            if first_disagreement_turn is None:
                first_disagreement_turn = int(before.get("turn") or 0)
        samples.append(
            TeacherSample(
                turn=int(before.get("turn") or 0),
                obs=before,
                groups=groups,
                plan=teacher_plan,
                actors=actors,
                value=value,
                seed=seed,
            )
        )
        if not policy_plan:
            break
        info = env.step_turn(policy_plan)
        if not info.get("ok"):
            raise RuntimeError(f"DAgger policy plan rejected: {info.get('error')}")
        after = env.observe()
        reward = _update_stats(stats, info, before, after)
        replay.append(
            {
                "turn": info.get("turn"),
                "teacher_plan": [json.loads(canonical(a)) for a in teacher_plan],
                "policy_plan": [json.loads(canonical(a)) for a in policy_plan],
                "teacher_agrees": not disagreement,
                "teacher_actor_agreement": agreement["actor_agreement"],
                "teacher_first_disagreement_actor": first_actor,
                "boss_hp_before": float((imago(before) or {}).get("hp") or 0),
                "boss_hp_after": float((imago(after) or {}).get("hp") or 0),
                "survivors_after": sum(
                    1 for u in after.get("units", [])
                    if u.get("kind") == "sinner" and u.get("alive")
                ),
                "state_hash_before": info.get("state_hash_before"),
                "state_hash_after": info.get("state_hash_after"),
                "transition_hash": info.get("transition_hash"),
                "reward": reward,
                "boss_sinking": boss_sinking(before),
                "stats": info.get("stats") or {},
            }
        )
        if stats.turns >= scene.max_turns:
            break

    final = env.observe()
    stats.winner = final.get("winner")
    stats.won = final.get("winner") == "Sinners"
    stats.kill_turn = stats.turns if stats.won else None
    stats.boss_hp_left = float((imago(final) or {}).get("hp") or 0)
    stats.survivors = sum(
        1 for u in final.get("units", []) if u.get("kind") == "sinner" and u.get("alive")
    )
    axis = detect_axis(replay)
    axis.update(detect_strategy_labels(replay))
    stats.axis = axis
    for sample in samples:
        sample.episode_won = stats.won
        sample.kill_turn = stats.kill_turn
        sample.episode_axis = axis
    row = stats.to_row()
    row.update(
        {
            "teacher_decisions": len(samples),
            "teacher_disagreements": disagreement_count,
            "teacher_exact_plan_agreement": exact_plan_count / len(samples) if samples else 0.0,
            "teacher_action_agreement": actor_matches / actor_decisions if actor_decisions else 0.0,
            "teacher_agreement": actor_matches / actor_decisions if actor_decisions else 0.0,
            "first_disagreement_turn": first_disagreement_turn,
            "first_disagreement_actor_counts": first_disagreement_actor_counts,
            "search": dict(teacher.stats),
        }
    )
    return row, encode_samples(encoder, samples), replay


def _collect_worker(args: Tuple[int, Dict[str, Any]]):
    return collect_one(*args)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--checkpoint", required=True)
    parser.add_argument("--scenario", choices=["half", "real"], default="half")
    parser.add_argument("--seed-start", type=int, required=True)
    parser.add_argument("--seed-count", type=int, default=100)
    parser.add_argument("--teacher-budget", choices=["t0", "t1", "t2"], default="t1")
    parser.add_argument("--sample", action="store_true")
    parser.add_argument(
        "--sample-weight", type=float, default=1.0,
        help="relative weight of newly collected DAgger labels when merged with base data",
    )
    parser.add_argument("--jobs", type=int, default=1)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()

    from lcb.dataset import save_dataset
    from lcb.evaluate import best_of_n, provenance, summarise
    from lcb.scenarios import scenario

    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    seeds = list(range(args.seed_start, args.seed_start + args.seed_count))
    cfg = {
        "checkpoint": args.checkpoint,
        "scenario": args.scenario,
        "teacher_budget": args.teacher_budget,
        "sample": args.sample,
        "sample_weight": args.sample_weight,
    }
    rows: List[Dict[str, Any]] = []
    started = time.time()
    jobs = [(seed, cfg) for seed in seeds]
    if args.jobs > 1:
        with ProcessPoolExecutor(
            max_workers=args.jobs, mp_context=multiprocessing.get_context("spawn")
        ) as pool:
            results = pool.map(_collect_worker, jobs)
    else:
        results = (collect_one(seed, cfg) for seed in seeds)
    for seed, (row, arrays, replay) in zip(seeds, results):
        if args.sample_weight != 1.0 and arrays["weight"].size:
            arrays["weight"] = arrays["weight"] * args.sample_weight
        save_dataset(out / f"seed_{seed:05d}.npz", arrays)
        if row["teacher_disagreements"] or row["won"]:
            (out / "replays").mkdir(exist_ok=True)
            with (out / "replays" / f"dagger_seed_{seed:05d}.json").open("w", encoding="utf-8") as handle:
                json.dump({"seed": seed, "row": row, "turns": replay}, handle, ensure_ascii=False, indent=1)
        rows.append(row)
        print(
            f"seed {seed}: policy_won={row['won']} actor_agreement={row['teacher_action_agreement']:.3f} "
            f"exact_plan_agreement={row['teacher_exact_plan_agreement']:.3f}",
            flush=True,
        )

    scene = scenario(args.scenario)
    summary = summarise(rows)
    summary["teacher_action_agreement_mean"] = (
        sum(r["teacher_action_agreement"] for r in rows) / max(1, len(rows))
    )
    summary["teacher_exact_plan_agreement_mean"] = (
        sum(r["teacher_exact_plan_agreement"] for r in rows) / max(1, len(rows))
    )
    summary["teacher_disagreements"] = sum(r["teacher_disagreements"] for r in rows)
    index = {
        "provenance": provenance(
            scene,
            seeds,
            {
                "dagger": cfg,
                "data_role": "Teacher labels on states visited by checkpoint policy",
            },
        ),
        "elapsed_seconds": round(time.time() - started, 1),
        "summary": summary,
        "best_of_n": best_of_n(rows, (1, 10, 50)),
        "episodes": rows,
    }
    with (out / "dagger_index.json").open("w", encoding="utf-8") as handle:
        json.dump(index, handle, ensure_ascii=False, indent=2)
    print(json.dumps(summary, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
