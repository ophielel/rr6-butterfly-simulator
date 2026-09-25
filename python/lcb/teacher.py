"""Stage A: the multi-turn search teacher (TRAINING_PLAN.md §4).

A search node is a **complete turn**, not a character action:

```text
state(t) -> candidate full 7-unit plan -> commit + step_turn -> state(t+1) -> ...
```

The teacher keeps a beam over turn-level nodes and, for every turn, a beam over
actor-ordered plans.  The leaf value is an explicit hand-written function (§4.3),
ordered so that a win dominates, then a faster win, then a healthier team:

```text
value(state) = win - boss_hp - turn - ally_damage - deaths
             + generic combat reward (damage, survival, turn cost)
```

Every `TeacherSample` it emits can be replayed: it stores the plan, the state
hashes before/after and the transition hash the simulator computed.

RNG: the simulator's RNG state is part of the state hash, so a "reopen" is a new
initial seed for the same scenario (that is what best-of-N means in §7).
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple

import numpy as np

from .env import LimbusEnv, SECTION5_WAVE
from .features import Encoder, SkillTable
from .plans import PlanGenerator, action_actor, canonical, group_candidates, payload
from .rewards import EpisodeStats, compute_reward

# ---------------------------------------------------------------------------
# Leaf value (§4.3)
# ---------------------------------------------------------------------------


@dataclass
class ValueWeights:
    win: float = 1000.0
    boss_hp: float = 100.0
    turn: float = 0.0
    ally_damage: float = 30.0
    death: float = 60.0
    # Specific Sinking/E.G.O terms are disabled by default. They remain
    # available as explicit ablations, but formal emergence searches use only
    # generic combat outcomes and post-hoc labels.
    sinking_readiness: float = 0.0
    ego_value: float = 0.0


def imago(obs: Dict[str, Any]) -> Optional[Dict[str, Any]]:
    """The encounter's main enemy: the enemy with the largest max HP."""
    enemies = [u for u in obs.get("units", []) if u.get("kind") != "sinner"]
    if not enemies:
        return None
    return max(enemies, key=lambda unit: float(unit.get("max_hp") or 0))


def boss_sinking(obs: Dict[str, Any]) -> Dict[str, float]:
    """The boss's [Sinking] Potency/Count at this moment (real state, not text)."""
    boss = imago(obs)
    sinking = ((boss or {}).get("statuses") or {}).get("Sinking") or {}
    return {
        "potency": float(sinking.get("potency") or 0),
        "count": float(sinking.get("count") or 0),
    }


def sinking_readiness(obs: Dict[str, Any]) -> float:
    """How close the [Sinking] on the boss is to being worth cashing in.

    `0.06 * Potency + 0.12 * Count`, capped at 1: the plan's first-version
    threshold (Potency +5 or Count +3) is roughly a third of the way up.
    """
    if imago(obs) is None:
        return 0.0
    sinking = boss_sinking(obs)
    return min(1.0, 0.06 * sinking["potency"] + 0.12 * sinking["count"])


def ego_readiness(obs: Dict[str, Any], table: SkillTable) -> float:
    """Fraction of the team's E.G.O whose resource cost is already covered."""
    resources = obs.get("ego_resources") or {}
    available = 0
    total = 0
    for unit in obs.get("units", []):
        if unit.get("kind") != "sinner":
            continue
        for ego_id in unit.get("ego_slots") or []:
            total += 1
            if table.ego_affordable(ego_id, resources):
                available += 1
    return available / total if total else 0.0


def state_value(
    obs: Dict[str, Any],
    table: SkillTable,
    weights: ValueWeights,
    boss_hp_start: float,
    allies_hp_start: float,
    turn_start: int,
) -> float:
    winner = obs.get("winner")
    if winner == "Sinners":
        # A faster win is worth more: the tunable part still applies, but the
        # win term dominates by two orders of magnitude (§4.3, lexicographic).
        elapsed = max(0, int(obs.get("turn") or 0) - turn_start)
        return weights.win - elapsed - (weights.boss_hp * 0.0)
    if winner in ("Enemies", "Draw"):
        return -weights.win

    boss = imago(obs)
    boss_hp = float(boss.get("hp") or 0) if boss else 0.0
    value = -weights.boss_hp * (boss_hp / boss_hp_start if boss_hp_start else 0.0)
    value -= weights.turn * max(0, int(obs.get("turn") or 0) - turn_start)

    sinners = [u for u in obs.get("units", []) if u.get("kind") == "sinner"]
    allies_hp = sum(float(u.get("hp") or 0) for u in sinners)
    value -= weights.ally_damage * (1.0 - allies_hp / allies_hp_start if allies_hp_start else 0.0)
    dead = sum(1 for u in sinners if not u.get("alive"))
    value -= weights.death * dead
    value += weights.sinking_readiness * sinking_readiness(obs)
    value += weights.ego_value * ego_readiness(obs, table)
    return value


