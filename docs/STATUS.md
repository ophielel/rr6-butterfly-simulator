# Implementation status

Mirrors the phases of the project plan.  "Done" means implemented **and** covered
by a test that cites a source in `docs/MECHANICS.md`.

**Current state (2026-09-26):** all seven identities, the seven E.G.O and the
whole **Section 5 wave** (the Imago plus its three Illusory Butterfly allies) have
**zero** unmodelled effect lines (see `docs/COVERAGE.md`); 147 Rust tests and the
Python suites pass; Sinking Potency/Count are capped at the sourced default Max Value
of 99; the training stack (`docs/TRAINING.md`,
`docs/TRAINING_RESULTS.md`) is implemented and evaluated; the honest unknowns are
listed at the end of this file.

## Phase 1 — base state — done

| Item | Where | Test |
|------|-------|------|
| HP / max HP / shield | `state::Unit` | `engineering::clone_is_a_deep_copy` |
| Sanity as an enum (`None` / `Sane{sp}`) | `state::Sanity` | `mechanics::sanity_changes_coin_flip_odds` |
| Speed ranges, per-turn roll | `setup`, `battle::begin_turn` | `mechanics::turn_advances_phase_and_logs` |
| Deterministic RNG | `rng::Rng` | `rng::tests::*` |
| Starting SP (45 by default, configurable) | `state::BattleConfig::starting_sp` | `mechanics::sinners_start_with_the_configured_sanity` |
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

The Imago's own state machine is driven by its skills ("[Clash Win] Gain 5 [In the
Past]", "[Clash Lose] Halve [In the Past]"), which required status grants to
respect Stack vs Potency/Count — a bug fixed at the data level
(`structure` per status in `data/statuses/statuses.json`).
Tests: `mechanics::imago_stack_gains_halve_and_bonuses`.

## Phase 4 — statuses — partial

Implemented: Burn, Bleed, Sinking (Potency/Count capped at the sourced default Max
Value 99), Butterfly (unique Sinking), The Living & The Departed (ammo), Poise,
Fragile, Rupture, Tremor + Tremor Burst, Protection,
Damage Up/Down, Power Up/Down, Attack Power Up/Down, Plus Coin Boost / Minus
Coin Drop, Offense/Defense Level Up/Down, `<Sin>`/`<Type>` Resist Down,
`<Sin>` Fragility, Bind, Haste, Paralyze (per-coin power fix), Charge tick,
Shield, Unbreakable Coin, Reload (Solemn Lament), Temporal Disjunction.

Also: per-turn / per-encounter effect limits, next-turn grants, resource
consumption for damage, Shield formulas, defense-skill retaliation and
Aggro-aware targeting.

Tests: `mechanics::per_turn_limits_are_enforced`,
`mechanics::next_turn_buffs_apply_at_turn_start`,
`mechanics::consume_status_for_damage`, `mechanics::shield_percent_from_sp`,
`mechanics::inflicted_statuses_actually_land`, `mechanics::rupture_ticks_on_hit`,
`mechanics::protection_and_fragile_modify_incoming_damage`,
`mechanics::power_statuses_change_final_power`,
`mechanics::tremor_burst_raises_stagger_threshold`, `mechanics::bind_lowers_speed`.

Loaded but **not** simulated: the identity-specific statuses the fixed content
uses (Deep Tears, Faint Aroma, Dazzle, Tear-sharpened, Protecting Sword,
Bullet - Solitude, Aggro, Discard, Amplitude Conversion, Scale Dust) and
Sin Resonance.  Anything not modelled is listed per skill in
`data/mechanics/effects.json` → `unmodeled` and reported by strict mode.

## Phase 5 — E.G.O — partial

Implemented: resource costs, SP costs, Awakening/Corrosion selection, Overclock
(1.5x cost, rounded up), reuse from "Reuse this Coin ..." text (tossed again, cap
per Skill), resource
generation from base attack skills.

Not implemented: E.G.O passives, `Indiscriminate` splash targeting for
hand-written Corrosion Skills, E.G.O corrosion
forced actions, threadspin tiers beyond the highest variant.

## Phase 6 — Section 5 boss — essentially complete

