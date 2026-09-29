"""Tests for the training layer (run directly: `python python/tests/test_training.py`).

They are fast on purpose: no long training runs, no evaluation sweep.  What is
checked is what the plan's §8 correctness gate asks for - the atomic turn
interface, the mask, replayability, the dataset contract and the metric math.
"""

from __future__ import annotations

import json
import os
import shutil
import sys
import tempfile
from contextlib import contextmanager
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "python"))

from lcb import dataset as ds  # noqa: E402
from lcb.baselines import FirstLegalPolicy, GreedyPolicy, NeuralPolicy, RandomPolicy  # noqa: E402
from lcb.env import LimbusEnv, SECTION5_WAVE  # noqa: E402
from lcb.evaluate import Scenario, best_of_n, restart_aware, run_episode, summarise, wilson_interval  # noqa: E402
from lcb.features import Encoder, SkillTable, action_effect_fingerprint  # noqa: E402
from lcb.nn import PolicyValueNet  # noqa: E402
import lcb.ppo as ppo_module  # noqa: E402
from lcb.rewards import (  # noqa: E402
    DAMAGE_REWARD,
    FAST_KILL_BONUS,
    LATE_KILL_PENALTY_PER_TURN,
    TURN_PENALTY,
    compute_reward,
    terminal_reward,
)
from lcb.stdio_client import StdioSimulator  # noqa: E402
from lcb.plans import (  # noqa: E402
    PlanGenerator,
    actor_order,
    canonical,
    estimate_action,
    first_legal_plan,
    group_candidates,
)
from lcb.teacher import BeamTeacher, TeacherConfig, detect_axis, detect_strategy_labels, encode_samples, teacher_budget  # noqa: E402

SCENARIO = Scenario(name="test", max_turns=4, enemy_hp_scale=0.08)

# Two tests write a throwaway checkpoint.  Keep that under the repository so the
# suite also runs where the platform temp directory is not writable.
_SCRATCH_ROOT = ROOT / ".tmp"


@contextmanager
def _scratch_dir():
    """A fresh directory under `.tmp/`, removed on exit.

    `tempfile.mkdtemp` creates the directory with a restrictive mode, which some
    containerised file sandboxes reject on the very next write; a plain `mkdir`
    under the repository keeps the parent's permissions.
    """
    _SCRATCH_ROOT.mkdir(parents=True, exist_ok=True)
    directory = _SCRATCH_ROOT / f"scratch_{os.urandom(4).hex()}"
    directory.mkdir()
    try:
        yield str(directory)
    finally:
        try:
            shutil.rmtree(directory)
        except OSError:  # pragma: no cover - best effort cleanup
            pass


def test_action_effect_fingerprint_reports_generic_state_deltas() -> None:
    before = {
        "units": [
            {
                "id": "boss",
                "kind": "enemy",
                "max_hp": 1000,
                "hp": 1000,
                "alive": True,
                "statuses": {},
                "staggered": False,
            },
            {
                "id": "sinner",
                "kind": "sinner",
                "max_hp": 1000,
                "hp": 1000,
                "alive": True,
                "statuses": {},
                "staggered": False,
            },
        ],
        "ego_resources": {"Wrath": 1},
        "resonance": {"Wrath": 1},
    }
    after = {
        "units": [
            {
                "id": "boss",
                "kind": "enemy",
                "max_hp": 1000,
                "hp": 900,
                "alive": True,
                "statuses": {"Generic": {"potency": 2, "count": 3}},
                "staggered": True,
            },
            {
                "id": "sinner",
                "kind": "sinner",
                "max_hp": 1000,
                "hp": 950,
                "alive": True,
                "statuses": {},
                "staggered": False,
            },
        ],
        "ego_resources": {"Wrath": 2},
        "resonance": {"Wrath": 2},
    }
    effect = action_effect_fingerprint(
        before,
        after,
        {"stats": {"damage_to_enemies": 100, "damage_to_allies": 50}},
        target_id="boss",
    )
    assert effect.shape == (22,)
    assert effect[0] == 1.0
    assert effect[2] > 0.0
    assert effect[9] > 0.0
    assert effect[18] > 0.0