# ---------------------------------------------------------------------------
# Teacher
# ---------------------------------------------------------------------------


@dataclass
class TeacherConfig:
    """Search budget (the plan's baseline is `plan_width=32`, `horizon=3`)."""

    horizon: int = 3
    plan_width: int = 32
    candidate_cap: int = 6
    turn_width: int = 4
    score_mode: str = "heuristic"  # "heuristic" | "rollout"
    rollout_width: int = 4
    transposition: bool = True
    max_turns: int = 12
    enemies: Tuple[str, ...] = tuple(SECTION5_WAVE)
    #: **Scenario knob** handed to `BattleConfig::enemy_hp_scale` (1.0 == the data).
    enemy_hp_scale: float = 1.0
    infinite_ego_resources: bool = False


@dataclass
class TeacherSample:
    """One (state, candidates, chosen plan) decision the teacher made."""

    turn: int
    obs: Dict[str, Any]
    groups: Dict[str, List[Any]]
    plan: List[Any]
    actors: List[str]
    value: float
    seed: int = 0
    #: filled in when the episode is over
    episode_won: bool = False
    kill_turn: Optional[int] = None
    episode_axis: Optional[Dict[str, Any]] = None

    def labels(self) -> List[int]:
        """Index of the teacher's action inside each actor's candidate list."""
        out: List[int] = []
        for actor, action in zip(self.actors, self.plan):
            options = self.groups.get(actor, [])
            key = canonical(action)
            out.append(next((i for i, o in enumerate(options) if canonical(o) == key), -1))
        return out