Deliverable scope: the Imago (`9567`) alone.  The Pupa and the illusory
butterflies are extras kept only where their data documents Imago mechanics.

Loaded with official ids and full effect text:

* `9563` Butterfly of Entangled Lives::The Pupa (6 skills, 1 passive)
* `9567` Butterfly of Entangled Lives::Imago (12 skills, 5 passives)
* `9564/9565/9566` Illusory Butterfly :: The Past / The Present / The Future

Implemented: the six-Skill-Slot action pattern (three-turn cycle, small/mid/big
skill per state of time, separate patterns below 66% / 33% HP, idle slots on
turn 3, acting while Staggered), the `In the Past / Present / Future` state
machine with its Stacks and Temporal Disjunction, the three state passives
(burn on hit, Kalpāgni SP damage, Poise gain, Stagger-Threshold raise, Bleed
counts, Bloodflower heal, Bleed lifesteal) and the Stack-based bonuses.
Tests: `mechanics::imago_plays_the_documented_rotation`,
`mechanics::time_state_follows_the_highest_stack`,
`mechanics::time_state_stack_bonus_matches_game_text`.

Implemented as well: station 1 (the Pupa) with its 1.3% encounter-start Shield,
the "HP does not fall below 90%" floor, the barrier-break branch and the
"End the Encounter" resolution (`Winner::EncounterEnded`), plus "deals 0 damage"
/ "does not take damage" skills.  Tests: `mechanics::pupa_shield_and_hp_floor`,
`mechanics::pupa_barrier_break_switches_pattern_and_quickening_ends_the_encounter`,
`mechanics::quickening_deals_and_takes_no_damage`.

Implemented as well: Sin Resonance and Absolute Resonance counting (the level
bonus table stays unimplemented, see MECHANICS.md), resonance payoffs such as
"Gain (highest Reson.)", plus attack adders, unbreakable conversion, resource
spending for damage and Shield formulas.  Tests:
`mechanics::sin_resonance_is_counted_from_the_dashboard`,
`mechanics::gain_from_resonance_uses_the_highest_value`.

Coverage of the fixed identities is now: Solemn Lament 109 clauses modelled /
0 unmodelled, Faint Aroma & Solitude 80/1, Jeong's Office 89/13, The Sword
Sharpened with Tears 46/11, Los Mariachis 33/1, LCA Udjat Vanguard 102/9,
Lamp 73/13 (660 modelled / 197 unmodelled overall, generated by
`tools/report.py`).

Stations 2-4 are playable: the illusory butterflies have 1 HP and 333 Shield,
Eclosion ends the stage (or runs one more turn when the Shield broke), and their
Segmentation passive moves Stacks into the campaign state.  `Simulator::section5`
starts the Imago from that campaign (Pupa HP, disabled passive components).
Tests: `mechanics::illusory_butterfly_shield_and_segmentation`,
`mechanics::section5_carries_the_campaign`.

Not implemented: the choice-event UI itself (the campaign records the disabled
components), the illusory butterflies' "Origination" damage transfer (they are
1 HP, so it rarely matters), Sin Resonance's level-gain table, Panic/Low Morale,
E.G.O Corrosion forced actions, focused-encounter Parts, and the debuffs' own
effects (HP Healing Down, Wrath Fragility).

## Training and evaluation (2026-09-21)

Specification: `docs/TRAINING_PLAN.md`.  Implementation: `docs/TRAINING.md`.
Results and the §7.1 acceptance table: `docs/TRAINING_RESULTS.md`. The current T1/curriculum report is `reports/ppo_hp_research_t1_curriculum.json`: 99.0% half-HP and 81.2% real-HP over 500 unseen seeds.