def test_reward_ignores_sinking_trigger_damage() -> None:
    before = {
        "units": [
            {"id": "boss", "kind": "enemy", "max_hp": 1000, "hp": 1000},
        ]
    }
    after = {
        "units": [
            {"id": "boss", "kind": "enemy", "max_hp": 1000, "hp": 1000},
        ]
    }
    reward = compute_reward(
        {"stats": {"damage_to_enemies": 0, "sinking_damage": 100}}, before, after
    )
    assert reward == TURN_PENALTY == -30.0


def test_opt_in_sinking_trigger_reward_only_credits_realized_trigger_damage() -> None:
    before = {
        "units": [
            {"id": "boss", "kind": "enemy", "max_hp": 1000, "hp": 1000},
        ]
    }
    after = {
        "units": [
            {"id": "boss", "kind": "enemy", "max_hp": 1000, "hp": 1000},
        ]
    }
    reward = compute_reward(
        {"stats": {"sinking_damage": 100}},
        before,
        after,
        sinking_trigger_reward=0.001,
    )
    assert abs(reward - (TURN_PENALTY + 0.1)) < 1e-9


def test_generic_kill_speed_reward_has_six_turn_bonus_and_seven_turn_neutral() -> None:
    before = {
        "units": [
            {"id": "boss", "kind": "enemy", "max_hp": 1000, "hp": 1000},
        ]
    }

    def reward_at(turn: int) -> float:
        after = {
            "turn": turn,
            "winner": "Sinners",
            "units": [
                {"id": "boss", "kind": "enemy", "max_hp": 1000, "hp": 0},
            ],
        }
        return compute_reward({"stats": {}}, before, after)

    six = reward_at(6)
    seven = reward_at(7)
    eight = reward_at(8)
    assert six - seven == FAST_KILL_BONUS
    assert eight - seven == LATE_KILL_PENALTY_PER_TURN
    assert six == TURN_PENALTY + DAMAGE_REWARD * 1000 + 1000.0 + FAST_KILL_BONUS
    assert terminal_reward({"winner": "Sinners", "turn": 6}) == 1400.0
    assert terminal_reward({"winner": "Sinners", "turn": 7}) == 1000.0
    assert terminal_reward({"winner": "Sinners", "turn": 8}) == 800.0


def test_reward_credits_main_boss_hp_not_butterfly_damage() -> None:
    before = {
        "units": [
            {"id": "boss", "kind": "enemy", "max_hp": 1000, "hp": 1000},
            {"id": "butterfly", "kind": "enemy", "max_hp": 1, "hp": 1},
        ]
    }
    after = {
        "units": [
            {"id": "boss", "kind": "enemy", "max_hp": 1000, "hp": 900},
            {"id": "butterfly", "kind": "enemy", "max_hp": 1, "hp": 0},
        ]
    }
    reward = compute_reward(
        {"stats": {"damage_to_enemies": 101, "sinking_damage": 0}}, before, after
    )
    assert reward == TURN_PENALTY + 1.0


def test_registered_budgets_and_wilson_interval() -> None:
    assert teacher_budget("t1") == {
        "horizon": 4,
        "plan_width": 32,
        "candidate_cap": 6,
        "turn_width": 4,
        "rollout_width": 4,
    }
    low, high = wilson_interval(0, 100)
    assert low == 0.0 and high > 0.0
    low, high = wilson_interval(100, 100)
    assert low < 1.0 and high > 0.999


def test_ppo_rollout_and_validation_propagate_infinite_ego_resources() -> None:
    calls = []

    class TerminalEnv:
        def __init__(self, strict=False):
            self.strict = strict

        def reset(self, seed, **kwargs):
            calls.append((seed, kwargs))

        def observe(self):
            return {"winner": "Sinners", "phase": "Finished", "units": []}

    original_env = ppo_module.LimbusEnv
    ppo_module.LimbusEnv = TerminalEnv
    try:
        scene = Scenario(name="ppo-infinite", infinite_ego_resources=True)
        encoder = Encoder()
        net = PolicyValueNet(state_dim=encoder.state_dim, action_dim=encoder.action_dim, hidden=4)
        ppo_module.collect_episode(net, encoder, scene, 17, np.random.default_rng(17))
        ppo_module.evaluate_argmax(net, encoder, scene, [18], episodes=1)
    finally:
        ppo_module.LimbusEnv = original_env

    assert len(calls) == 2
    assert all(kwargs["infinite_ego_resources"] is True for _, kwargs in calls)


