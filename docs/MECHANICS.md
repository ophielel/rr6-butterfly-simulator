# Rule book — what the simulator implements, and where it comes from

Every rule below names its source and its verification status.  Status values
follow the project plan: `official`, `multi_source_verified`,
`single_source_verified`, `video_verified`, `unknown`, `synthetic`.
`REAL` mode refuses `unknown` and `synthetic` records.

Sources used below:

* **EN-wiki** = `limbuscompany.wiki.gg` (`Battles`, `Damage`, `Status Effects`)
* **JA-wiki** = `wikiwiki.jp/lcbwiki` → `Wiki管理/戦闘指南/戦闘システム詳細` and its
  siblings (`守備スキル`, `威力`, `破壊不能コイン`, `ダメージ計算`), cached in
  `data/_raw/pages/ja_wiki_*.txt` and re-fetchable with `tools/fetch_jawiki.py`
* **game** = in-game localisation dump (official text, keyed by internal id)

| # | Rule | Implementation | Source | Status |
|---|------|----------------|--------|--------|
| 1 | Damage = `CoinRoll x (1 + Static) x (1 + Dynamic)`, floored, min 1, min 5% of coin roll | `damage::compute_damage` | wiki.gg `Damage` | single_source_verified |
| 2 | Static = sin res + type res + off/def level + crit + `clash_count x 0.03` + observation (always 0) | `damage::compute_damage` | wiki.gg `Damage` | single_source_verified |
| 3 | Resistance → modifier: `x<0 ⇒ -0.5`, `0≤x<1 ⇒ (x-1)/2`, `x≥1 ⇒ x-1` | `damage::resistance_modifier` | wiki.gg `Damage`, `Clash` | single_source_verified |
| 4 | Off/Def level advantage = `(Off-Def)/(|Off-Def|+25)` | `damage::offense_defense_advantage` | wiki.gg `Damage` | single_source_verified |
| 5 | Clash power from levels: +1 per 3 levels, rounded down | `damage::level_clash_bonus` | wiki.gg `Clash` | single_source_verified |
| 6 | **Clash power is the sum over all coins**: both sides toss *every* remaining coin; `Final Power = Base Power + Σ(Coin Power of Heads coins)`; Match Power adds `+1 per 3 levels` for the higher skill level; resistances, defense level, attack type and sin affinity never take part | `battle::final_power`, `battle::match_power` | EN-wiki `Battles` + JA-wiki マッチの仕組み | multi_source_verified |
| 6a | Clash resolution: on a difference the lower side destroys **its first remaining coin**; on a tie **nothing** is destroyed and the clash continues; the side with coins left wins and attacks with them | `battle::resolve_clash`, `battle::destroy_first_coin` | JA-wiki マッチの仕組み | multi_source_verified |
| 6b | Attack damage accumulates: `Base Power + Coin Power` for each Heads coin, and every coin deals the power accumulated so far (4+4 ×3 → 8, 12, 16 = 36; 1+6 ×5 → 7…31 = 95) | `battle::one_sided_attack` | JA-wiki 攻撃 | multi_source_verified |
| 7 | Unbreakable coins become *Cracked* instead of destroyed and attack after the clash with coin power fixed to 1 (+1 for plus coins) | `battle::break_coin`, `battle::effective_coin_power` | wiki.gg `Clash` / Unbreakable Coins | single_source_verified |
| 7a | Paralyze: when a coin is tossed its Coin Power becomes 0 and the Paralyze count is consumed once per tossed coin | `battle::toss_all`, `battle::toss_single` | JA-wiki 麻痺 | single_source_verified |
| 8 | Stagger: thresholds are % of max HP; crossing one staggers for the current and next turn; levels give x2 / x2.5 / x3 to the type modifier | `state::StaggerState`, `battle::check_stagger` | wiki.gg `Clash` / Stagger, `Damage` | single_source_verified |
| 9 | Heads chance `H = 50 + SP`, SP clamped to [-45, 45]; units without Sanity flip at 50% | `rng::heads_chance`, `state::Sanity` | wiki.gg `Sanity`, `Clash` | single_source_verified |
| 10 | Burn: turn end, fixed damage by Potency, then Count −1 | `battle::end_turn` | wiki.gg `Status Effects`, in-game text | multi_source_verified |
| 11 | Bleed: when tossing an attack coin, fixed damage by Potency, then Count −1 | `battle::tick_bleed` | wiki.gg `Status Effects`, in-game text | multi_source_verified |
| 12 | Sinking: when hit, SP damage by Potency then Count −1; non-SP units take Gloom damage instead | `battle::apply_sinking` | wiki.gg `Status Effects`, in-game text | multi_source_verified |
| 13 | Poise: on hit, Potency% chance of a crit (1.2x, static +0.2), Count −1 on success and at turn end | `battle::apply_hit`, `battle::end_turn` | wiki.gg `Status Effects`, `Damage` | multi_source_verified |
| 12a | Rupture: when hit, fixed damage by Potency, then Count −1 | `battle::apply_hit` | wiki.gg `Status Effects` / Rupture | single_source_verified |
| 14 | Fragile: +10% damage taken per Count (max 10) | `battle::incoming_damage_modifier` | wiki.gg `Status Effects` | single_source_verified |
| 14a | Protection: −10% damage taken per Count (max 10); `<Sin> Fragility`: +10% damage taken from that affinity per Count (max 10); `<Sin>/<Type> Resist Down`: +0.1 resistance per Count | `battle::incoming_damage_modifier`, `state::Unit::resist*` | wiki.gg `Status Effects` | single_source_verified |
| 14b | Damage Up / Damage Down: ±10% damage dealt per Count (max 10) | `state::Unit::outgoing_damage_modifier` | wiki.gg `Status Effects` | single_source_verified |
| 14c | Power Up/Down (all skills) and Attack Power Up/Down (attacks only) change Final Power by the status value; Plus Coin Boost / Minus Coin Drop change Coin Power | `battle::final_power`, `battle::effective_coin_power` | wiki.gg `Status Effects`, `Damage` | single_source_verified |
| 14d | Offense/Defense Level Up/Down change the unit's levels by their Potency | `state::Unit::offense_level`, `defense_level` | wiki.gg `Status Effects` | single_source_verified |
| 14e | Bind: Speed −Potency for the turn; Haste: Speed +Count | `battle::begin_turn` | wiki.gg `Status Effects` | single_source_verified |
| 15a | Tremor: when hit by a skill that bursts Tremor, raise the Stagger Threshold by Tremor Potency, then Count −1; at turn end Count −1 | `battle::apply_effects` (`tremor_burst`), `battle::end_turn` | wiki.gg `Status Effects` / Tremor, Tremor Burst | single_source_verified |
| 16a | Charge: Count −1 at the end of each turn | `battle::end_turn` | wiki.gg `Status Effects` / Charge | single_source_verified |
| 15 | Butterfly (unique Sinking): attacker heals `The Living / 4` SP on hit; turn end converts The Living into The Departed | `battle::apply_hit`, `battle::end_turn` | wiki.gg `Status Effects`, in-game `Bufs-walpu4` | multi_source_verified |
| 16 | The Living & The Departed (unique Ammo): consumed by skills, Potency+Count capped at 20 | `battle::apply_effects` (`spend_ammo`) | in-game `BattleKeywords-walpu4` | official |
| 17 | Shield is consumed before HP and expires at the start of the next turn | `state::Unit::take_damage`, `battle::begin_turn` | wiki.gg `Clash` / Shield | single_source_verified |
| 18 | **Dashboard**: each slot shows two *selectable* skills (both usable this turn) plus one faint preview above them; using a skill consumes it and the rest rotate up, drawing a new preview | `state::DashboardSlot`, `battle::rotate_used_slots` | JA-wiki チェーンパネル ("ハッキリと見える2つのスキルとそれらの上に薄らと見える1つのスキル") | multi_source_verified |
| 18f | **Discard** deletes a selectable Skill from the Dashboard (never the preview) and the panel refills; "[Discard] N Skills of the lowest rank" and "if the other Skill in the same Skill Slot is different, [Discard] it" are implemented | `battle::discard_from_slot`, `battle::discard_lowest_rank` | wiki.gg `Status Effects` / Discard + skill text | single_source_verified |
| 18a | The composition ("Skill Amount", typically 3/2/1) is shared by all of a unit's slots; new skills are drawn **randomly** from the copies not yet placed, and the counts reset once every copy has been placed | `state::SkillDeck::{draw, reset}` | JA-wiki スキル構成 | multi_source_verified |
| 18b | A slot whose skill was never used (target died, user staggered, skill cancelled) keeps its skills | `battle::resolve_combat` | JA-wiki スキル | single_source_verified |
| 18c | Defense skills and E.G.O replace the bottom skill; using one still consumes the replaced skill | `battle::submit`, `battle::rotate_used_slots` | JA-wiki 守備スキル・E.G.Oの生成 | single_source_verified |
| 18d | Extra Skill Slots: from turn 2, the Sinner lowest in Deployment Order without an extra slot gets one, until the encounter maximum (6 in Focused Encounters) | `battle::begin_turn` | EN-wiki `Battles` / Deployment Order | single_source_verified |
| 18e | Defense skills: **Guard** gains Shield equal to its Final Power *when the unit is first attacked that turn* (and does nothing if the unit is never attacked), and replaces the unit's Defense Level with the skill's for the turn; **Evade** flips against every incoming coin, negates it when equal or higher and is lost when it fails; **Counter** strikes back at the attacker | `battle::ActiveDefense`, `battle::activate_guard`, `battle::active_defense_level` | EN-wiki `Battles` / Defense Skills + JA-wiki 守備スキル | multi_source_verified |
| 19 | Attack skills generate 1 E.G.O resource of their affinity on use | `battle::prepare_use` | wiki.gg `Clash` / Attack Skills | single_source_verified |
| 20 | E.G.O costs its listed resources and SP; Overclock uses the corrosion skill at 1.5x cost (rounded up) | `battle::pay_ego`, `battle::ego_affordable` | wiki.gg `Clash` / E.G.O Skills, Overclocking | single_source_verified |
| 21 | Unit HP `= Base + Mult x Level`; speed is rolled from the unit's range each turn | `library::IdentityStats::hp_at_level`, `battle::begin_turn` | wiki.gg `Clash` / Health, Speed | single_source_verified |
| 23 | **Imago action pattern**: six Skill Slots, a three-turn cycle, small/mid/big skill per state of time, different patterns below 66% and 33% HP, idle slots on turn 3, and the unit acts even while Staggered | `scripts::EnemyScript`, `battle::enemy_turn_skills` | wiki.gg Imago `Behavior` + JA-wiki 行動パターン | multi_source_verified |
| 24 | **States of time**: encounter starts with 10 Stacks of each; Turn Start activates the highest Stack (ties keep the current state); crossing 66%/33% HP grants 10 more of each once; switching adds 1 Temporal Disjunction | `battle::update_time_state` | wiki.gg Imago passives, in-game `Bufs_Refraction6` | multi_source_verified |
| 25 | **Stack bonus**: 0-10 → +1 Potency/Count; 11-20 → Clash Power +1, +2 Potency/+1 Count; 21-30 → Final Power +2, +3 Potency/+2 Count, applied to the state's status (Burn / Poise / Bleed) | `scripts::TimeState::stack_bonus`, `battle::time_state_bonus` | in-game `Bufs_Refraction6` (`StackPast/Present/FutureActivate`) | official |
| 26 | **Past**: hitting the Imago burns the attacker (1 Burn, +1 Count); `Kalpāgni` +2 Final Power and its last Coin deals (Stack/2) SP damage; Turn Start inflicts 5 HP Healing Down + 3 Wrath Fragility on Sinners with 10+ (Burn Potency + Count) | `battle::time_passives` | wiki.gg Imago passive `Past [過去]` | single_source_verified |
| 27 | **Present**: `Smite the Wicked` +2 Final Power and its last Coin raises the Stagger Threshold by (Stack/2); Turn Start gains (2 + #Sinners) Poise Potency and Count; no Poise Count loss on crit; critical hits deal +((Poise Potency + Count) x 2)% (max 120%) | `battle::time_passives` | wiki.gg Imago passive `Present [現在]` | single_source_verified |
| 28 | **Future**: `Bloodflower` +2 Final Power and its last Coin heals (Stack x 3) HP; Turn Start gives all Sinners +(3 + turn/2) Bleed Count; Bleed damage heals the Imago | `battle::time_passives` | wiki.gg Imago passive `Future [未來]` | single_source_verified |
| 32 | Effects with a "(N times per turn)" / "(N times per Encounter)" limit are counted; "next turn" grants are queued and applied at the next Turn Start; "At N+ [X], consume M [X] to deal +K% damage" spends the status and adds damage; "Gain Shield equal to (SP / N)% of max HP" and "for every [X] on self, gain Shield" compute Shields | `battle::apply_effects`, `state::Unit::{turn_effect_usage, pending_next_turn}` | skill effect text + wiki.gg `Status Effects` | single_source_verified |
| 33 | Defense-skill retaliation: "When hit while this unit has Shield, inflict N [X] against the attacker" fires while the Shield holds | `battle::apply_hit`, `state::RetaliateOnHit` | wiki.gg skill text | single_source_verified |
| 34 | Enemy targeting prefers Sinners with `Aggro`, then the fastest | `battle::enemy_targets` | wiki.gg `Status Effects` / Aggro | single_source_verified |
| 43 | Status grants respect the status's structure: Stack-based statuses (In the Past/Present/Future, Temporal Disjunction, Dazzle, Faint Aroma, Lamp, Aggro, Blue Sand, …) are written to Stack, Potency/Count statuses to Potency/Count; the structure is derived per status in `tools/build_library.py` (game text + wiki text + explicit overrides) | `data/statuses/statuses.json`, `battle::apply_effects` | in-game `Bufs*` text, wiki.gg `Status Effects` | multi_source_verified |
| 42 | Imago clauses: "Target cannot be Staggered until this Skill's Attack End", "[Hit after Clash Lose]" gates (the skill must have lost the Clash), "At less than N% HP, ...", "Deal +([In the Past/Present/Future] x N)% damage", "Deal +([X] Count on self)% damage", "If this unit has 20+ [X] Potency, consume up to 20 surplus ... to deal +(consumed x N)% damage", "If target is killed, Reuse this Skill on the target that has the highest HP (once per turn)", "treat the target's Pierce Resist. as Weak (1.5)" | `battle::{apply_hit, one_sided_attack, apply_effects}` | wiki.gg Imago skill text | single_source_verified |
| 40 | **Stations 2-4 (illusory butterflies)**: 1 HP, 333 Shield on the first Turn Start; Eclosion is Unclashable/Target Fixed, deals 0 damage and ends the Encounter — unless the Shield broke, in which case the stage runs one more turn (the choice event) before ending; **Segmentation** removes 1 Stack from the Imago in the campaign per hit as a main target (once per Coin), heals the attacker 10 SP (once per turn per Sinner) and grants the Imago +5 Stacks at Combat End if the unit was never hit | `state::CampaignState`, `battle::apply_hit`, `battle::end_turn`, `scripts::Segmentation` | wiki.gg Illusory Butterfly pages + JA-wiki part info | single_source_verified |
| 41 | **Campaign**: station 5 starts from the Pupa's remaining HP (excluding Shield) and 10 Stacks of each state; the earlier choice events disable components of the Past/Present/Future passives (`Simulator::section5`, `CampaignState::disabled_passives`) | `Simulator::section5`, `battle::time_passives` | JA-wiki station-5 notes, wiki.gg choice events | single_source_verified |
| 39 | Boss scaling clauses: "Final Power/Clash Power +N for every M [X] on the main target / on self (max K)", "Deal +(In the Past/Present/Future x N)% damage", "Halve [In the Past]", "while clashing the main target's Bleed Count does not drop below 1" | `battle::apply_effects`, `battle::tick_bleed_with_floor` | wiki.gg Imago skill text | single_source_verified |
| 36 | Ammo- and spent-resource scaling: "Base Power +N for every [X] about to be spent by this Skill", "Deal +([X] spent x N)% damage", "At N+ [X], consume up to M [X]" (the consumed amount feeds the damage bonus) | `battle::prepare_use`, `battle::apply_effects` | skill effect text | single_source_verified |
| 37 | Critical modifiers: "If target's SP is below 0, boost crit chance proportional to target's SP", "If user has N+ [X] Count, +M% Critical Damage" | `battle::apply_hit`, `battle::apply_effects` | skill effect text + wiki.gg `Damage` / Critical Hits | single_source_verified |
| 38 | Self-inflicted Stagger Threshold reduction ("Lower user's Stagger Threshold by N% of damage dealt") and `<X> Resist Down`/`<Sin> Fragility` counting | `battle::apply_hit` | skill effect text, wiki.gg `Status Effects` | single_source_verified |
| 35 | **Sin Resonance**: 2+ Skills of the same affinity on the Dashboard; **Absolute Resonance** = 3+ *consecutive* same-affinity Skills; separate A-Reson chains count separately; A-Reson also counts as Reson; skill effects read the highest resonance ("Gain (highest Reson.)", "4+ A-Reson") | `battle::compute_resonance`, `battle::highest_resonance`, `battle::gain_from_resonance` | wiki.gg `Resonance` + JA-wiki 罪悪共鳴 | multi_source_verified |
| 30 | **Station 1 (the Pupa)**: six Skill Slots, turns 1-3 = 3x Fluttering Havoc + 3x Pulverization (turn 3 adds The Quickening); encounter start grants 1.3% of max HP as Shield and its HP never falls below 90%; if the Shield is fully consumed before the end of turn 3 it plays pattern a next turn (Entangled Life x3 + The Quickening) | `data/mechanics/enemy_scripts.json`, `state::Unit::take_damage` (`hp_floor_percent`), `battle::enemy_turn_skills` | wiki.gg Pupa page + JA-wiki 行動パターン | multi_source_verified |
| 31 | "End the Encounter" skills (The Quickening, Eclosion) finish the station: `Winner::EncounterEnded`, not a wipe; "This Attack Skill deals 0 damage" and "Does not take damage for this turn" are honoured | `battle::ends_encounter`, `battle::apply_effects` (`zero_damage`, `no_damage_taken`) | wiki.gg Pupa / Illusory Butterfly pages | single_source_verified |
| 29 | Temporal Disjunction: take +(Stack x 15)% damage (max 150%); Turn Start at 10 Stack grants 5 Fragile | `battle::temporal_disjunction_bonus`, `battle::begin_turn` | in-game `Bufs_Refraction6` (`TimeGap`) | official |
| 22 | Boss numbers (HP, resistances, coin power, effect text) for the fixed content | `data/enemies/*.json`, `data/identities/*.json` | wiki.gg pages + in-game text keyed by official id | multi_source_verified |

## Unknown rules (explicitly *not* guessed)

| Rule | Handling |
|------|----------|
| SP gained/lost per clash | The wiki documents that the base factors changed on 2023-06-01 but not the current values → `BattleConfig::sp_on_clash_win/lose = None`, which is treated as 0 **and logged as an unresolved rule**. Set the fields to change it. |
| Client RNG algorithm | Not observable from the permitted sources → the simulator uses a documented xoshiro256** (`rng.rs`); only the *probability model* comes from the wiki. Marked `synthetic`. |
| Ammo split (The Living vs The Departed) | In-game text only says "randomly determined" → 50/50, recorded as an assumption in `data/mechanics/effects.json` (`"assumption": "random"`). |
| Which coin a clash destroys | The JA-wiki says "1コイン目から順に一つ破壊" (from the first coin in order) → implemented as *the first surviving coin*. |
| Coin flips during the winner's attack | The final clash round already tossed every coin; the attack reuses those results (the accumulated power of the last coin equals the Final Power the clash used). |

## Not implemented (and why)

| Feature | Status | Note |
|---------|--------|------|
| Illusory Butterfly damage transfer ("Origination") | `NOT_IMPLEMENTED` | Out of scope: the deliverable is the Section 5 Imago; the butterfly data and its Segmentation passive exist for completeness but the campaign is optional (`Simulator::section5`). |
| Section 5 choice event (Sunset Wayfarer) and the earlier stations' choices that disable Past/Present/Future components | `NOT_IMPLEMENTED` | The encounter starts in `BattleConfig::initial_time_state` instead (the wiki ties the real starting state to those choices). |
| HP Healing Down / Wrath Fragility / Gloom Fragility etc. | `NOT_IMPLEMENTED` | The Past passive and several skills apply them and they show up in the state, but their own effects (healing reduction, damage amplification by affinity) are not yet part of the damage formula. |
| Illusory Butterfly Eclosion / encounter-end skills | `NOT_IMPLEMENTED` | "End the Encounter" skills are extracted but not acted upon. |
| Stations 2-4 (the illusory butterflies): 1 HP + 333 Shield, unclashable Eclosion, "hit as main target knocks a Stack off the Imago" and the choice event that disables a passive component | data only | `data/enemies/9564`-`9566` and `9572`-`9574` are loaded with their passives; the encounter itself is not scripted yet (the Section 5 starting state is set through `BattleConfig::initial_time_state` instead). |
| Sin Resonance **level bonus** (the +1…+11 Offense/Defense Level table) | `NOT_IMPLEMENTED` | The resonance *counts* are implemented; the level-gain table on wiki.gg is malformed in the fetched markup (rows have fewer cells than columns), so the exact gain per chain length is not applied. |
| Panic / Low Morale, E.G.O Corrosion forced actions | `NOT_IMPLEMENTED` | Sanity is modelled, the panic tables are not loaded. |
| Focused-encounter Parts (Core/Part splitting, part destruction) | `NOT_IMPLEMENTED` | Only the core unit is instantiated. |
| Multiple enemy skill slots | `NOT_IMPLEMENTED` | One slot per enemy; a clash only happens with the unit the enemy targeted. |
| E.G.O passives, E.G.O gifts, Observation Level | `NOT_IMPLEMENTED` | Observation Level is 0 in the game itself (wiki.gg `Damage`). |

`Simulator::unknown_rules()` and `Simulator::strict_blockers()` return these
lists at runtime; `python/demo.py --strict` prints them.
