# Post-audit Teacher sweep

This directory is fresh post-repair data. It was generated only after the Rust/Python tests, PyO3/stdio parity, strict critical preflight, and PPO finite-difference check passed.

- Objective: generic combat outcome; Sinking/E.G.O route labels are post-hoc only.
- Scenario: RR6 Section 5, `infinite_ego_resources=true`.
- HP scales: 0.08, 0.20, 0.50, 1.00.
- Seeds: 1–8 per scale.
- Budget: horizon 3, plan width 16, candidate cap 4, turn width 3, max 20 turns.

Do not merge this data with `data/teacher`, `data/teacher_tree`, or `data/teacher_rollout`; those directories are legacy/pre-audit unless separately re-generated.