def test_legacy_dataset_candidates_can_be_padded_for_effect_features() -> None:
    legacy = {"cand": np.ones((2, 44), dtype=np.float32)}
    expanded = ds.adapt_action_dim(legacy, 66)
    assert expanded["cand"].shape == (2, 66)
    assert np.allclose(expanded["cand"][:, :44], 1.0)
    assert np.allclose(expanded["cand"][:, 44:], 0.0)
    with np.testing.assert_raises(ValueError):
        ds.adapt_action_dim({"cand": np.ones((1, 66), dtype=np.float32)}, 44)


def test_loaded_checkpoint_can_be_padded_for_effect_features() -> None:
    base = PolicyValueNet(state_dim=3, action_dim=2, hidden=4, seed=73)
    with _scratch_dir() as directory:
        path = Path(directory) / "base.npz"
        base.save(path)
        expanded = PolicyValueNet.load(path, action_dim=5)
    assert expanded.action_dim == 5
    assert expanded.params["W2"].shape == (4 + 5, 4)
    assert np.allclose(expanded.params["W2"][:6], base.params["W2"])
    assert np.allclose(expanded.params["W2"][6:], 0.0)


def test_ppo_accepts_actor_level_credit_multiplier() -> None:
    net = PolicyValueNet(state_dim=3, action_dim=2, hidden=5, seed=72, lr=1e-4)
    state = np.asarray([0.2, -0.4, 0.7], dtype=np.float64)
    candidates = np.asarray([[1.0, 0.0], [0.0, 1.0]], dtype=np.float64)
    action = 1
    old_logprob = float(np.log(net.probs(state, candidates)[action]))
    result = net.ppo_update(
        [(state, [(candidates, action, old_logprob, 1.25)], 0.8)], clip=0.2
    )
    assert np.isfinite(result["loss"])
    assert result["count"] == 1.0


def test_critic_is_trained_before_the_advantages_are_computed() -> None:
    """The advantage must be `return - V(s)` with a *fitted* V, not the return.

    BC never trains the value head, so at the first PPO iteration the head is
    still the random initialisation.  Without the warm-up every actor of a won
    turn would be pushed by the same huge positive scalar; with the warm-up the
    head has already moved towards the observed return.
    """
    encoder = Encoder()
    net = PolicyValueNet(
        state_dim=encoder.state_dim, action_dim=encoder.action_dim, hidden=4, seed=5
    )
    state = np.zeros(encoder.state_dim, dtype=np.float32)
    before = net.value(state)
    net.value_update([(state, 1.5)], value_coef=1.0)
    for _ in range(200):
        net.value_update([(state, 1.5)], value_coef=1.0)
        net.value_update([(state, 1.5)], value_coef=1.5)
    after = net.value(state)
    assert abs(after - before) > 1e-4
    assert net.value(state) > 0.0


def test_value_warmup_epochs_shrink_the_raw_advantage() -> None:
    """`value_warmup_epochs` has to be what makes the advantage non-trivial."""
    states = [np.full(6, 0.1 * index, dtype=np.float32) for index in range(6)]
    returns = [1.0, 0.8, 0.6, 0.4, 0.2, 0.0]

    def advantages(warmup: int) -> list:
        net = PolicyValueNet(state_dim=6, action_dim=2, hidden=4, seed=11, lr=5e-3)
        values = list(zip(states, returns))
        for _ in range(warmup):
            net.value_update(values, value_coef=0.5)
        return [ret - net.value(state) for state, ret in zip(states, returns)]

    # No warm-up: the head is still the near-random initialisation, so the
    # advantage is essentially the return itself.
    cold = advantages(0)
    assert max(abs(value) for value in cold) > 0.9
    # The warm-up moves the head towards the observed return, which shrinks the
    # magnitude of the learning signal and its spread across the same batch.
    warm = advantages(200)
    assert float(np.mean(np.abs(warm))) < 0.5 * float(np.mean(np.abs(cold)))


