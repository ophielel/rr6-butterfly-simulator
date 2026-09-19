"""Demo driver: play the fixed team against the RR6 Imago and print a summary.

    python python/demo.py --turns 3 --policy greedy --seed 1

Policies: `random`, `greedy`, `beam`, `first`.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from lcb import LimbusEnv, beam_turn, greedy_turn, random_turn  # noqa: E402
from lcb.env import BOSS_IMAGO, TEAM, Action  # noqa: E402
from lcb.search import _fill_and_commit  # noqa: E402


def first_turn(env: LimbusEnv):
    plan = []
    for action in env.legal_actions():
        if isinstance(action, Action) and action.actor not in {a.actor for a in plan}:
            plan.append(action)
    return plan


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--turns", type=int, default=3)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--policy", default="greedy", choices=["random", "greedy", "beam", "first"])
    parser.add_argument("--team", default=",".join(TEAM))
    parser.add_argument("--enemies", default=BOSS_IMAGO)
    parser.add_argument("--strict", action="store_true")
    args = parser.parse_args()

    env = LimbusEnv(strict=args.strict)
    env.reset(
        args.seed,
        team=args.team.split(","),
        enemies=args.enemies.split(","),
    )
    print(f"unknown rules ({len(env.unknown_rules())}):")
    for rule in env.unknown_rules():
        print(f"  * {rule}")
    if args.strict:
        blockers = env.strict_blockers()
        print(f"strict blockers: {len(blockers)}")

    for turn in range(args.turns):
        if args.policy == "random":
            plan = random_turn(env)
        elif args.policy == "greedy":
            plan = greedy_turn(env)
        elif args.policy == "beam":
            plan = beam_turn(env, width=2)
        else:
            plan = first_turn(env)
        for action in plan:
            env.step(action)
        result = _fill_and_commit(env, [])
        state = env.state()
        alive = [u for u in state["units"] if u["alive"]]
        enemies = [u for u in alive if "Sinner" not in u["kind"]]
        sinners = [u for u in alive if "Sinner" in u["kind"]]
        print(
            f"turn {state['turn']:>2}  sinners {len(sinners)}  "
            f"enemy hp {sum(u['hp'] for u in enemies)}  "
            f"ally hp {sum(u['hp'] for u in sinners)}  "
            f"state {result['state_hash']}  transition {result['transition_hash']}"
        )
        if state.get("winner"):
            print("winner:", state["winner"])
            break
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
