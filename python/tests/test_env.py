"""Python-side smoke tests: run directly (`python python/tests/test_env.py`).

They only exercise the wrapper, the search layer and the hash/clone contract; the
rules themselves are tested in Rust (`sim/crates/lcb-core/tests`).
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "python"))

from lcb import EngageAction, LimbusEnv, greedy_turn, random_turn  # noqa: E402
from lcb.env import BACKEND, BOSS_IMAGO, TEAM, Action  # noqa: E402
from lcb.search import _fill_and_commit  # noqa: E402


def test_reset_is_deterministic() -> None:
    a = LimbusEnv()
    b = LimbusEnv()
    assert a.reset(seed=11) == b.reset(seed=11)
    assert a.reset(seed=12) != b.state_hash()


def test_action_space_is_unit_skill_target_product() -> None:
    env = LimbusEnv()
    env.reset(seed=3)
    actions = [a for a in env.legal_actions() if isinstance(a, Action)]
    assert actions, "there must be legal actions"
    actors = {a.actor for a in actions}
    targets = {a.target for a in actions}
    assert len(actors) == len(TEAM), "every fixed identity gets a slot"
    assert targets and all(t.startswith("enemy-") for t in targets)


def test_clone_isolation() -> None:
    env = LimbusEnv()
    env.reset(seed=5)
    clone = env.clone_state()
    assert clone.state_hash() == env.state_hash()
    action = next(a for a in env.legal_actions() if isinstance(a, Action))
    clone.step(action)
    clone.commit()
    assert clone.state_hash() != env.state_hash()
    assert env.state_hash() != clone.state_hash()


def test_transition_hash_changes_per_turn() -> None:
    env = LimbusEnv()
    env.reset(seed=7)
    for action in random_turn(env):
        env.step(action)
    first = env.commit()
    assert first["ok"], first
    for action in random_turn(env):
        env.step(action)
    second = env.commit()
    assert second["transition_hash"] != first["transition_hash"]


def test_greedy_picks_the_best_simulated_option() -> None:
    """Greedy must choose the option with the best *simulated* outcome.

    The plan requires that greedy compares actions through the real simulator
    rather than a hand-written estimate, so this test re-derives the choice by
    cloning the environment for every legal action of the first unit and
    checking that greedy's pick achieves the maximum of those scores.
    """
    env = LimbusEnv()
    env.reset(seed=42)
    actor = next(
        a.actor
        for a in env.legal_actions()
        if isinstance(a, (Action, EngageAction))
    )
    options = [
        a
        for a in env.legal_actions()
        if isinstance(a, (Action, EngageAction)) and a.actor == actor
    ]
    assert len(options) > 1, "need several options to make the choice meaningful"

    scores = {}
    for option in options:
        probe = env.clone_state()
        probe.step(option)
        _fill_and_commit(probe)
        scores[option] = LimbusEnv.score(probe.state())

    plan = greedy_turn(env)
    assert plan, "greedy produced no plan"
    assert plan[0] in scores, "greedy picked an action it never evaluated"
    assert scores[plan[0]] == max(scores.values()), "greedy did not pick the best option"


def test_greedy_is_not_worse_than_default_over_seeds() -> None:
    """A sanity check that the simulated search pays off on average.

    This is a stochastic comparison, so it is measured over several seeds (and
    with a small tolerance) rather than on a single one.
    """
    def total(seed: int, policy) -> float:
        env = LimbusEnv()
        env.reset(seed=seed)
        for action in policy(env):
            env.step(action)
        _fill_and_commit(env)
        return LimbusEnv.score(env.state())

    seeds = [7, 11, 13]
    greedy = sum(total(seed, greedy_turn) for seed in seeds) / len(seeds)
    default = sum(
        total(seed, lambda e: [a for a in e.legal_actions() if isinstance(a, Action)][:1] or [])
        for seed in seeds
    ) / len(seeds)
    assert greedy >= default - 500.0, (greedy, default)


def test_unknown_rules_are_reported() -> None:
    env = LimbusEnv()
    env.reset(seed=1)
    rules = env.unknown_rules()
    assert any("clash" in r.lower() for r in rules)
    assert any("RNG" in r or "rng" in r for r in rules)


def main() -> int:
    print(f"backend: {BACKEND}")
    failures = 0
    for name, func in sorted(globals().items()):
        if name.startswith("test_") and callable(func):
            try:
                func()
                print(f"  ok   {name}")
            except AssertionError as exc:
                failures += 1
                print(f"  FAIL {name}: {exc}")
    print("failures:", failures)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
