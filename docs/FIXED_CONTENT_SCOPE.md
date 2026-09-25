# Fixed Content Scope: RR6 Section 5 Sinking Research

This simulator is a focused forward model for RR6 Section 5 strategy experiments. It is not a complete Limbus Company emulator. Formal retraining is gated on the Core items below; Simplified items must not be mistaken for verified exact behavior.

## Core / high-fidelity requirements

| Mechanic | Current model | Known difference / evidence | Research relevance |
|---|---|---|---|
| Rime Shank Threadspin IV Awakening | 15 + 15, Attack Weight 3, +30% damage above 50% target HP, Tremor Burst, 5 Sinking Potency + 5 Count | Data is generated from the unnumbered current wiki skill variant; exact P/C, AW, and the >50% HP damage boundary are tested. Source: cached wiki.gg page `Rime_Shank_Rodion.wikitext`, Awakening skill variants. | Critical setup |
| Rime Shank Threadspin IV Corrosion / Overclock | 21 + 6, Attack Weight 3, +30% above 50%, 10 Potency + 8 Count | Data is generated from the unnumbered current wiki skill variant; exact P/C, AW, and the >50% HP damage boundary are tested. | Critical setup |
| Yi Sang Bygone Days Threadspin IV Awakening | `N = floor(6 + 1.5 × Gloom Resonance)` independent 1-Potency applications, each randomly assigned to a live opposing unit | Implemented as repeated deterministic RNG selections and status-gain events. Events emitted through a SkillUse carry the owning skill ID; passive/status-only paths intentionally carry no skill ID. | Critical setup, event granularity |
| Echoes of the Manor | Per qualifying Sinking gain component, 50% replayable RNG chance for +1 Count; next-turn status grants replace the old value; potency reduces at Turn End | Implemented for Sinking gains through normal skill/effect resolution and Bygone's repeated events. Panic-type effects and Non-SP head chance modifier are not implemented. | Critical integration |
| Sinking Potency / Count trigger | Potency damages SP or HP; each trigger consumes 1 Count; existing multi-hit loop triggers per hit | A 10/8 status applied across five triggers is tested: five triggers, Potency 10 remains, Count falls to 3. | Critical burst math |
| E.G.O / Overclock costs and legal actions | `infinite_ego_resources` bypasses affordability and resource deduction only; SP cost and skill effects remain | Config is explicit, default false. Python scenarios can opt in; replay records include the complete scenario config, and PyO3/stdio parity is tested. | Experiment condition |
| Boss HP, damage, targeting | Actual Section 5 wave, real Imago HP by default, existing damage and target resolution | `enemy_hp_scale` remains a labeled test knob only. Engage carries the owning enemy UnitId and slot through Rust, PyO3, stdio, search, and feature encoding. | Long-horizon planning |

## Simplified mechanics (not retrain-gate evidence)

| Mechanic | Current approximation | Expected difference | Research relevance |
|---|---|---|---|
| Identity-specific damage loops | Existing script/effect coverage | Some auto-follow-ups, ammo loops, and chained attacks can be incomplete or approximated | High for final clear-turn count; defer until after setup emergence |
| Support passives | Disabled in training experiment by prior policy | Real team may gain additional damage/status | Moderate to high; must be explicit in benchmark config |
| Panic and secondary status interactions | Partial, sourced clauses only | Some Echoes Panic Type behavior and secondary effects are absent | Moderate |
| RNG | Deterministic substitute, replayable | Not the client's RNG | Relevant to distribution, not deterministic mechanics assertions |
| Targeting / multi-slot Engage | Action carries `(enemy UnitId, slot)` and resolution keys pull ownership by both | Full multi-enemy parity is tested; unusual same-slot collisions remain a future edge case | Critical; now covered for formal search |

## Gate status

This document describes scope, not a declaration that every gate is passed. See the pre-retrain checklist in the repair plan and the current tests. The post-audit Teacher/BC/PPO artifacts are current-format outputs of the documented critical gates and live in separately named paths. Existing checkpoints/reports outside those paths remain legacy; do not merge the datasets. A complete global unknown-rule inventory is still a scope limitation.