| Item | Where | Test |
|------|-------|------|
| Atomic whole-turn plan API (`submit_plan`, rollback on illegal/incomplete) | `Simulator::submit_plan` | `engineering::submit_plan_is_validated_atomically`, `engineering::submit_plan_matches_incremental_submission` |
| Per-turn real statistics (damage, Sinking triggers, Skill/E.G.O uses, deaths) | `state::TurnStats` | `engineering::submit_plan_is_validated_atomically` |
| Log-free transposition key, compact observation | `hash::search_key`, `PySimulator::observation_json` | `python/tests/test_training.py` |
| One action per unit, mask shared by search/policy/eval | `plans.actor_order` / `PlanGenerator` | `test_plan_and_mask_match_the_simulator` |
| Turn-level beam teacher (registered T0/T1/T2 budgets) | `teacher.BeamTeacher` | `test_teacher_plans_are_replayable`, `test_teacher_samples_are_labelled_inside_the_candidates` |
| Reward (§3) and episode metrics (§7) | `rewards.py`, `evaluate.py` | `test_summarise_and_best_of_n` |
| Behaviour cloning, PPO, state-clone DAgger | `bc.py`, `ppo.py`, `training/run_dagger.py` | `training/train_bc.py`, `training/train_ppo.py` logs |
| Axis detection from real state and stats | `teacher.detect_axis` | `test_axis_detection_uses_real_state` |
| Encounter main enemy ends the wave | `battle` victory check, `EnemyRecord::encounter_boss` | `mechanics::defeating_the_main_enemy_ends_the_wave` (rule 102) |

## Search ladder

| Stage | Status |
|-------|--------|
| Random | done — `lcb.search.random_turn` |
| Greedy | done — `lcb.search.greedy_turn`, evaluates every candidate through a real simulated turn |
| Beam | done (turn-level, width/horizon configurable) — `lcb.search.beam_turn` |
| MCTS | **not implemented on purpose** — `lcb.search.mcts_turn` raises, see the plan's "do not start large-scale search before the simulator is finished" |
| Learning | not started |

## Full-review items (2026-09-20)

`docs/archive/REVIEW_SIMULATOR_FULL.md` lists 20 groups.  All of them are resolved; the two
rows that remain open are limits of scope or data, not unknown rules, and they are
marked as such instead of being guessed.

| Item | Status | Why it is still open |
|---|---|---|
| Ammo subsystem (The Living & The Departed, Bullet - Solitude, LCA Fracture Round) | `DONE` (one gap) | The three pools are spent by name, the two-part pool opens at 10 of each and its split mirrors onto Butterfly, an emptied pool cancels the remaining Coins and reloads, and `[Reload]` refills each pool (SP cost included).  Open: Outis' LCA Fracture Rounds have no in-kit source (they arrive from outside the identity), so a scenario has to grant them. |
| Rodion / Hong Lu / Ryoshu / Sinclair core passives | `DONE` | Rodion's Blessing / Despair (the Turn Start state, the statuses' own upkeep and the **Plus / Minus Coin Skill replacement**), Ryoshu's Petals (Sinking damage, Tremor Bursts, the [Faint Aroma] she inflicts, +([Faint Aroma])% against Staggered targets) and her three Unopposed Attacks, Hong Lu's Kōzan follow-up plus the `[Bright -光-]` kit and the deferred Hand conversion, Outis' Sheut Fracture rider, Gregor's Speed-free redirect. |
| Gregor's "Clash regardless of Speed" passive | `DONE` | `Action::Engage` accepts a chain when the unit carries `Dazzling Lamp` or when the enemy Slot's Skill is tagged "Can Clash with this Skill regardless of Speed". |
| E.G.O target counts and Corrosion targeting | `PARTIAL` | Coin-3 hit counting, "2 allies including this unit" and the Resonance-driven SP-heal count are fixed; the general Corrosion target selection is still approximated. |
| Stations 2-4 butterflies / Pupa, Parts | `OUT OF SCOPE` (the **Section 5** illusions are done, rules 99-101) | Loaded as documentation only, as the plan requires; the choice events of the earlier stations are explicitly not modelled (an "End the Encounter" Skill is honoured where its text is understood). |
| Breath (Poise) critical rate | `DONE` | The critical hit modifier is the fixed 20% the wiki documents (`0.2` plus modifiers) and Poise's chance is its Potency in percentage points (the status caps at 99), so nothing is left as an assumption (rule 96). |
| Sin Resonance level-gain table | `DONE` | Rules 1..11+ are applied to each Skill's Offense/Defense Level from its chain position (rule 88). |

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