class BeamTeacher:
    """Turn-level beam search with a transposition table."""

    def __init__(
        self,
        table: SkillTable,
        encoder: Encoder,
        config: Optional[TeacherConfig] = None,
        weights: Optional[ValueWeights] = None,
    ) -> None:
        self.table = table
        self.encoder = encoder
        self.config = config or TeacherConfig()
        self.weights = weights or ValueWeights()
        #: `search_key|plan` -> (the state after resolving it, the turn reward).
        #: The key contains the RNG continuation, because the RNG state is part
        #: of `search_key` (§4.4).
        self.transposition: Dict[str, Tuple[LimbusEnv, float]] = {}
        self.stats = {"nodes": 0, "transposition_hits": 0, "plans": 0}

    # -- turn planning -----------------------------------------------------
    def _rollout(self, env: LimbusEnv, partial: List[Any]) -> float:
        """Score a partial plan by filling it and resolving the turn for real."""
        probe = env.clone_state()
        obs_before = probe.observe()
        info = probe.step_turn(partial)
        if not info.get("ok"):
            return float("-inf")
        after = probe.observe()
        self.stats["nodes"] += 1
        return state_value(
            after,
            self.table,
            self.weights,
            getattr(self, "_boss_hp_start", 1.0),
            getattr(self, "_allies_hp_start", 1.0),
            obs_before.get("turn") or 0,
        ) + compute_reward(info, obs_before, after)

    def plan_turn(
        self, env: LimbusEnv, obs: Dict[str, Any], legal: Sequence[Any]
    ) -> Tuple[List[Any], float, Dict[str, List[Any]], List[str]]:
        """The plan to play now, chosen over a **turn-level tree** of `horizon`
        turns (§4.1/§4.2).

        A node is a complete turn: a candidate plan is generated for it (beam over
        the actor-ordered partial plans), resolved in a clone, and the resulting
        state becomes a child node.  Each level keeps `turn_width` nodes, so the
        cost is `horizon * turn_width^2` turn resolutions per decision - and it is
        what lets the teacher plan a `Sinking -> E.G.O -> trigger` line instead of
        a single greedy turn.

        The returned plan is the first turn of the best line; the candidate
        groups and actor order always describe the **current** state, which is
        what the BC labels are built from.
        """
        from .plans import actor_order

        actors = actor_order(obs, legal)
        groups = group_candidates(legal)
        rollout = self._rollout if self.config.score_mode == "rollout" else None
        generator = PlanGenerator(
            self.table,
            width=self.config.rollout_width if rollout else self.config.plan_width,
            cap=self.config.candidate_cap,
            rollout=rollout,
        )
        turn_start = int(obs.get("turn") or 0)
        nodes: List[Tuple[float, LimbusEnv, List[List[Any]]]] = [(0.0, env, [])]
        for _depth in range(max(1, self.config.horizon)):
            children: List[Tuple[float, LimbusEnv, List[List[Any]]]] = []
            for cum, node_env, path in nodes:
                node_obs = node_env.observe()
                if node_obs.get("winner") or node_obs.get("phase") == "Finished":
                    children.append((cum, node_env, path))
                    continue
                node_legal = node_env.legal_actions()
                plans = generator.generate(node_env, node_obs, node_legal)[
                    : self.config.turn_width
                ]
                for _prior, plan in plans:
                    key = self._plan_key(node_env, plan)
                    cached = self.transposition.get(key) if self.config.transposition else None
                    if cached is not None:
                        self.stats["transposition_hits"] += 1
                        child, reward = cached
                    else:
                        child = node_env.clone_state()
                        info = child.step_turn(plan)
                        self.stats["plans"] += 1
                        if not info.get("ok"):
                            continue
                        reward = compute_reward(info, node_obs, child.observe())
                        if self.config.transposition:
                            # Bounded table: it holds cloned states, so it is
                            # cleared rather than allowed to grow without limit.
                            if len(self.transposition) >= 20_000:
                                self.transposition.clear()
                            self.transposition[key] = (child, reward)
                    children.append((cum + reward, child, path + [plan]))
            if not children:
                break
            children.sort(key=lambda item: item[0], reverse=True)
            nodes = children[: max(1, self.config.turn_width)]
        # Lexicographic leaf ordering: a win first, then the faster win, then the
        # accumulated turn value (§4.3).
        def rank(node: Tuple[float, LimbusEnv, List[List[Any]]]) -> Tuple[int, int, float]:
            value, node_env, _path = node
            final = node_env.observe()
            won = final.get("winner") == "Sinners"
            elapsed = int(final.get("turn") or 0) - turn_start
            return (1 if won else 0, -elapsed if won else 0, value)

        best = max(nodes, key=rank)
        plan = best[2][0] if best[2] else []
        return plan, best[0], groups, actors

    def _plan_key(self, env: LimbusEnv, plan: Sequence[Any]) -> str:
        return env.search_key() + "|" + ";".join(canonical(a) for a in plan)

    # -- episodes ----------------------------------------------------------
    def run_episode(
        self,
        seed: int,
        strict: bool = True,
        collect: bool = True,
        enemies: Optional[Sequence[str]] = None,
        enemy_hp_scale: Optional[float] = None,
        max_turns: Optional[int] = None,
    ) -> Tuple[EpisodeStats, List[TeacherSample], List[Dict[str, Any]]]:
        env = LimbusEnv(strict=strict)
        env.reset(
            seed,
            enemies=list(enemies or self.config.enemies),
            max_turns=max_turns or self.config.max_turns,
            enemy_hp_scale=(
                enemy_hp_scale if enemy_hp_scale is not None else self.config.enemy_hp_scale
            ),
            infinite_ego_resources=self.config.infinite_ego_resources,
        )
        obs = env.observe()
        self._boss_hp_start = 0.0
        boss = imago(obs)
        self._boss_hp_start = float(boss.get("hp") or 1) if boss else 1.0
        self._allies_hp_start = sum(
            float(u.get("hp") or 0) for u in obs.get("units", []) if u.get("kind") == "sinner"
        ) or 1.0

        stats = EpisodeStats(seed=seed, boss_hp_start=self._boss_hp_start)
        samples: List[TeacherSample] = []
        replay: List[Dict[str, Any]] = []
        while True:
            obs = env.observe()
            if obs.get("winner") or obs.get("phase") == "Finished":
                break
            legal = env.legal_actions()
            plan, value, groups, actors = self.plan_turn(env, obs, legal)
            if not plan:
                break
            info = env.step_turn(plan)
            if not info.get("ok"):
                raise RuntimeError(f"teacher plan rejected: {info.get('error')}")
            after = env.observe()
            reward = compute_reward(info, obs, after)
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
                if any(u["id"] == unit_id and u.get("kind") == "sinner" for u in obs["units"])
            )
            for ego in turn_stats.get("ego_uses") or []:
                if ego not in stats.ego_uses:
                    stats.ego_uses.append(ego)
            stats.skill_uses.append(list(turn_stats.get("skill_uses") or []))
            replay.append(
                {
                    "turn": info.get("turn"),
                    "plan": [json.loads(canonical(a)) for a in plan],
                    "state_hash_before": info.get("state_hash_before"),
                    "state_hash_after": info.get("state_hash_after"),
                    "transition_hash": info.get("transition_hash"),
                    "reward": reward,
                    "stats": turn_stats,
                    "boss_sinking": boss_sinking(obs),
                }
            )
            if collect:
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
            if stats.turns >= (max_turns or self.config.max_turns):
                break

        final = env.observe()
        stats.winner = final.get("winner")
        stats.won = final.get("winner") == "Sinners"
        stats.kill_turn = stats.turns if stats.won else None
        stats.boss_hp_left = float((imago(final) or {}).get("hp") or 0)
        stats.survivors = sum(
            1
            for u in final.get("units", [])
            if u.get("kind") == "sinner" and u.get("alive")
        )
        axis = detect_axis(replay, samples, table=self.table)
        axis.update(detect_strategy_labels(replay))
        stats.axis = axis
        for sample in samples:
            sample.episode_won = stats.won
            sample.kill_turn = stats.kill_turn
            sample.episode_axis = axis
        return stats, samples, replay


