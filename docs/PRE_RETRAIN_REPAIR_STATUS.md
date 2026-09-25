# Pre-Retrain Repair Status

**Status: post-audit retraining and evaluation complete.** This is a live checkpoint of the current working tree. Changes are uncommitted and have not been pushed. The fresh post-audit data/checkpoints are separate from legacy outputs; the remaining items below are scope/documentation follow-ups, not permission to mix datasets.

## Done so far

### Core data and mechanics edits

- Updated the source-driven pipeline (`build_library.py`, `extract_effects.py`, `extract_status_effects.py`) so a normal `tools/build_all.py --skip-fetch` regenerates current unnumbered Threadspin IV skill variants.
- Updated `data/ego/20903.json` and corresponding mechanics entries for Rime Shank Threadspin IV:
  - Awakening inflicts 5 Sinking Potency and 5 Count.
  - Corrosion inflicts 10 Potency and 8 Count, with Attack Weight 3.
  - Awakening and Corrosion retain the +30% damage clause above 50% target HP from their skill text.
- Updated the Bygone Days Yi Sang awakening data to use `6 + 1.5 × Gloom Resonance`.
- Added `inflict_random_each`: Bygone computes `floor(6 + 1.5 × Gloom Resonance)`, makes that many individual 1-Potency Sinking applications, and chooses a living opponent for each with the replayable RNG. Each application is added to `BattleState.status_gain_events`.
- Added Sinking gain-event recording for ordinary Skill/Coin infliction. Events now carry source actor, target, source type, skill ID when emitted by a SkillUse, coin index, and raw effect text. Echoes of the Manor rolls 50% separately for each qualifying Potency or Count gain component and records a successful +1 Count with source type `echoes_of_the_manor`.
- Corrected Echoes of the Manor's status data to reduce Potency by 1 at Turn End and removed the non-cumulative clause from the unmodeled list. Direct and queued grants replace the active value rather than adding to it.
- Added `BattleConfig.infinite_ego_resources` (default `false`). When enabled, E.G.O resource affordability is unlimited and resource spending is skipped; SP cost and Overclock multiplier remain active.
- Exposed the setting through PyO3 reset, Python environment/scenario, and stdio reset configuration. Evaluation replay payloads now include the scenario's complete config.
- Removed an always-true branch in `detect_axis` and replaced the dataset test's `or True` with uniqueness, nonempty candidate, and label-range checks.
- `Action::Engage` now carries the enemy UnitId and slot; resolution keys pulled slots by both identity and slot. Python features and stdio/PyO3 wire paths preserve that identity.
- Added atomic `submit`/`step_turn` commands to the stdio CLI and a PyO3/stdio parity test covering reset config, legal mask, state hash, search key, and turn stats.
- Added strict core preflight for the critical Bygone Days and Rime Shank E.G.O mechanics, and added restart-aware evaluation metrics for N=1/5/10/20/50 with an 8-turn threshold.
- Added `search/run_oracle.py` for the scripted Ishmael/Rodion/Yi Sang setup; a burst smoke run clears in 3 turns and a real-HP three-turn smoke run executes successfully.
- Added `docs/FIXED_CONTENT_SCOPE.md` documenting core fidelity targets, current approximations, and open gates.

### Tests added

- Rime Shank Awakening/Corrosion exact Sinking values and Attack Weight 3.
- Bygone Days creates 12 individual gain events at Gloom Resonance 4, with total Potency 12 and deterministic target assignment.
- Echoes + Bygone repeated application is deterministic for a fixed seed and makes separate per-event Count rolls.
- Infinite resources bypass resource cost while Overclock still spends 23 SP for the tested 15-SP E.G.O.
- PPO finite-difference direction check added in `python/tests/test_training.py`.

## Verification performed

- Rust targeted tests passed individually:
  - `rime_shank_threadspin_iv_inflicts_exact_sinking_values_and_weight`
  - `bygone_days_threadspin_iv_applies_sinking_as_individual_gain_events`
  - `echoes_of_the_manor_rolls_once_per_bygone_gain_and_replays_deterministically`
  - `infinite_ego_resources_bypass_affordability_and_spending_only`