def test_value_update_clips_the_target_against_the_baseline() -> None:
    net = PolicyValueNet(state_dim=3, action_dim=2, hidden=4, seed=13, lr=1e-2)
    state = np.asarray([0.3, -0.2, 0.5], dtype=np.float64)
    baseline = net.value(state)
    clipped_loss = net.value_update([(state, baseline + 10.0, baseline)], clip=0.1)
    assert np.isfinite(clipped_loss)
    assert clipped_loss > 0.0


def test_entropy_bonus_increases_the_candidate_entropy() -> None:
    state = np.asarray([0.2, -0.4, 0.7], dtype=np.float64)
    candidates = np.asarray([[1.0, 0.0], [0.0, 1.0], [0.5, -0.3]], dtype=np.float64)

    def build() -> PolicyValueNet:
        net = PolicyValueNet(state_dim=3, action_dim=2, hidden=5, seed=17, lr=1e-2)
        net.params["W3"] = net.params["W3"] * 4.0  # a peaked distribution
        return net

    def measure(net: PolicyValueNet) -> float:
        probs = net.probs(state, candidates)
        return float(-np.sum(probs * np.log(np.maximum(probs, 1e-12))))

    plain = build()
    started = measure(plain)
    for _ in range(12):
        broken = float(np.log(plain.probs(state, candidates)[0]))
        plain.ppo_update(
            [(state, [(candidates, 0, broken)], 0.0)], clip=0.2, entropy_coef=0.0
        )
    assert abs(measure(plain) - started) < 1e-9  # zero advantage, no bonus: nothing moves

    net = build()
    for _ in range(12):
        old_logprob = float(np.log(net.probs(state, candidates)[0]))
        stats = net.ppo_update(
            [(state, [(candidates, 0, old_logprob)], 0.0)], clip=0.2, entropy_coef=0.5
        )
    assert stats["entropy"] > 0.0
    assert measure(net) > started


def test_ppo_demo_updates_decay_by_iteration() -> None:
    calls = []
    original_collect = ppo_module.collect_episode
    original_bc_update = PolicyValueNet.bc_update

    def fake_collect(*args, **kwargs):
        return [], {"won": False, "return": 0.0, "counterfactual_calls": 0}

    def fake_bc_update(self, batch):
        calls.append(len(batch))
        return 0.0

    ppo_module.collect_episode = fake_collect
    PolicyValueNet.bc_update = fake_bc_update
    try:
        encoder = Encoder()
        net = PolicyValueNet(state_dim=encoder.state_dim, action_dim=encoder.action_dim, hidden=4)
        train_net, history, _info = ppo_module.train_ppo(
            net,
            encoder,
            SCENARIO,
            [1, 2, 3],
            ppo_module.PPOConfig(
                iterations=4,
                episodes_per_iteration=1,
                demo_updates_per_iteration=4,
                demo_decay=0.5,
                demo_min_updates=0,
            ),
            demonstrations=[(np.zeros(1), [])],
        )
    finally:
        ppo_module.collect_episode = original_collect
        PolicyValueNet.bc_update = original_bc_update
    assert train_net is net
    assert history.demo_updates == [4, 2, 1, 0]
    assert len(calls) == 7


def test_ppo_policy_gradient_matches_finite_difference_direction() -> None:
    net = PolicyValueNet(state_dim=3, action_dim=2, hidden=5, seed=71, lr=1e-4)
    state = np.asarray([0.2, -0.4, 0.7], dtype=np.float64)
    candidates = np.asarray([[1.0, 0.0], [0.0, 1.0], [0.5, -0.3]], dtype=np.float64)
    action = 1
    old_logprob = float(np.log(net.probs(state, candidates)[action]))
    advantage = 0.8
    parameter = net.params["W3"]
    index = 0
    original = float(parameter[index])

    def surrogate() -> float:
        logprob = float(np.log(net.probs(state, candidates)[action]))
        ratio = np.exp(logprob - old_logprob)
        return -min(ratio * advantage, np.clip(ratio, 0.8, 1.2) * advantage)

    epsilon = 1e-6
    parameter[index] = original + epsilon
    plus = surrogate()
    parameter[index] = original - epsilon
    minus = surrogate()
    parameter[index] = original
    finite_difference = (plus - minus) / (2 * epsilon)
    assert abs(finite_difference) > 1e-8

    net.ppo_update([(state, [(candidates, action, old_logprob)], advantage)], clip=0.2)
    assert (float(parameter[index]) - original) * finite_difference < 0.0


