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
    /// `hp_below_percent` includes equality (the wiki writes "N% or less").
    #[serde(default)]
    pub hp_or_equal: Option<bool>,
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

    /// Skills referenced by the fixed content that are missing from the book.
    pub fn missing(&self, ids: &[SkillId]) -> Vec<SkillId> {
        ids.iter().filter(|id| !self.skills.contains_key(id.as_str())).cloned().collect()
    }
}
