# Implementation status

Mirrors the phases of the project plan.  "Done" means implemented **and** covered
by a test that cites a source in `docs/MECHANICS.md`.

## Phase 1 — base state — done

| Item | Where | Test |
|------|-------|------|
| HP / max HP / shield | `state::Unit` | `engineering::clone_is_a_deep_copy` |
| Sanity as an enum (`None` / `Sane{sp}`) | `state::Sanity` | `mechanics::sanity_changes_coin_flip_odds` |
| Speed ranges, per-turn roll | `setup`, `battle::begin_turn` | `mechanics::turn_advances_phase_and_logs` |
| Deterministic RNG | `rng::Rng` | `rng::tests::*` |
| Statuses (Potency/Count/Stack) | `state::StatusSet` | `mechanics::burn_ticks_at_turn_end` |
| Clone / hash / serialisation | `replay`, `hash` | `engineering::*` |

## Phase 2 — skills — done

Skill composition with `Skill Amount` copies, two-skill dashboard slots with
rotation, extra Skill Slots from turn 2, uptie resolution (1..4), defense
skills, action slots.  Tests: `mechanics::deck_draws_randomly_and_resets_after_full_placement`,
`mechanics::panel_rotates_after_use`.

## Phase 3 — combat — done

Coin rolls, **summed** clash power with skill-level bonus, ties that destroy
nothing, "destroy the first remaining coin", accumulating attack damage,
resistances, stagger, guard/evade/counter (guard shield on first incoming
attack, guard replacing the Defense Level).  Tests:
`mechanics::clash_power_sums_every_coin`, `mechanics::clash_tie_destroys_no_coin`,
`mechanics::clash_loser_loses_one_coin_per_round`, `mechanics::damage_formula_matches_source`,
`mechanics::stagger_levels_match_source`.

## Phase 4 — statuses — partial

Implemented: Burn, Bleed, Sinking, Butterfly (unique Sinking), The Living & The
Departed (ammo), Poise, Fragile, Paralyze (power fix), Haste, Shield,
Unbreakable Coin, Reload (Solemn Lament).

Loaded but **not** simulated: the Imago's `In the Past / Present / Future`
state machine, `Temporal Disjunction`, Scale Dust, and anything else listed per
skill in `data/mechanics/effects.json` → `unmodeled`.

## Phase 5 — E.G.O — partial

Implemented: resource costs, SP costs, Awakening/Corrosion selection, Overclock
(1.5x cost, rounded up), coin reuse from "Reuse this Coin ..." text, resource
generation from base attack skills.

Not implemented: E.G.O passives, `Indiscriminate` targeting, E.G.O corrosion
forced actions, threadspin tiers beyond the highest variant.

## Phase 6 — boss — partial

Loaded with official ids and full effect text:

* `9563` Butterfly of Entangled Lives::The Pupa (6 skills, 1 passive)
* `9567` Butterfly of Entangled Lives::Imago (12 skills, 5 passives)
* `9564/9565/9566` Illusory Butterfly :: The Past / The Present / The Future

Not implemented: the time-state machine and its rotations, illusory-butterfly
stack removal and damage transfer, the Section 5 choice event.  The enemy takes
one skill slot and walks its skill list in order (`EnemyPolicy::Cyclic`), which
is a documented stand-in, not the real rotation.

## Search ladder

| Stage | Status |
|-------|--------|
| Random | done — `lcb.search.random_turn` |
| Greedy | done — `lcb.search.greedy_turn`, evaluates every candidate through a real simulated turn |
| Beam | done (turn-level, width/horizon configurable) — `lcb.search.beam_turn` |
| MCTS | **not implemented on purpose** — `lcb.search.mcts_turn` raises, see the plan's "do not start large-scale search before the simulator is finished" |
| Learning | not started |

## Verification gaps

* No golden test from gameplay footage or screenshots yet (the plan requires
  video/screenshot/manual records).  Everything currently rests on wiki.gg plus
  the in-game text dump, i.e. `single_source_verified` /
  `multi_source_verified` — **no `video_verified` record exists**.
* Derived numbers that a player can read off the screen have not been compared
  against screenshots: Imago max HP (`9090 + 275.44 x 60 = 25616`), the Pupa's
  shield (1.3% of max HP), speed rolls.
* Damage magnitudes have not been compared against a recording.  As a
  consistency check, the simulated coin rolls reproduce the JA-wiki worked
  examples exactly (4+4 x3 → 8/12/16, 1+6 x5 → 7/13/19/25/31), and the Pupa's
  "1.3% HP as Shield" gives 333 on 25616 max HP, matching the illusory
  butterfly's fixed 333 Shield in the same encounter.
* `BattleState::preset_flips` exists to feed a recorded coin sequence into the
  simulator (golden tests); no recording has been made yet.