def test_pyo3_and_stdio_have_matching_reset_mask_and_turn() -> None:
    py = LimbusEnv(strict=True)
    std = StdioSimulator(data_dir=str(ROOT / "data"))
    team = ["10110", "10913"]
    enemies = list(SECTION5_WAVE)
    try:
        py.reset(77, team=team, enemies=enemies, max_turns=6, enemy_hp_scale=0.08,
                 infinite_ego_resources=True)
        std.reset(77, team=team, enemies=enemies, strict=True, max_turns=6,
                  enemy_hp_scale=0.08, infinite_ego_resources=True)
        assert py.state_hash() == std.state_hash()
        py_actions = json.loads(py._sim.legal_actions())
        std_actions = json.loads(std.legal_actions())
        assert sorted(json.dumps(a, sort_keys=True) for a in py_actions) == sorted(
            json.dumps(a, sort_keys=True) for a in std_actions
        )
        plan = first_legal_plan(py.observe(), py.legal_actions())
        py_result = py.step_turn(plan)
        wire_plan = [json.loads(action.to_wire()) for action in plan]
        std_result = json.loads(std.step_turn(json.dumps(wire_plan)))
        assert py_result["ok"] and std_result["ok"]
        assert py_result["state_hash_after"] == std_result["state_hash_after"]
        assert py_result["search_key"] == std_result["search_key"]
        assert py_result["stats"] == std_result["stats"]
    finally:
        std.close()


def test_step_turn_is_atomic() -> None:
    env = LimbusEnv(strict=True)
    env.reset(7, enemies=SECTION5_WAVE, max_turns=6, enemy_hp_scale=0.08)
    before = env.state_hash()
    plan = first_legal_plan(env.observe(), env.legal_actions())

    # A partial plan is refused and the state is untouched.
    result = env.step_turn(plan[:-1])
    assert result["ok"] is False, result
    assert "incomplete" in result["error"]
    assert env.state_hash() == before

    # An illegal action is refused too.
    broken = list(plan)
    wire = json.loads(canonical(broken[0]))
    kind = next(iter(wire))
    wire[kind]["skill"] = "9999999"
    broken[0] = wire
    result = env.step_turn(broken)
    assert result["ok"] is False
    assert "illegal action" in result["error"]
    assert env.state_hash() == before

    # The full plan resolves the turn and reports replay material.
    info = env.step_turn(plan)
    assert info["ok"], info
    assert info["state_hash_before"] == before
    assert info["state_hash_after"] != before
    assert info["transition_hash"]
    assert info["seed"] == 7
    assert "draw_count" in info["rng_before"]
    assert info["stats"]["skill_uses"]
    assert isinstance(info["log"], list)


def test_plan_and_mask_match_the_simulator() -> None:
    env = LimbusEnv()
    env.reset(11)
    obs = env.observe()
    legal = env.legal_actions()
    actors = actor_order(obs, legal)
    assert actors, "there must be units to act"
    groups = group_candidates(legal)
    for actor in actors:
        assert groups.get(actor), f"{actor} has no candidates"
    plan = first_legal_plan(obs, legal)
    assert len(plan) == len(actors)
    # The generator's candidate cap is only a prior: every plan it emits is legal.
    generator = PlanGenerator(SkillTable(), width=3, cap=2)
    for _score, candidate in generator.generate(env, obs, legal):
        info = env.clone_state().step_turn(candidate)
        assert info["ok"], info


def test_encoder_is_stable_and_versioned() -> None:
    encoder = Encoder()
    env = LimbusEnv()
    env.reset(3)
    obs = env.observe()
    state = encoder.encode_state(obs)
    assert state.shape == (encoder.state_dim,)
    assert np.array_equal(state, encoder.encode_state(obs))
    legal = env.legal_actions()
    groups = group_candidates(legal)
    actor = actor_order(obs, legal)[0]
    matrix = encoder.action_matrix(obs, groups[actor])
    assert matrix.shape[1] == encoder.action_dim
    # The same state and action always give the same vector.
    assert np.array_equal(matrix[0], encoder.encode_action(obs, groups[actor][0]))
    # The analytic prior is finite for every candidate of the turn.
    for options in groups.values():
        for action in options:
            assert np.isfinite(estimate_action(obs, action, encoder.table))


