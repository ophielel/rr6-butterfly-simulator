//! Skill mechanics: the machine-readable description of what a skill's effect
//! text does, plus the evaluator for its conditions.
//!
//! Provenance: every entry in `data/mechanics/effects.json` is derived from the
//! skill's own effect text (`raw` keeps the source line).  Effect lines that the
//! extractor could not translate into one of the supported kinds are kept in
//! `unmodeled`; the engine reports them and strict mode refuses to run a skill
//! whose `unmodeled` list is non-empty.

use crate::ids::SkillId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Which value of a status is being read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Component {
    Potency,
    Count,
    Stack,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Condition {
    /// `self` or `target` (defaults to `self`).
    #[serde(default)]
    pub source: Option<String>,
    /// Single status read as `component`.
    #[serde(default)]
    pub status: Option<String>,
    /// Several statuses summed together; listing the same key twice sums both
    /// of its components (the wiki's "both [Butterfly]" notation).
    #[serde(default)]
    pub statuses: Vec<String>,
    #[serde(default)]
    pub component: Option<Component>,
    #[serde(default)]
    pub gte: Option<i32>,
    #[serde(default)]
    pub lte: Option<i32>,
    #[serde(default)]
    pub hp_below_percent: Option<i32>,
    #[serde(default)]
    pub clash_count_gte: Option<i32>,
    /// True when several conditions are alternatives ("If any of the following
    /// conditions are met").
    #[serde(default)]
    pub any_of: Vec<Condition>,
    #[serde(default)]
    pub self_speed_at_most: Option<i32>,
    #[serde(default)]
    pub target_speed_advantage: Option<i32>,
    #[serde(default)]
    pub self_sp_at_least: Option<i32>,
    /// The actor's own SP must be below this ("at less than 0 SP").
    #[serde(default)]
    pub self_sp_below: Option<i32>,
    /// Any sin's Absolute Resonance chain is at least this long.
    #[serde(default)]
    pub a_reson_gte: Option<i32>,
    /// The resonance that triggered this entry must be an Absolute Resonance.
    #[serde(default)]
    pub requires_a_reson: bool,
    /// Any sin's Resonance count is at least this value.
    #[serde(default)]
    pub resonance_gte: Option<i32>,
    /// The Resonance count of one specific sin must be at least `resonance_gte`
    /// ("At 3+ (Gloom Reson.)").
    #[serde(default)]
    pub resonance_of: Option<String>,
    /// The target's SP is below this value.
    #[serde(default)]
    pub target_sp_below: Option<i32>,
    /// `hp_below_percent` includes equality (the wiki writes "N% or less").
    #[serde(default)]
    pub hp_or_equal: Option<bool>,
    /// "If the target has more than N% HP" (also reads the actor when
    /// `source` is left at its `self` default).
    #[serde(default)]
    pub hp_above_percent: Option<i32>,
    /// "If the target is defeated" / "If target survives this attack".
    #[serde(default)]
    pub target_defeated: bool,
    #[serde(default)]
    pub target_survived: bool,
    /// "If target is a SP Unit" / "For targets that are Non-SP Units".
    #[serde(default)]
    pub target_is_sp_unit: bool,
    /// "If an enemy has any of the Panic type changing effects": the unit must
    /// carry at least one of the listed statuses.
    #[serde(default)]
    pub any_status: Vec<String>,
    /// "against targets in either Low Morale or Panic states".
    #[serde(default)]
    pub target_is_low_morale: bool,
    #[serde(default)]
    pub target_is_panicked: bool,
    #[serde(default)]
    pub target_is_non_sp_unit: bool,
    /// "If this unit has [X]" - every listed status must be present.
    #[serde(default)]
    pub has_status: Vec<String>,
    /// "If target is in an [Amplitude Conversion] or [Amplitude Entanglement]
    /// state".
    #[serde(default)]
    pub target_has_amplitude: bool,
    /// The listed status must be absent ("If target isn't in an [Amplitude
    /// Conversion] state").
    #[serde(default)]
    pub lacks_status: Vec<String>,
    /// "If 1 or more targets are killed" (by this Skill's use).
    #[serde(default)]
    pub any_target_killed: bool,
    /// "If this Skill was equipped on this unit's leftmost Skill Slot".
    #[serde(default)]
    pub slot: Option<String>,
    /// "When attacking just a single target" - the Skill use must target exactly
    /// one unit.
    #[serde(default)]
    pub single_target: bool,
    /// "against Staggered targets".
    #[serde(default)]
    pub target_staggered: bool,
}

/// A single mechanical effect.  The struct is deliberately loose: every kind
/// reads the fields it needs and ignores the rest, which keeps the generated
/// JSON readable and diff-friendly.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Effect {
    pub kind: String,
    #[serde(default)]
    pub raw: Option<String>,
    #[serde(default)]
    pub condition: Option<Condition>,

    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub status2: Option<String>,
    #[serde(default)]
    pub potency: Option<i32>,
    #[serde(default)]
    pub count: Option<i32>,
    /// Stack-based statuses (Blue Sand, In the Past, ...) fill Stack.
    #[serde(default)]
    pub stack: Option<i32>,
    #[serde(default)]
    pub value: Option<i32>,
    #[serde(default)]
    pub per: Option<i32>,
    /// Increment applied per whole `per` (defaults to 1).
    #[serde(default)]
    pub step: Option<i32>,
    #[serde(default)]
    pub max: Option<i32>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub component: Option<Component>,
    #[serde(default)]
    pub coins: Vec<u32>,
    #[serde(default)]
    pub percent: Option<i32>,
    #[serde(default)]
    pub times: Option<i32>,
    #[serde(default)]
    pub consume_count: Option<i32>,
    /// Usage limit per turn / per encounter.
    #[serde(default)]
    pub per_turn: Option<i32>,
    #[serde(default)]
    pub per_encounter: Option<i32>,
    /// The effect is applied at the start of the next turn.
    #[serde(default)]
    pub next_turn: bool,
    /// `consume_status_for_damage`: how much of the status must be present.
    #[serde(default)]
    pub threshold: Option<i32>,
    /// Explicit sin affinity for damage-dealing effects.
    #[serde(default)]
    pub sin: Option<String>,
    /// The clause only applies when the skill lost its Clash
    /// ("[Hit after Clash Lose]").
    #[serde(default)]
    pub only_after_clash_lose: bool,
    /// "[On Hit without Cracking]": the clause is skipped when its Coin was
    /// destroyed (cracked) in the Clash.
    #[serde(default)]
    pub only_without_cracking: bool,
    /// A named flag a phase turns on for this Skill use ("flag" kind).
    #[serde(default)]
    pub flag: Option<String>,
    /// `gain_from_resonance`: multiplier applied to the highest Resonance.
    #[serde(default)]
    pub multiplier: Option<i32>,
    /// Percentages that the wiki writes with a decimal ("+0.5% damage").
    #[serde(default)]
    pub step_f: Option<f64>,
    #[serde(default)]
    pub value_f: Option<f64>,
    /// A gain scaled by the actor's SP ("for every 8 SP (max 5)").
    #[serde(default)]
    pub per_sp: Option<i32>,
    /// The ammo pool an `inflict_equal_ammo_spent` clause reads.
    #[serde(default)]
    pub ammo: Option<String>,
    /// A `[min ~ max]` random amount (HP damage taken, status gained).
    #[serde(default)]
    pub range_min: Option<i32>,
    #[serde(default)]
    pub range_max: Option<i32>,
    /// Several effects written as one wiki line (resolved in order).
    #[serde(default)]
    pub sub_effects: Vec<Effect>,
    /// The resonance that must be present for a per-Resonance scaling, and the
    /// status whose amount the scaling reads.
    #[serde(default)]
    pub resonance_of: Option<String>,
    #[serde(default)]
    pub from_resonance: bool,
    #[serde(default)]
    pub per_ammo: bool,
    /// "For 2 turns, lose 8 SP at Combat End": how many Combat Ends.
    #[serde(default)]
    pub turns: Option<i32>,
    /// The Coin this clause belongs to (filled by the extractor / attack loop).
    #[serde(default)]
    pub coin_index: Option<u32>,
    /// `ally_count`/heal targets taken from a status on the actor:
    /// "([Bind] on self / 3) other allies".
    #[serde(default)]
    pub ally_from_status: Option<String>,
    #[serde(default)]
    pub ally_from_divisor: Option<i32>,
    /// The resonance that produced this effect must be an Absolute Resonance.
    #[serde(default)]
    pub requires_a_reson: bool,
    #[serde(default)]
    pub a_reson_gte: Option<i32>,
    /// "50% chance to ..." - a coin flip taken from the replayable RNG stream.
    #[serde(default)]
    pub chance: Option<i32>,
    /// "Gain [X] up to N Stack" - the gain stops at N.
    #[serde(default)]
    pub up_to: Option<i32>,
    /// Which units a Heal / SP Heal / gain reaches.  `self` (default),
    /// `all_allies`, `lowest_hp`, `lowest_sp`, `slowest`, `random`.
    #[serde(default)]
    pub ally: Option<String>,
    #[serde(default)]
    pub ally_count: Option<i32>,
    /// "self and N other allies" / "N other allies" - whether the actor is
    /// part of an `ally` selection.
    #[serde(default)]
    pub include_self: Option<bool>,
    /// "take HP damage equal to 1% of max HP" (self-inflicted downsides).
    #[serde(default)]
    pub self_damage_percent: Option<i32>,
    #[serde(default)]
    pub self_damage_min: Option<i32>,
    #[serde(default)]
    pub self_damage_max: Option<i32>,
    /// "this effect does not reduce this unit's HP below 1" /
    /// "does not get Staggered due to this effect".
    #[serde(default)]
    pub hp_floor_one: bool,
    #[serde(default)]
    pub no_stagger: bool,
    /// "[On Hit] Lose 2~6 SP" / "lose 15 SP".
    #[serde(default)]
    pub self_sp_damage: Option<i32>,
    #[serde(default)]
    pub self_sp_damage_min: Option<i32>,
    #[serde(default)]
    pub self_sp_damage_max: Option<i32>,
    /// "regain half of [X] consumed by this Skill" - percentage of the amount
    /// the skill consumed.
    #[serde(default)]
    pub refund_percent: Option<i32>,
    /// "trigger [Amplitude Conversion] into [Tremor - Decay]".
    #[serde(default)]
    pub amplitude_into: Option<String>,
    /// "Deal Gloom damage equal to (N)% of this Coin's final damage".
    #[serde(default)]
    pub final_damage_percent: Option<i32>,
    /// "[Butterfly](The Departed)" / "(The Living)" - which half of the
    /// unique Sinking is inflicted.
    #[serde(default)]
    pub butterfly_part: Option<String>,
    /// A status rider that inflicts on the attacker instead of the target.
    #[serde(default)]
    pub on_attacker: bool,
    /// "[Reuse - On Hit]": the clause only resolves on a Reused Coin (wiki.gg
    /// `Clash`, trigger table).
    #[serde(default)]
    pub reuse_only: bool,
    /// "Reuse this Coin ([X] Potency - N) times (max K)": the count comes from a
    /// status on the actor.
    #[serde(default)]
    pub reuse_from_status: Option<String>,
    #[serde(default)]
    pub reuse_minus: Option<i32>,
    /// A status rider that only fires on the first Coin of a Skill.
    #[serde(default)]
    pub first_coin_only: bool,
    /// "On Hit with a Base Attack Skill" (a passive that rides on hits).
    #[serde(default)]
    pub on_base_attack_hit: bool,
    /// A Panic-Type clause that resolves at Turn End rather than Turn Start.
    #[serde(default)]
    pub trigger_turn_end: bool,
    /// "Deal more damage based on missing HP on self (max 15%)": percent at
    /// 100% HP lost equals this value.
    #[serde(default)]
    pub damage_percent_missing_hp: Option<i32>,
    /// "[Before Attack] At 3+ (Gloom Reson.), Atk Weight +1".
    #[serde(default)]
    pub attack_weight: Option<i32>,
    /// "Then, Reuse this Coin (N times per Skill)".
    #[serde(default)]
    pub reuse_coin: Option<i32>,
    /// "If target was killed, activate the effect above once more".
    #[serde(default)]
    pub repeat_on_kill: bool,
    /// "Heal SP equal to [X] Potency on target (Max SP heal: N)".
    #[serde(default)]
    pub heal_from_status: Option<String>,
    #[serde(default)]
    pub heal_from_component: Option<Component>,
    #[serde(default)]
    pub heal_from_divisor: Option<i32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillMechanics {
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub on_use: Vec<Effect>,
    #[serde(default)]
    pub combat_start: Vec<Effect>,
    #[serde(default)]
    pub clash_win: Vec<Effect>,
    #[serde(default)]
    pub clash_lose: Vec<Effect>,
    #[serde(default)]
    pub attack_end: Vec<Effect>,
    #[serde(default)]
    pub turn_start: Vec<Effect>,
    #[serde(default)]
    pub turn_end: Vec<Effect>,
    /// Continuous clauses ("Deal +N% damage for every [X] on self").
    #[serde(default)]
    pub passive: Vec<Effect>,
    /// "On Tails Hit" clauses.
    #[serde(default)]
    pub tails_hit: Vec<Effect>,
    /// "[When Clash ends]" clauses of a status.
    #[serde(default)]
    pub clash_end: Vec<Effect>,
    /// "[When hit]" clauses of a status.
    #[serde(default)]
    pub on_hit: Vec<Effect>,
    /// "[Before Attack]" - resolved after On Use and before the first toss.
    #[serde(default)]
    pub before_attack: Vec<Effect>,
    /// "[Heads Hit]" - resolved in addition to the coin's On Hit effects when
    /// that coin lands on Heads.
    #[serde(default)]
    pub heads_hit: BTreeMap<String, Vec<Effect>>,
    /// "[On Target Kill]" / "[On Kill]" - resolved when a Coin kills.
    #[serde(default)]
    pub on_kill: Vec<Effect>,
    /// "[On Evade]" - resolved when this defense skill evades.
    #[serde(default)]
    pub on_evade: Vec<Effect>,
    /// coin index (1-based, as string) -> effects resolved when that coin lands.
    #[serde(default)]
    pub coins: BTreeMap<String, Vec<Effect>>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Effect-text lines that this project does **not** model.
    #[serde(default)]
    pub unmodeled: Vec<String>,
}

impl SkillMechanics {
    /// "[Heads Hit]" effects of a coin (resolved on top of its On Hit ones).
    pub fn heads_hit(&self, index: u32) -> &[Effect] {
        self.heads_hit
            .get(&index.to_string())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn coin(&self, index: u32) -> &[Effect] {
        self.coins
            .get(&index.to_string())
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn is_complete(&self) -> bool {
        self.unmodeled.is_empty()
    }
}

/// A passive ("Combat Passive" / "Support Passive") of an identity or enemy.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Passive {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub owner: String,
    /// `combat` for the unit's own passive, `support` for the team passive.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub desc: Option<String>,
    /// Phrases that stay UNKNOWN for this project.
    #[serde(default)]
    pub unmodeled: Vec<String>,
    #[serde(default)]
    pub effects: SkillMechanics,
}

impl Passive {
    pub fn is_complete(&self) -> bool {
        self.effects.unmodeled.is_empty() && self.unmodeled.is_empty()
    }
}

/// One row of the wiki's `Sanity` panic table.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PanicType {
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub low_morale: Vec<Effect>,
    #[serde(default)]
    pub panic: Vec<Effect>,
    #[serde(default)]
    pub low_morale_text: String,
    #[serde(default)]
    pub panic_text: String,
    #[serde(default)]
    pub unmodeled: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PanicBook {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub types: BTreeMap<String, PanicType>,
    /// identity id -> panic type name (defaults to `Panic`).
    #[serde(default)]
    pub identities: BTreeMap<String, String>,
}

impl PanicBook {
    pub fn load(path: &Path) -> Result<PanicBook, String> {
        if !path.exists() {
            return Ok(PanicBook::default());
        }
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn for_identity(&self, identity: &str) -> Option<&PanicType> {
        let name = self.identities.get(identity)?;
        self.types.get(name)
    }

    /// Every Panic clause this project has not modelled.
    pub fn gaps(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (name, entry) in &self.types {
            for clause in &entry.unmodeled {
                out.push(format!("panic type {name}: {clause}"));
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

/// The behaviour of one status, parsed from its own text.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StatusBehaviour {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub effects: SkillMechanics,
    #[serde(default)]
    pub max_stack: Option<i32>,
    /// The component a plain "Gain N [X]" fills (`potency` or `count`).
    #[serde(default)]
    pub primary: Option<String>,
    /// `potency_count`, `stack` or `single` (a status with only one value, e.g.
    /// the Charge-like resources).
    #[serde(default)]
    pub structure: Option<String>,
    /// When the status stops existing: `either_zero` (a double-value status is
    /// removed once either value reaches 0, wiki.gg `Status Effects`),
    /// `count_zero`, `potency_zero`, `both_zero` (Butterfly) or `none`.
    #[serde(default)]
    pub expiry: Option<String>,
    /// "for one turn" / "for this turn": removed at Turn End.
    #[serde(default)]
    pub expires_at_turn_end: bool,
    /// True when one of the fixed content's Skills references this status.
    #[serde(default)]
    pub used_by_fixed_content: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StatusBook {
    #[serde(default)]
    pub version: u32,
    /// Status display name -> behaviour.
    #[serde(default)]
    pub statuses: BTreeMap<String, StatusBehaviour>,
}

impl StatusBook {
    pub fn load(path: &Path) -> Result<StatusBook, String> {
        if !path.exists() {
            return Ok(StatusBook::default());
        }
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn get(&self, name: &str) -> Option<&StatusBehaviour> {
        self.statuses.get(name)
    }

    /// Status clauses this project does not model yet.
    pub fn gaps(&self, only: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        for (name, entry) in &self.statuses {
            if !only.is_empty() && !only.contains(&name.as_str()) {
                continue;
            }
            for clause in &entry.effects.unmodeled {
                out.push(format!("status {name}: {clause}"));
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PassiveBook {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub passives: BTreeMap<String, Passive>,
}

impl PassiveBook {
    pub fn load(path: &Path) -> Result<PassiveBook, String> {
        if !path.exists() {
            return Ok(PassiveBook::default());
        }
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Passives owned by a unit (its own combat passives).
    pub fn for_owner(&self, owner: &str) -> Vec<&Passive> {
        let mut found: Vec<&Passive> = self
            .passives
            .values()
            .filter(|p| p.owner == owner && p.kind == "combat")
            .collect();
        found.sort_by(|a, b| a.id.cmp(&b.id));
        // The higher id is the Uptie 4 version of the same passive; it replaces
        // the lower one.
        if let Some(last) = found.last() {
            let name = last.name.clone();
            let top = found
                .iter()
                .filter(|p| p.name == name)
                .map(|p| p.id.clone())
                .max();
            if let Some(top) = top {
                found.retain(|p| p.name != name || p.id == top);
            }
        }
        found
    }

    /// Support passives of the whole team.
    pub fn supports(&self) -> Vec<&Passive> {
        self.passives
            .values()
            .filter(|p| p.kind == "support")
            .collect()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MechanicsBook {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub skills: BTreeMap<String, SkillMechanics>,
}

impl MechanicsBook {
    pub fn load(path: &Path) -> Result<MechanicsBook, String> {
        if !path.exists() {
            return Ok(MechanicsBook::default());
        }
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn get(&self, id: &SkillId) -> Option<&SkillMechanics> {
        self.skills.get(id.as_str())
    }

    /// Mechanics for a skill at a given uptie tier.  Keys are `id@tier`; the
    /// lookup falls back to the highest tier and then to enemies (`@1`).
    pub fn get_for(&self, id: &SkillId, uptie: crate::ids::Uptie) -> Option<&SkillMechanics> {
        for tier in [uptie.0, 4, 1] {
            if let Some(found) = self.skills.get(&format!("{}@{tier}", id.as_str())) {
                return Some(found);
            }
        }
        self.skills.get(id.as_str())
    }

    pub fn get_or_default(&self, id: &SkillId) -> SkillMechanics {
        self.skills.get(id.as_str()).cloned().unwrap_or_default()
    }

    pub fn get_or_default_for(&self, id: &SkillId, uptie: crate::ids::Uptie) -> SkillMechanics {
        self.get_for(id, uptie).cloned().unwrap_or_default()
    }

    /// Mechanics of an E.G.O Skill.  E.G.O entries are keyed `<ego id>.<kind>`
    /// and stored with an uptie suffix (`<ego id>.awakening@1`), so an exact
    /// lookup by the bare id would miss them.
    pub fn get_ego(&self, ego_id: &str, kind: &str) -> SkillMechanics {
        let base = format!("{ego_id}.{kind}");
        if let Some(found) = self.skills.get(&base) {
            return found.clone();
        }
        for tier in [1u8, 2, 3, 4] {
            if let Some(found) = self.skills.get(&format!("{base}@{tier}")) {
                return found.clone();
            }
        }
        SkillMechanics::default()
    }

    /// Skills referenced by the fixed content that are missing from the book.
    pub fn missing(&self, ids: &[SkillId]) -> Vec<SkillId> {
        ids.iter().filter(|id| !self.skills.contains_key(id.as_str())).cloned().collect()
    }
}