# ---------------------------------------------------------------------------
# Axis detection (§7.1 item 5)
# ---------------------------------------------------------------------------

#: "Harmony" (Sinclair) and "Solemn Lament" (Yi Sang) - the two E.G.O of the
#: documented Sinking burst.
AXIS_EGO = {"21009": "Harmony", "20109": "Solemn Lament"}


def detect_strategy_labels(replay: Sequence[Dict[str, Any]]) -> Dict[str, Any]:
    """Post-hoc labels for analysis; these never enter policy features."""
    peak_potency = 0
    peak_count = 0
    first_setup: Optional[int] = None
    first_burst: Optional[int] = None
    sinking_by_turn: Dict[int, float] = {}
    ego_by_turn: Dict[int, set[str]] = {}
    for entry in replay:
        turn = int(entry.get("turn") or 0)
        sinking = entry.get("boss_sinking") or {}
        peak_potency = max(peak_potency, int(sinking.get("potency") or 0))
        peak_count = max(peak_count, int(sinking.get("count") or 0))
        sinking_by_turn[turn] = float((entry.get("stats") or {}).get("sinking_damage") or 0)
        ids = {str(value).split("|")[-1] for value in (entry.get("stats") or {}).get("ego_uses") or []}
        ego_by_turn[turn] = ids
        if {"20807", "20903"}.issubset(ids) and first_setup is None:
            first_setup = turn
        if ids.intersection(set(AXIS_EGO) | {"20106", "20807", "20903"}) and first_burst is None:
            first_burst = turn
    known_setup = False
    if first_setup is not None:
        known_setup = any(
            "20106" in ego_by_turn.get(turn, set())
            for turn in range(first_setup + 1, first_setup + 3)
        )
    pre_count = max(
        (int((entry.get("boss_sinking") or {}).get("count") or 0)
         for entry in replay if first_burst is None or int(entry.get("turn") or 0) < first_burst),
        default=0,
    )
    pre_values = [value for turn, value in sinking_by_turn.items() if first_burst is None or turn < first_burst]
    post_values = [value for turn, value in sinking_by_turn.items() if first_burst is None or turn >= first_burst]
    pre_median = float(np.median(pre_values)) if pre_values else 0.0
    direct_burst = bool(post_values and max(post_values) > max(1.0, 2.0 * pre_median))
    return {
        "known_setup_like": known_setup,
        "direct_burst": direct_burst,
        "peak_sinking_potency": peak_potency,
        "peak_sinking_count": peak_count,
        "pre_burst_count": pre_count,
        "setup_turn": first_setup,
        "burst_turn": first_burst,
    }


def detect_axis(
    replay: Sequence[Dict[str, Any]],
    samples: Sequence[TeacherSample] = (),
    table: Optional[SkillTable] = None,
) -> Dict[str, Any]:
    """Did this win use the real `Sinking -> Harmony / Solemn Lament -> trigger` axis?"""
    first_ego_turn: Dict[str, int] = {}
    trigger_damage: Dict[int, float] = {}
    sinking_before: Dict[int, Dict[str, float]] = {}
    for entry in replay:
        turn = int(entry.get("turn") or 0)
        stats = entry.get("stats") or {}
        if entry.get("boss_sinking") is not None:
            sinking_before[turn] = entry["boss_sinking"]
        for ego in stats.get("ego_uses") or []:
            ego_id = ego.split("|")[-1]
            if ego_id in AXIS_EGO and ego_id not in first_ego_turn:
                first_ego_turn[ego_id] = turn
        trigger_damage[turn] = float(stats.get("sinking_damage") or 0)
    # Sinking established before the first E.G.O of the axis (first-version
    # threshold: Potency +5 or Count +3).
    setup_ok = False
    if first_ego_turn:
        earliest = min(first_ego_turn.values())
        for turn, sinking in sinking_before.items():
            if turn < earliest and (sinking.get("potency", 0) >= 5 or sinking.get("count", 0) >= 3):
                setup_ok = True
    pair_ok = False
    if len(first_ego_turn) >= 2:
        turns = sorted(first_ego_turn.values())
        pair_ok = turns[-1] - turns[0] <= 2  # a 3-turn window
    post_trigger = 0.0
    pre_trigger = 0.0
    if first_ego_turn:
        first = min(first_ego_turn.values())
        post_trigger = sum(v for t, v in trigger_damage.items() if first <= t <= first + 2)
        pre_trigger = sum(v for t, v in trigger_damage.items() if t < first)
    return {
        "first_ego_turn": first_ego_turn,
        "setup_ok": setup_ok,
        "pair_ok": pair_ok,
        "post_trigger_damage": post_trigger,
        "pre_trigger_damage": pre_trigger,
        "burst_ok": post_trigger > pre_trigger,
        "axis_ok": bool(setup_ok and (pair_ok or post_trigger > 0) and post_trigger > pre_trigger),
    }