def test_teacher_plans_are_replayable() -> None:
    encoder = Encoder()
    teacher = BeamTeacher(
        encoder.table,
        encoder,
        TeacherConfig(horizon=2, plan_width=6, candidate_cap=3, turn_width=2, max_turns=3,
                      enemy_hp_scale=0.08),
    )
    env = LimbusEnv(strict=True)
    env.reset(42, enemies=SECTION5_WAVE, max_turns=3, enemy_hp_scale=0.08)
    while True:
        obs = env.observe()
        if obs.get("winner") or obs.get("phase") == "Finished":
            break
        legal = env.legal_actions()
        plan, value, groups, actors = teacher.plan_turn(env, obs, legal)
        assert plan, "the teacher must produce a plan"
        info = env.step_turn(plan)
        assert info["ok"], info
        # Replaying the same plan from the same state hash gives the same result.
        replay = env.clone_state()
        assert replay.state_hash() == info["state_hash_after"]
        assert value == value  # not NaN


def test_rollout_teacher_completes_partial_plans_and_keeps_diverse_candidates() -> None:
    encoder = Encoder()
    teacher = BeamTeacher(
        encoder.table,
        encoder,
        TeacherConfig(
            horizon=1,
            plan_width=4,
            candidate_cap=6,
            turn_width=2,
            rollout_width=3,
            score_mode="rollout",
            candidate_mode="diverse",
            leaf_value_weight=1.0,
            max_turns=2,
            enemy_hp_scale=0.08,
            infinite_ego_resources=True,
        ),
    )
    env = LimbusEnv(strict=True)
    env.reset(
        1234,
        enemies=SECTION5_WAVE,
        max_turns=2,
        enemy_hp_scale=0.08,
        infinite_ego_resources=True,
    )
    obs = env.observe()
    plan, _value, _groups, _actors = teacher.plan_turn(env, obs, env.legal_actions())
    assert plan
    assert env.clone_state().step_turn(plan)["ok"]
    assert teacher.stats["rollout_calls"] > 0
    assert teacher.stats.get("rollout_rejections", 0) == 0
    assert teacher.stats["candidate_kept"] < teacher.stats["candidate_available"]
    assert teacher.stats["bucket_defense_kept"] > 0


def test_teacher_samples_are_labelled_inside_the_candidates() -> None:
    encoder = Encoder()
    teacher = BeamTeacher(
        encoder.table,
        encoder,
        TeacherConfig(horizon=1, plan_width=4, candidate_cap=3, turn_width=2, max_turns=3),
    )
    stats, samples, replay = teacher.run_episode(1234, enemy_hp_scale=0.08)
    assert samples, "the episode must produce decisions"
    assert replay
    for sample in samples:
        labels = sample.labels()
        assert all(label >= 0 for label in labels), labels
        for actor, label in zip(sample.actors, labels):
            assert label < len(sample.groups[actor])
    data = encode_samples(encoder, samples)
    assert data["state"].shape[1] == encoder.state_dim
    assert data["cand"].shape[1] == encoder.action_dim
    assert data["offsets"].sum() == data["cand"].shape[0]
    assert ds.describe(data)["episodes"] == 1


def test_quality_weighting_downweights_generic_near_failures() -> None:
    encoder = Encoder()
    teacher = BeamTeacher(
        encoder.table,
        encoder,
        TeacherConfig(
            horizon=1,
            plan_width=4,
            candidate_cap=3,
            turn_width=1,
            max_turns=2,
            counterfactual_credit=True,
        ),
    )
    _stats, samples, _replay = teacher.run_episode(4321, enemy_hp_scale=0.08)
    assert teacher.stats.get("counterfactual_calls", 0) > 0
    assert samples
    for sample in samples:
        sample.episode_won = False
        sample.episode_quality = 0.25
    plain = encode_samples(encoder, samples)
    weighted = encode_samples(
        encoder,
        samples,
        quality_weighting=True,
        quality_power=2.0,
        failure_weight=0.1,
        credit_weighting=True,
    )
    assert np.all(weighted["weight"] < plain["weight"])
    assert np.all(weighted["weight"] >= 0.1)


