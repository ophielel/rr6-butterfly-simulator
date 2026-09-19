//! Typed view over the generated data library (`data/identities`, `data/ego`,
//! `data/enemies`, `data/statuses`).
//!
//! The JSON is produced by `tools/build_library.py` from two independent
//! sources; each record keeps its provenance in `sources` and lists anything the
//! parser could not read in `unparsed`, which strict mode refuses to run.

use crate::ids::{DamageType, EgoId, EnemyId, IdentityId, Sin, SkillId, SkillSlot, Uptie};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SourceRecord {
    pub kind: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub verification: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PassiveRecord {
    pub kind: String,
    pub uptie_from: u8,
    pub text: String,
    #[serde(default)]
    pub raw_key: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillTier {
    #[serde(default)]
    pub base_power: Option<i32>,
    #[serde(default)]
    pub coin_power: Option<i32>,
    #[serde(default)]
    pub coins: Option<u32>,
    #[serde(default)]
    pub offense_level_mod: Option<i32>,
    #[serde(default)]
    pub attack_weight: Option<u32>,
    #[serde(default)]
    pub defense_level_mod: Option<i32>,
    #[serde(default)]
    pub sin_affinity: Option<String>,
    #[serde(default)]
    pub skill_amount: Option<u32>,
    #[serde(default)]
    pub on_use_text: String,
    #[serde(default)]
    pub coin_texts: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillRecord {
    pub id: String,
    pub slot: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub name_en: Option<String>,
    #[serde(default)]
    pub name_zh: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub rank: Option<u8>,
    pub upties: BTreeMap<String, SkillTier>,
}

impl SkillRecord {
    pub fn tier(&self, uptie: Uptie) -> Option<&SkillTier> {
        self.upties.get(&uptie.0.clamp(1, 4).to_string())
    }

    pub fn sin(&self, uptie: Uptie) -> Option<Sin> {
        self.tier(uptie).and_then(|t| t.sin_affinity.as_deref()).and_then(Sin::parse)
    }

    pub fn damage_type(&self, _uptie: Uptie) -> Option<DamageType> {
        self.kind.as_deref().and_then(DamageType::parse)
    }

    pub fn slot(&self) -> Option<SkillSlot> {
        SkillSlot::parse(&self.slot)
    }

    pub fn display_name(&self) -> String {
        self.name_en
            .clone()
            .or_else(|| self.name.clone())
            .unwrap_or_else(|| self.id.clone())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IdentityStats {
    #[serde(default)]
    pub hp: Option<i32>,
    #[serde(default)]
    pub hp_growth: Option<f64>,
    #[serde(default)]
    pub defense_level_mod: Option<i32>,
    #[serde(default)]
    pub speed: BTreeMap<String, Vec<i32>>,
    #[serde(default)]
    pub resist: BTreeMap<String, f64>,
    #[serde(default)]
    pub stagger_thresholds: Vec<i32>,
}

impl IdentityStats {
    /// `HP = Base + Mult * Level` (wiki.gg `Clash` / Health).
    pub fn hp_at_level(&self, level: i32) -> i32 {
        let base = self.hp.unwrap_or(0) as f64;
        let mult = self.hp_growth.unwrap_or(0.0);
        (base + mult * level as f64).floor() as i32
    }

    pub fn speed_range(&self, uptie: Uptie) -> Option<(i32, i32)> {
        let key = match uptie.0 {
            4 => "4",
            3 => "3",
            2 => "2",
            _ => self.speed.keys().next()?.as_str(),
        };
        let range = self.speed.get(key)?;
        Some((*range.first()?, *range.get(1)?))
    }

    pub fn resist_physical(&self, kind: DamageType) -> Option<f64> {
        let key = match kind {
            DamageType::Slash => "slash",
            DamageType::Pierce => "pierce",
            DamageType::Blunt => "blunt",
        };
        self.resist.get(key).copied()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdentityRecord {
    pub id: String,
    #[serde(default)]
    pub sinner: Option<String>,
    #[serde(default)]
    pub sinner_zh: Option<String>,
    #[serde(default)]
    pub title_en: Option<String>,
    #[serde(default)]
    pub title_zh: Option<String>,
    #[serde(default)]
    pub wiki_title: Option<String>,
    #[serde(default)]
    pub rarity: Option<i32>,
    #[serde(default)]
    pub season: Option<String>,
    #[serde(default)]
    pub release_date: Option<String>,
    #[serde(default)]
    pub world: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub stats: IdentityStats,
    #[serde(default)]
    pub skills: Vec<SkillRecord>,
    #[serde(default)]
    pub passives: Vec<PassiveRecord>,
    #[serde(default)]
    pub unparsed: Vec<String>,
    #[serde(default)]
    pub sources: Vec<SourceRecord>,
}

impl IdentityRecord {
    pub fn skill(&self, slot: SkillSlot) -> Option<&SkillRecord> {
        self.skills.iter().find(|s| s.slot() == Some(slot))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EgoSkill {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub sin: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub base_power: Option<i32>,
    #[serde(default)]
    pub coin_power: Option<i32>,
    #[serde(default)]
    pub coins: Option<u32>,
    #[serde(default)]
    pub attack_weight: Option<u32>,
    #[serde(default)]
    pub offense_level_mod: Option<i32>,
    #[serde(default)]
    pub on_use_text: String,
    #[serde(default)]
    pub coin_texts: Vec<String>,
    #[serde(default)]
    pub reuse: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EgoRecord {
    pub id: String,
    #[serde(default)]
    pub name_en: Option<String>,
    #[serde(default)]
    pub name_zh: Option<String>,
    #[serde(default)]
    pub wiki_title: Option<String>,
    #[serde(default)]
    pub risk: Option<String>,
    #[serde(default)]
    pub sin_affinity: Option<String>,
    #[serde(default)]
    pub awakening_sp: Option<i32>,
    #[serde(default)]
    pub corrosion_sp: Option<i32>,
    #[serde(default)]
    pub resource_cost: BTreeMap<String, i32>,
    #[serde(default)]
    pub awakening: Option<EgoSkill>,
    #[serde(default)]
    pub corrosion: Option<EgoSkill>,
    #[serde(default)]
    pub unparsed: Vec<String>,
    #[serde(default)]
    pub sources: Vec<SourceRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnemySkillRecord {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub hint: Option<i32>,
    #[serde(default)]
    pub level: Option<i32>,
    #[serde(default)]
    pub sin: Option<String>,
    #[serde(default)]
    pub rank: Option<i32>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub base_power: Option<i32>,
    #[serde(default)]
    pub coin_power: Option<i32>,
    #[serde(default)]
    pub coins: Option<u32>,
    #[serde(default)]
    pub attack_weight: Option<u32>,
    #[serde(default)]
    pub offense_level_mod: Option<i32>,
    #[serde(default)]
    pub on_use_text: String,
    #[serde(default)]
    pub coin_texts: Vec<String>,
}

impl EnemySkillRecord {
    pub fn sin(&self) -> Option<Sin> {
        self.sin.as_deref().and_then(Sin::parse)
    }
    pub fn damage_type(&self) -> Option<DamageType> {
        self.kind.as_deref().and_then(DamageType::parse)
    }
    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| "?".to_string())
    }

    /// Official skill id (falls back to the display name for records the
    /// pipeline could not match to an in-game id).
    pub fn skill_id(&self) -> String {
        self.id.clone().unwrap_or_else(|| self.display_name())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EnemyPartRecord {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub hp: Option<i32>,
    #[serde(default)]
    pub hp_growth: Option<f64>,
    #[serde(default)]
    pub level: Option<i32>,
    #[serde(default)]
    pub speed: Option<String>,
    #[serde(default)]
    pub defense_level_mod: Option<i32>,
    #[serde(default)]
    pub stagger_thresholds: Vec<Option<i32>>,
    #[serde(default)]
    pub resist_physical: BTreeMap<String, Option<f64>>,
    #[serde(default)]
    pub resist_sin: BTreeMap<String, Option<f64>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnemyRecord {
    pub id: String,
    #[serde(default)]
    pub name_en: Option<String>,
    #[serde(default)]
    pub wiki_title: Option<String>,
    #[serde(default)]
    pub abno_code: Option<String>,
    #[serde(default)]
    pub risk: Option<String>,
    #[serde(default)]
    pub sin_affinity: Option<String>,
    #[serde(default)]
    pub hp: Option<i32>,
    #[serde(default)]
    pub hp_growth: Option<f64>,
    #[serde(default)]
    pub level: Option<i32>,
    #[serde(default)]
    pub parts: Vec<EnemyPartRecord>,
    #[serde(default)]
    pub skills: Vec<EnemySkillRecord>,
    #[serde(default)]
    pub passives: Vec<EnemyPassiveRecord>,
    #[serde(default)]
    pub unparsed: Vec<String>,
    #[serde(default)]
    pub sources: Vec<SourceRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnemyPassiveRecord {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub hint: Option<i32>,
    #[serde(default)]
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatusRecord {
    pub key: String,
    #[serde(default)]
    pub name_en: Option<String>,
    #[serde(default)]
    pub name_zh: Option<String>,
    #[serde(default)]
    pub wiki_name: Option<String>,
    #[serde(default)]
    pub text_en: String,
    #[serde(default)]
    pub text_zh: String,
    #[serde(default)]
    pub source_kind: Option<String>,
    #[serde(default)]
    pub sources: Vec<SourceRecord>,
}

/// Everything the simulator can load, keyed by official id.
#[derive(Clone, Debug, Default)]
pub struct Library {
    pub identities: BTreeMap<String, IdentityRecord>,
    pub egos: BTreeMap<String, EgoRecord>,
    pub enemies: BTreeMap<String, EnemyRecord>,
    pub statuses: BTreeMap<String, StatusRecord>,
}

#[derive(Debug)]
pub enum LibraryError {
    Io(String),
    Parse(String),
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LibraryError::Io(m) => write!(f, "io error: {m}"),
            LibraryError::Parse(m) => write!(f, "parse error: {m}"),
        }
    }
}

impl std::error::Error for LibraryError {}

fn read_dir<T: for<'de> Deserialize<'de>>(dir: &Path) -> Result<BTreeMap<String, T>, LibraryError> {
    let mut out = BTreeMap::new();
    if !dir.exists() {
        return Ok(out);
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| LibraryError::Io(e.to_string()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "json").unwrap_or(false))
        .collect();
    entries.sort();
    for path in entries {
        let text = std::fs::read_to_string(&path).map_err(|e| LibraryError::Io(e.to_string()))?;
        let value: T = serde_json::from_str(&text)
            .map_err(|e| LibraryError::Parse(format!("{}: {e}", path.display())))?;
        let key = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        out.insert(key, value);
    }
    Ok(out)
}

impl Library {
    pub fn load(root: &Path) -> Result<Library, LibraryError> {
        let status_path = root.join("statuses").join("statuses.json");
        let statuses = if status_path.exists() {
            let text = std::fs::read_to_string(&status_path)
                .map_err(|e| LibraryError::Io(e.to_string()))?;
            let list: Vec<StatusRecord> = serde_json::from_str(&text)
                .map_err(|e| LibraryError::Parse(format!("{}: {e}", status_path.display())))?;
            list.into_iter().map(|s| (s.key.clone(), s)).collect()
        } else {
            BTreeMap::new()
        };
        Ok(Library {
            identities: read_dir(&root.join("identities"))?,
            egos: read_dir(&root.join("ego"))?,
            enemies: read_dir(&root.join("enemies"))?,
            statuses,
        })
    }

    pub fn identity(&self, id: &IdentityId) -> Option<&IdentityRecord> {
        self.identities.get(id.as_str())
    }

    pub fn ego(&self, id: &EgoId) -> Option<&EgoRecord> {
        self.egos.get(id.as_str())
    }

    pub fn enemy(&self, id: &EnemyId) -> Option<&EnemyRecord> {
        self.enemies.get(id.as_str())
    }

    pub fn skill(&self, id: &SkillId) -> Option<(&IdentityRecord, &SkillRecord)> {
        for identity in self.identities.values() {
            if let Some(skill) = identity.skills.iter().find(|s| s.id == id.0) {
                return Some((identity, skill));
            }
        }
        None
    }

    /// Records that strict mode must refuse: anything with unparsed fields.
    pub fn strict_blockers(&self) -> Vec<String> {
        let mut out = Vec::new();
        for r in self.identities.values() {
            for u in &r.unparsed {
                out.push(format!("identity {}: {u}", r.id));
            }
        }
        for r in self.egos.values() {
            for u in &r.unparsed {
                out.push(format!("ego {}: {u}", r.id));
            }
        }
        for r in self.enemies.values() {
            for u in &r.unparsed {
                out.push(format!("enemy {}: {u}", r.id));
            }
        }
        out
    }
}