- Full Rust suite now passes: 118 tests.
- Python training suite now passes: all tests, including PPO finite-difference and PyO3/stdio parity.
- The PyO3 extension was rebuilt from the current Rust sources and copied to `python/lcb/lcb_sim.pyd`; the stale-extension failure and W3 indexing issue are resolved.
- `tools/build_all.py --skip-fetch` now passes after the extractor changes and regenerates the intended Threadspin IV records and `inflict_random_each` mechanics.
- Backend parity, strict critical preflight, multi-hit Sinking audit, and the human setup oracle are complete. A fresh 4-point Teacher HP sweep (8 seeds/scale), BC-from-scratch, corrected PPO fine-tune, 50-seed burst/real-HP evaluation, N=1/5/10/20/50 restart metrics, and `reports/post_audit_summary.json` are complete. Real HP produced 0/50 wins for Random, FirstLegal, Greedy, Teacher, BC, and PPO; the 0.08 burst sanity scene produced 100% wins for Greedy, Teacher, BC, and PPO in this run.

## Not done / open

1. **Rime Shank multi-target allocation:** exact P/C, AW=3, the >50% HP damage boundary, and Attack Weight 3 resolving the main slot plus two additional enemy slots are tested.
2. **Status events are partial:** SkillUse paths carry `skill_id`; passive/status-only paths intentionally do not. Not every Sinking application path emits events, and the event buffer is cleared at Turn Start rather than exposed as a separate immutable turn log.
3. **Echoes integration gaps:** direct target replacement, queued replacement, Turn End reduction, and Bygone per-event integration tests pass. Echoes panic-type transfer and Non-SP head-rate effects remain outside this patch. Echoes panic-type transfer and Non-SP head-rate effects remain outside this patch.
4. **Fixed-content data pipeline:** current Threadspin IV and Bygone event semantics are now reproducible from `tools/build_all.py`; broader generated-data diffs should still be reviewed before commit.
5. **Infinite-resource metadata and strict mode:** scenario serialization and Python/PyO3/stdio reset parity carry the option. Strict mode currently blocks the critical Bygone/Rime mechanics only; a complete global unknown-rule blocker audit remains open.
6. **Engage edge cases:** `Action::Engage` now carries enemy identity and slot. Same-slot collision behavior across multiple enemy owners needs a focused regression test.
7. **Backend parity:** reset/mask/one-turn parity is covered; broader clone/load-state and error-path parity remain open.
8. **PPO:** the finite-difference direction test passes. It checks the current clipped-surrogate implementation, not a full statistical PPO training run.
9. **Sinking trigger audit:** P/C 10/8 across five triggers now asserts five triggers, unchanged Potency, and Count 3.
10. **Research workflow:** generic-only Teacher objective, post-hoc route labels, fresh data, BC, PPO, and restart-budget evaluation are complete. The Teacher route labels did not identify the hand-scripted setup (`known_setup_like_rate=0`), which is an expected emergence result rather than a success claim.
11. **Documentation/reports:** README, scope/checkpoint docs, and training results now distinguish post-audit from legacy outputs. Existing training reports/checkpoints outside the post-audit paths remain legacy and must not be presented as new-condition results.

## Working tree

All current changes are local and uncommitted. Main modified areas are `sim/crates/lcb-core`, `sim/crates/lcb-py`, CLI stdio reset, Python scenario/evaluation, test files, E.G.O records, and mechanics JSON. Review `git status --short` before resuming.

## Suggested resume order

1. Review the generated-data diff and decide whether the remaining global unknown-rule inventory is in scope for a later simulator release.
2. Add clone/load-state and error-path parity coverage beyond reset/mask/one-turn parity if the backend becomes a deployment target.
3. Repeat the post-audit experiment with larger seeds/search budgets before making statistical claims; the current run is an engineering benchmark, not a power analysis.
4. Update README/STATUS/results with the compact post-audit report and clearly separate real-HP failure from scaled burst sanity results.
5. Commit and push the source/docs changes; keep legacy and post-audit artifacts in separately named directories.
