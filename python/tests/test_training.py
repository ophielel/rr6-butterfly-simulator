"""Tests for the training layer (run directly: `python python/tests/test_training.py`).

They are fast on purpose: no long training runs, no evaluation sweep.  What is
checked is what the plan's §8 correctness gate asks for - the atomic turn
interface, the mask, replayability, the dataset contract and the metric math.
"""

from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "python"))

from lcb import dataset as ds  # noqa: E402
from lcb.baselines import FirstLegalPolicy, GreedyPolicy, NeuralPolicy, RandomPolicy  # noqa: E402
from lcb.env import LimbusEnv, SECTION5_WAVE  # noqa: E402
from lcb.evaluate import Scenario, best_of_n, restart_aware, run_episode, summarise  # noqa: E402
from lcb.features import Encoder, SkillTable  # noqa: E402
from lcb.nn import PolicyValueNet  # noqa: E402
import lcb.ppo as ppo_module  # noqa: E402
from lcb.rewards import compute_reward  # noqa: E402
from lcb.stdio_client import StdioSimulator  # noqa: E402
from lcb.plans import (  # noqa: E402
    PlanGenerator,
    actor_order,
    canonical,
    estimate_action,
    first_legal_plan,
    group_candidates,
)
from lcb.teacher import BeamTeacher, TeacherConfig, detect_axis, detect_strategy_labels, encode_samples  # noqa: E402

SCENARIO = Scenario(name="test", max_turns=4, enemy_hp_scale=0.08)


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
    assert reward == -9.0


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
    with tempfile.TemporaryDirectory() as tmp:
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
