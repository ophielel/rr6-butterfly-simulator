"""Python-side smoke tests: run directly (`python python/tests/test_env.py`).

They only exercise the wrapper, the search layer and the hash/clone contract; the
rules themselves are tested in Rust (`sim/crates/lcb-core/tests`).
"""

from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "python"))

from lcb import LimbusEnv, greedy_turn, random_turn  # noqa: E402
from lcb.env import BACKEND, BOSS_IMAGO, TEAM, Action  # noqa: E402


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


def test_greedy_uses_simulation_and_beats_random_rollout() -> None:
    """Greedy must be at least as good as a random turn on the same seed.

    Both policies are scored with the same objective, read from the simulated
    state (never from a hand-written damage estimate).
    """
    def play(policy) -> float:
        env = LimbusEnv()
        env.reset(seed=42)
        for action in policy(env):
            env.step(action)
        env.commit()
        return LimbusEnv.score(env.state())

    greedy = play(lambda e: greedy_turn(e))
    best_random = max(play(lambda e: random_turn(e, __import__("random").Random(s))) for s in range(3))
    assert greedy >= best_random, (greedy, best_random)


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
