# Archive

Planning and review documents whose work is **finished**.  They are kept because
every mechanic in the simulator cites them, and because the review documents are
the record of what was checked and how.

| Document | Status |
|---|---|
| `rr6_butterfly_simulator_plan_v2.md` | The project plan.  Its **first-stage goal is complete**: the library, the Rust core, base combat, the seven identities and seven E.G.O, the Section 5 Imago, replay/hash and the tests all exist and are covered (see `docs/COVERAGE.md` and `docs/STATUS.md`).  The plan's **search stage** is deliberately a later step ("模拟器完成后"): Random, Greedy and Beam exist in `python/lcb/search.py`, MCTS raises on purpose and Learning is not started. |
| `REVIEW_2026-09-20.md` | 5 findings, all fixed. |
| `REVIEW_SIMULATOR_RECHECK.md` | 4 findings, all fixed. |
| `REVIEW_SIMULATOR_LATEST.md` | 3 findings: the cracked-Coin power and the stolen chain are fixed; the E.G.O resource reservation is skipped on purpose (the E.G.O resources are treated as unlimited in the experiments). |
| `REVIEW_SIMULATOR_UNLIMITED_EGO.md` | E.G.O-focused review, addressed together with `REVIEW_SIMULATOR_LATEST.md`. |
| `REVIEW_SIMULATOR_FULL.md` | 20 groups across the whole simulator.  All of them are resolved: see the "Full-review items" table in `docs/STATUS.md`.  Two entries stay open by scope/data, not by unknown rules (the illusory butterflies and Parts are documentation-only per the plan; Outis' LCA Fracture Rounds have no in-kit source). |

Nothing here is normative any more: the living documents are `README.md`,
`docs/MECHANICS.md` (the rule table, sources and verification status),
`docs/COVERAGE.md` (generated coverage) and `docs/STATUS.md` (what is done and
what is honestly unknown).