def test_dataset_split_is_by_seed() -> None:
    encoder = Encoder()
    teacher = BeamTeacher(
        encoder.table,
        encoder,
        TeacherConfig(horizon=1, plan_width=4, candidate_cap=3, turn_width=1, max_turns=2),
    )
    pieces = []
    for seed in (1, 2, 3, 4):
        _stats, samples, _replay = teacher.run_episode(seed, enemy_hp_scale=0.08)
        pieces.append(encode_samples(encoder, samples))
    merged = ds.merge(pieces)
    assert set(np.unique(merged["seed"])) == {1, 2, 3, 4}
    train, validation = ds.seed_split(merged, 0.5, seed=0)
    train_seeds = set(int(s) for s in np.unique(train["seed"]))
    val_seeds = set(int(s) for s in np.unique(validation["seed"]))
    assert not (train_seeds & val_seeds), "a seed may not be in both splits"
    assert train["cand"].shape[0] == int(train["offsets"].sum())
    # Each decision contains multiple actors, but no actor occurs twice in it.
    decision_keys = list(zip(train["seed"].tolist(), train["decision"].tolist(), train["actor"].tolist()))
    assert len(decision_keys) == len(set(decision_keys))
    assert np.all(train["offsets"] > 0)
    assert np.all(train["label"] >= 0)
    assert np.all(train["label"] < train["offsets"])
    with _scratch_dir() as tmp:
        path = Path(tmp) / "data.npz"
        ds.save_dataset(path, merged)
        again = ds.load_dataset(path)
        assert np.array_equal(again["label"], merged["label"])


def test_policy_and_baselines_produce_legal_plans() -> None:
    encoder = Encoder()
    env = LimbusEnv()
    env.reset(9, enemies=SECTION5_WAVE, max_turns=4, enemy_hp_scale=0.08)
    net = PolicyValueNet(encoder.state_dim, encoder.action_dim, seed=0)
    policies = [
        RandomPolicy(0),
        FirstLegalPolicy(),
        GreedyPolicy(encoder.table, cap=2),
        NeuralPolicy(net, encoder, name="AI-test"),
    ]
    for policy in policies:
        probe = env.clone_state()
        obs = probe.observe()
        plan = policy.plan(probe, obs, probe.legal_actions())
        assert plan, f"{policy.name} produced no plan"
        assert probe.step_turn(plan)["ok"], policy.name


def test_summarise_and_best_of_n() -> None:
    rows = [
        {"seed": 1, "won": True, "kill_turn": 3, "damage_to_enemies": 100, "boss_hp_left": 0,
         "boss_hp_start": 512, "survivors": 5, "sinking_damage": 10, "ego_uses": [], "axis_ok": True},
        {"seed": 2, "won": False, "kill_turn": None, "damage_to_enemies": 50, "boss_hp_left": 400,
         "boss_hp_start": 512, "survivors": 2, "sinking_damage": 5, "ego_uses": [], "axis_ok": False},
    ]
    summary = summarise(rows, short_turn=4)
    assert summary["episodes"] == 2
    assert summary["win_rate"] == 0.5
    assert summary["fastest_kill_turn"] == 3
    assert summary["short_win_rate"] == 0.5
    assert summary["long_tail_failure_rate"] == 0.5
    best = best_of_n(rows, (1, 2))
    # One restart per window: half of the windows win.
    assert best["n1"]["window_win_rate"] == 0.5
    # A single window covering both seeds: best-of-2 wins because one of them did.
    assert best["n2"]["window_win_rate"] == 1.0
    assert best["n2"]["best_kill_turn_min"] == 3
    restart = restart_aware(rows, turn_threshold=4, n_values=(1, 2))
    assert restart["clear_rate_per_attempt"] == 0.5
    assert restart["n2"]["clear_within_n_rate"] == 1.0
    assert restart["n2"]["median_attempts_until_clear"] == 1.0


