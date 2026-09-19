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
| 14 | Fragile: +10% damage taken per Count (max 10) | `battle::fragile_bonus` | wiki.gg `Status Effects` | single_source_verified |
| 15 | Butterfly (unique Sinking): attacker heals `The Living / 4` SP on hit; turn end converts The Living into The Departed | `battle::apply_hit`, `battle::end_turn` | wiki.gg `Status Effects`, in-game `Bufs-walpu4` | multi_source_verified |
| 16 | The Living & The Departed (unique Ammo): consumed by skills, Potency+Count capped at 20 | `battle::apply_effects` (`spend_ammo`) | in-game `BattleKeywords-walpu4` | official |
| 17 | Shield is consumed before HP and expires at the start of the next turn | `state::Unit::take_damage`, `battle::begin_turn` | wiki.gg `Clash` / Shield | single_source_verified |
| 18 | **Dashboard**: each slot shows two skills — the usable one (bottom) and the already drawn follow-up (top); using the bottom skill consumes it, the top rotates down and a new one is drawn | `state::DashboardSlot`, `battle::rotate_used_slots` | JA-wiki スキル | multi_source_verified |
| 18a | The composition ("Skill Amount", typically 3/2/1) is shared by all of a unit's slots; new skills are drawn **randomly** from the copies not yet placed, and the counts reset once every copy has been placed | `state::SkillDeck::{draw, reset}` | JA-wiki スキル構成 | multi_source_verified |
| 18b | A slot whose skill was never used (target died, user staggered, skill cancelled) keeps its skills | `battle::resolve_combat` | JA-wiki スキル | single_source_verified |
| 18c | Defense skills and E.G.O replace the bottom skill; using one still consumes the replaced skill | `battle::submit`, `battle::rotate_used_slots` | JA-wiki 守備スキル・E.G.Oの生成 | single_source_verified |
| 18d | Extra Skill Slots: from turn 2, the Sinner lowest in Deployment Order without an extra slot gets one, until the encounter maximum (6 in Focused Encounters) | `battle::begin_turn` | EN-wiki `Battles` / Deployment Order | single_source_verified |
| 18e | Defense skills: **Guard** gains Shield equal to its Final Power *when the unit is first attacked that turn* (and does nothing if the unit is never attacked), and replaces the unit's Defense Level with the skill's for the turn; **Evade** flips against every incoming coin, negates it when equal or higher and is lost when it fails; **Counter** strikes back at the attacker | `battle::ActiveDefense`, `battle::activate_guard`, `battle::active_defense_level` | EN-wiki `Battles` / Defense Skills + JA-wiki 守備スキル | multi_source_verified |
| 19 | Attack skills generate 1 E.G.O resource of their affinity on use | `battle::prepare_use` | wiki.gg `Clash` / Attack Skills | single_source_verified |
| 20 | E.G.O costs its listed resources and SP; Overclock uses the corrosion skill at 1.5x cost (rounded up) | `battle::pay_ego`, `battle::ego_affordable` | wiki.gg `Clash` / E.G.O Skills, Overclocking | single_source_verified |
| 21 | Unit HP `= Base + Mult x Level`; speed is rolled from the unit's range each turn | `library::IdentityStats::hp_at_level`, `battle::begin_turn` | wiki.gg `Clash` / Health, Speed | single_source_verified |
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
| Imago three-state machine (`In the Past / Present / Future`) and its skill rotations | `NOT_IMPLEMENTED` | The wiki lists rotations in a notation this project could not disambiguate (lines of 4–6 skills under "three-turn cycle"). The rotation text is preserved in `data/_raw/pages/boss_imago.wikitext`. The enemy stands in with `BattleConfig::enemy_policy` (`Cyclic` by default: walks the unit's skill list in order). |
| Illusory Butterfly damage transfer and stack removal | `NOT_IMPLEMENTED` | Data (`9564/9565/9566`) is loaded, the passive text is kept, but the interaction is not simulated. |
| Section 5 choice event (Sunset Wayfarer) | `NOT_IMPLEMENTED` | Event data is not in the library yet. |
| Sin Resonance / A-Reson | `NOT_IMPLEMENTED` | Several effects ("highest Reson.", "A-Reson.") are listed in the per-skill `unmodeled` arrays. |
| Panic / Low Morale, E.G.O Corrosion forced actions | `NOT_IMPLEMENTED` | Sanity is modelled, the panic tables are not loaded. |
| Focused-encounter Parts (Core/Part splitting, part destruction) | `NOT_IMPLEMENTED` | Only the core unit is instantiated. |
| Multiple enemy skill slots | `NOT_IMPLEMENTED` | One slot per enemy; a clash only happens with the unit the enemy targeted. |
| E.G.O passives, E.G.O gifts, Observation Level | `NOT_IMPLEMENTED` | Observation Level is 0 in the game itself (wiki.gg `Damage`). |

`Simulator::unknown_rules()` and `Simulator::strict_blockers()` return these
lists at runtime; `python/demo.py --strict` prints them.
