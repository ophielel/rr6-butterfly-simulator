"""The scenes the training/evaluation harness uses.

Two are defined, and every report says which one it used:

* `real` - the Line 6 Section 5 wave **as the data describes it**: the Imago with
  its 25616 HP, its three Illusory Butterfly allies, 30 turns.  No scenario knob
  is touched (`enemy_hp_scale = 1.0`).
* `short` - the same wave with the **scenario knob** `enemy_hp_scale = 0.2`
  (Imago 5123 HP) and 20 turns.
* `burst` - `enemy_hp_scale = 0.08` (Imago 2049 HP) and 12 turns.  This is the
  scene the kill-turn statistics are measured on: it is small enough that a
  terminal win is reachable inside the team's lifetime, and the fastest kill is
  still produced by the real `Sinking -> E.G.O -> trigger` burst rather than by
  chip damage.

The scale was chosen by a sweep with the Greedy baseline (cap 6, three seeds):
`0.05` -> kill on turn 3 every time, `0.08` -> 2 of 3 wins on turn 3-4, `0.12` and
`0.16` -> no win (the team wipes with ~2400 damage).  `0.08` is therefore the
scene where a policy can still lose, which is what makes the comparison
meaningful.

Why the knob is needed at all: measured on `real`, the team of 7 the project
models deals ~200-400 damage per turn and wipes around turn 9, so the Imago's
25616 HP cannot be emptied by any policy this harness can search for.  The fight
is a race the current content loses; a reachable terminal win is required to
measure `kill_turn`, short-win rates and best-of-N at all.  The knob changes the
HP pool and nothing else, and `provenance()` writes it into every report.

All three formal research scenes enable `infinite_ego_resources=True`; this is an
explicit experiment condition, not a game rule. `Scenario.enemy_hp_scale` is
documented in `BattleConfig` as a scenario knob, not a game rule, and
`provenance()` writes both settings into every report.
"""

from __future__ import annotations

from typing import Dict

from .evaluate import Scenario

REAL = Scenario(
    name="real",
    max_turns=30,
    enemy_hp_scale=1.0,
    strict=True,
    infinite_ego_resources=True,
    description="Line 6 Section 5 wave exactly as the data describes it (Imago 25616 HP)",
)

SHORT = Scenario(
    name="short",
    max_turns=20,
    enemy_hp_scale=0.2,
    strict=True,
    infinite_ego_resources=True,
    description="the same wave with the scenario knob enemy_hp_scale=0.2 (Imago 5123 HP)",
)

BURST = Scenario(
    name="burst",
    max_turns=12,
    enemy_hp_scale=0.08,
    strict=True,
    infinite_ego_resources=True,
    description="the same wave with the scenario knob enemy_hp_scale=0.08 (Imago 2049 HP)",
)

SCENARIOS: Dict[str, Scenario] = {"real": REAL, "short": SHORT, "burst": BURST}


def scenario(name: str) -> Scenario:
    if name not in SCENARIOS:
        raise KeyError(f"unknown scenario {name!r}; known: {sorted(SCENARIOS)}")
    return SCENARIOS[name]


__all__ = ["REAL", "SHORT", "BURST", "SCENARIOS", "scenario"]