# ---------------------------------------------------------------------------
# Sample serialisation for BC
# ---------------------------------------------------------------------------


def encode_samples(
    encoder: Encoder, samples: Iterable[TeacherSample], weight_by_speed: bool = True
) -> Dict[str, np.ndarray]:
    """Turn teacher decisions into arrays the policy can be trained on.

    Layout (one row per actor decision):

    * `state`: the encoded observation of the turn the decision belongs to,
    * `cand`: candidate action features, concatenated (`cand_offsets` splits it),
    * `label`: index of the teacher's action inside its candidate list,
    * `weight`: sample weight (short wins and burst windows count more, §5),
    * `decision`: which turn-decision the row belongs to (one row per actor),
    * `seed`: the episode's seed, so training/validation splits are by seed.
    """
    states: List[np.ndarray] = []
    cands: List[np.ndarray] = []
    offsets: List[int] = []
    labels: List[int] = []
    weights: List[float] = []
    actors: List[int] = []
    seeds: List[int] = []
    decisions: List[int] = []
    decision_index = 0
    for sample in samples:
        state_vec = encoder.encode_state(sample.obs)
        labels_for_turn = sample.labels()
        index = encoder.unit_slots(sample.obs)[2]
        chosen: List[Any] = []
        for position, actor in enumerate(sample.actors):
            options = sample.groups.get(actor, [])
            if not options or labels_for_turn[position] < 0:
                continue
            matrix = encoder.action_matrix(sample.obs, options, index, chosen)
            states.append(state_vec)
            cands.append(matrix)
            offsets.append(len(options))
            labels.append(labels_for_turn[position])
            weight = 1.0
            if weight_by_speed and sample.episode_won:
                kill = sample.kill_turn or 12
                weight += max(0.0, (12 - kill) / 12.0) * 2.0
            if sample.episode_axis and sample.episode_axis.get("axis_ok"):
                weight += 0.5
            weights.append(weight)
            actors.append(position)
            seeds.append(int(sample.seed))
            decisions.append(decision_index)
            chosen.append(sample.plan[position])
        decision_index += 1
    if not states:
        return {
            "state": np.zeros((0, encoder.state_dim), dtype=np.float32),
            "cand": np.zeros((0, encoder.action_dim), dtype=np.float32),
            "offsets": np.zeros((0,), dtype=np.int32),
            "label": np.zeros((0,), dtype=np.int32),
            "weight": np.zeros((0,), dtype=np.float32),
            "actor": np.zeros((0,), dtype=np.int32),
            "seed": np.zeros((0,), dtype=np.int32),
            "decision": np.zeros((0,), dtype=np.int32),
        }
    return {
        "state": np.stack(states).astype(np.float32),
        "cand": np.concatenate(cands).astype(np.float32),
        "offsets": np.asarray(offsets, dtype=np.int32),
        "label": np.asarray(labels, dtype=np.int32),
        "weight": np.asarray(weights, dtype=np.float32),
        "actor": np.asarray(actors, dtype=np.int32),
        "seed": np.asarray(seeds, dtype=np.int32),
        "decision": np.asarray(decisions, dtype=np.int32),
    }


__all__ = [
    "BeamTeacher",
    "boss_sinking",
    "TeacherConfig",
    "TeacherSample",
    "ValueWeights",
    "state_value",
    "sinking_readiness",
    "ego_readiness",
    "detect_axis",
    "detect_strategy_labels",
    "encode_samples",
    "imago",
    "AXIS_EGO",
]