def test_axis_detection_uses_real_state() -> None:
    replay = [
        {"turn": 1, "stats": {"ego_uses": [], "sinking_damage": 5},
         "boss_sinking": {"potency": 6, "count": 2}},
        {"turn": 2, "stats": {"ego_uses": ["sinner-0-10110|20109"], "sinking_damage": 40},
         "boss_sinking": {"potency": 8, "count": 4}},
        {"turn": 3, "stats": {"ego_uses": [], "sinking_damage": 90},
         "boss_sinking": {"potency": 2, "count": 1}},
    ]
    axis = detect_axis(replay)
    assert axis["first_ego_turn"] == {"20109": 2}
    assert axis["setup_ok"] is True
    assert axis["post_trigger_damage"] >= axis["pre_trigger_damage"]
    empty = detect_axis([])
    assert empty["axis_ok"] is False
    labels = detect_strategy_labels([
        {"turn": 1, "boss_sinking": {"potency": 0, "count": 0},
         "stats": {"ego_uses": ["sinner-2|20807", "sinner-3|20903"], "sinking_damage": 4}},
        {"turn": 2, "boss_sinking": {"potency": 8, "count": 5},
         "stats": {"ego_uses": ["sinner-0|20106"], "sinking_damage": 20}},
    ])
    assert labels["known_setup_like"] is True
    assert labels["peak_sinking_potency"] == 8
    assert labels["pre_burst_count"] == 0


def test_lookahead_policy_returns_a_legal_plan_and_counters_switches() -> None:
    """The hybrid must stay inside the simulator's legal set.

    Its whole point is that the search only *reorders* plans the network already
    considered, so a plan it returns must still be accepted atomically by
    `step_turn`, exactly like the plain argmax policy's plan.
    """
    from lcb.baselines import NeuralLookaheadPolicy

    encoder = Encoder()
    net = PolicyValueNet(encoder.state_dim, encoder.action_dim, seed=3)
    env = LimbusEnv()
    env.reset(11, enemies=SECTION5_WAVE, max_turns=4, enemy_hp_scale=0.08)
    policy = NeuralLookaheadPolicy(net, encoder, beam=3, branch=2, horizon=0)
    record = run_episode(policy, SCENARIO, seed=11)
    assert record.error is None, record.error
    assert record.stats["turns"] >= 1
    assert policy.last_plan_source in ("policy", "lookahead")


def test_effect_probe_is_skipped_for_checkpoints_that_cannot_use_it() -> None:
    """A zero-padded effect block must not cost a simulator turn per candidate.

    Every checkpoint in this repository was trained with `action_dim=44`; adding
    the effect features pads `W2` with zero rows, and those rows stay zero.  The
    probe that fills the features therefore cannot change any decision, but used
    to cost one complete `step_turn` clone per candidate (measured ~1.3 s per
    turn of a battle).
    """
    net = PolicyValueNet(state_dim=6, action_dim=44, hidden=4, seed=5)
    with _scratch_dir() as directory:
        path = Path(directory) / "base.npz"
        net.save(path)
        widened = PolicyValueNet.load(path, action_dim=44 + 22)
    assert widened.action_dim == 66
    assert widened.uses_action_effects() is False

    widened.params["W2"][-22:, :] = 0.25
    assert widened.uses_action_effects() is True


def test_evaluation_smoke() -> None:
    encoder = Encoder()
    record = run_episode(FirstLegalPolicy(), SCENARIO, seed=9001, record_replay=True)
    assert record.error is None, record.error
    assert record.stats["turns"] >= 1
    assert record.stats["boss_hp_start"] > 0
    # The replay carries the hashes the plan requires.
    for entry in record.replay:
        assert entry["state_hash_before"] and entry["state_hash_after"]
        assert entry["transition_hash"]


def main() -> int:
    failures = 0
    for name, function in sorted(globals().items()):
        if not name.startswith("test_") or not callable(function):
            continue
        try:
            function()
            print(f"ok   {name}")
        except AssertionError as exc:  # pragma: no cover - reported
            failures += 1
            print(f"FAIL {name}: {exc}")
        except Exception as exc:  # pragma: no cover - reported
            failures += 1
            print(f"ERROR {name}: {exc!r}")
    print(f"failures: {failures}")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
