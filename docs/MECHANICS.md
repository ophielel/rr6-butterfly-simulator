# Rule book — what the simulator implements, and where it comes from

Every rule below names its source and its verification status.  Status values
follow the project plan: `official`, `multi_source_verified`,
`single_source_verified`, `video_verified`, `unknown`, `synthetic`.
`REAL` mode refuses `unknown` and `synthetic` records.

| # | Rule | Implementation | Source | Status |
|---|------|----------------|--------|--------|
| 1 | Damage = `CoinRoll x (1 + Static) x (1 + Dynamic)`, floored, min 1, min 5% of coin roll | `damage::compute_damage` | wiki.gg `Damage` | single_source_verified |
| 2 | Static = sin res + type res + off/def level + crit + `clash_count x 0.03` + observation (always 0) | `damage::compute_damage` | wiki.gg `Damage` | single_source_verified |
| 3 | Resistance → modifier: `x<0 ⇒ -0.5`, `0≤x<1 ⇒ (x-1)/2`, `x≥1 ⇒ x-1` | `damage::resistance_modifier` | wiki.gg `Damage`, `Clash` | single_source_verified |
| 4 | Off/Def level advantage = `(Off-Def)/(|Off-Def|+25)` | `damage::offense_defense_advantage` | wiki.gg `Damage` | single_source_verified |
| 5 | Clash power from levels: +1 per 3 levels, rounded down | `damage::level_clash_bonus` | wiki.gg `Clash` | single_source_verified |
| 6 | Clash: both sides toss the coin they are currently on, loser destroys that coin and clashes again; the survivor attacks with its remaining coins | `battle::resolve_clash` | wiki.gg `Clash` / Battle Information | single_source_verified |
| 7 | Unbreakable coins become *Cracked* instead of destroyed, attack after the clash with coin power fixed to 1 | `battle::break_coin`, `coin_power` | wiki.gg `Clash` / Unbreakable Coins | single_source_verified |
| 8 | Stagger: thresholds are % of max HP; crossing one staggers for the current and next turn; levels give x2 / x2.5 / x3 to the type modifier | `state::StaggerState`, `battle::check_stagger` | wiki.gg `Clash` / Stagger, `Damage` | single_source_verified |
| 9 | Heads chance `H = 50 + SP`, SP clamped to [-45, 45]; units without Sanity flip at 50% | `rng::heads_chance`, `state::Sanity` | wiki.gg `Sanity`, `Clash` | single_source_verified |
| 10 | Burn: turn end, fixed damage by Potency, then Count −1 | `battle::end_turn` | wiki.gg `Status Effects`, in-game text | multi_source_verified |
| 11 | Bleed: when tossing an attack coin, fixed damage by Potency, then Count −1 | `battle::tick_bleed` | wiki.gg `Status Effects`, in-game text | multi_source_verified |
| 12 | Sinking: when hit, SP damage by Potency then Count −1; non-SP units take Gloom damage instead | `battle::apply_sinking` | wiki.gg `Status Effects`, in-game text | multi_source_verified |
| 13 | Poise: on hit, Potency% chance of a crit (1.2x, static +0.2), Count −1 on success and at turn end | `battle::apply_hit`, `battle::end_turn` | wiki.gg `Status Effects`, `Damage` | multi_source_verified |
| 14 | Fragile: +10% damage taken per Count (max 10) | `battle::fragile_bonus` | wiki.gg `Status Effects` | single_source_verified |
| 15 | Butterfly (unique Sinking): attacker heals `The Living / 4` SP on hit; turn end converts The Living into The Departed | `battle::apply_hit`, `battle::end_turn` | wiki.gg `Status Effects`, in-game `Bufs-walpu4` | multi_source_verified |
| 16 | The Living & The Departed (unique Ammo): consumed by skills, Potency+Count capped at 20 | `battle::apply_effects` (`spend_ammo`) | in-game `BattleKeywords-walpu4` | official |
| 17 | Shield is consumed before HP and expires at the start of the next turn | `state::Unit::take_damage`, `battle::begin_turn` | wiki.gg `Clash` / Shield | single_source_verified |
| 18 | Skill deck: copies per skill from `Skill Amount`; the deck only refreshes after every card was used | `state::SkillDeck` | wiki.gg `Clash` / Skills | single_source_verified |
| 19 | Attack skills generate 1 E.G.O resource of their affinity on use | `battle::prepare_use` | wiki.gg `Clash` / Attack Skills | single_source_verified |
| 20 | E.G.O costs its listed resources and SP; Overclock uses the corrosion skill at 1.5x cost (rounded up) | `battle::pay_ego`, `battle::ego_affordable` | wiki.gg `Clash` / E.G.O Skills, Overclocking | single_source_verified |
| 21 | Unit HP `= Base + Mult x Level`; speed is rolled from the unit's range each turn | `library::IdentityStats::hp_at_level`, `battle::begin_turn` | wiki.gg `Clash` / Health, Speed | single_source_verified |
| 22 | Boss numbers (HP, resistances, coin power, effect text) for the fixed content | `data/enemies/*.json`, `data/identities/*.json` | wiki.gg pages + in-game text keyed by official id | multi_source_verified |

## Unknown rules (explicitly *not* guessed)

| Rule | Handling |
|------|----------|
| SP gained/lost per clash | The wiki documents that the base factors changed on 2023-06-01 but not the current values → `BattleConfig::sp_on_clash_win/lose = None`, which is treated as 0 **and logged as an unresolved rule**. Set the fields to change it. |
| Client RNG algorithm | Not observable from the permitted sources → the simulator uses a documented xoshiro256** (`rng.rs`); only the *probability model* comes from the wiki. Marked `synthetic`. |
| Clash tie resolution | Not documented → `BattleConfig::tie_rule`, default `BothLoseCoin`. |
| Ammo split (The Living vs The Departed) | In-game text only says "randomly determined" → 50/50, recorded as an assumption in `data/mechanics/effects.json` (`"assumption": "random"`). |

## Not implemented (and why)

| Feature | Status | Note |
|---------|--------|------|
| Imago three-state machine (`In the Past / Present / Future`) and its skill rotations | `NOT_IMPLEMENTED` | The wiki lists rotations in a notation this project could not disambiguate (lines of 4–6 skills under "three-turn cycle"). The rotation text is preserved in the raw wiki cache; `docs/DATA.md` explains how to supply a script instead. |
| Illusory Butterfly damage transfer and stack removal | `NOT_IMPLEMENTED` | Data (`9564/9565/9566`) is loaded, the passive text is kept, but the interaction is not simulated. |
| Section 5 choice event (Sunset Wayfarer) | `NOT_IMPLEMENTED` | Event data is not in the library yet. |
| Sin Resonance / A-Reson | `NOT_IMPLEMENTED` | Several effects ("highest Reson.", "A-Reson.") are listed in the per-skill `unmodeled` arrays. |
| Panic / Low Morale, E.G.O Corrosion forced actions | `NOT_IMPLEMENTED` | Sanity is modelled, the panic tables are not loaded. |
| Focused-encounter Parts (Core/Part splitting, part destruction) | `NOT_IMPLEMENTED` | Only the core unit is instantiated. |
| Multiple enemy skill slots | `NOT_IMPLEMENTED` | One slot per enemy; a clash only happens with the unit the enemy targeted. |
| E.G.O passives, E.G.O gifts, Observation Level | `NOT_IMPLEMENTED` | Observation Level is 0 in the game itself (wiki.gg `Damage`). |

`Simulator::unknown_rules()` and `Simulator::strict_blockers()` return these
lists at runtime; `python/demo.py --strict` prints them.
