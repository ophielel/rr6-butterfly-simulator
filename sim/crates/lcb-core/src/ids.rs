//! Official game identifiers.
//!
//! Every id here is the *in-game* id taken from the client localisation data
//! (`Personalities.json`, `Egos.json`, `Enemies_Refraction6.json`,
//! `Skills_Abnormality_Refraction6.json`, `Skills_personality-*.json`).
//! Player nicknames are never used as an identifier.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! string_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(IdentityId, "Identity id, e.g. `10110` (Lobotomy E.G.O::Solemn Lament Yi Sang).");
string_id!(EgoId, "E.G.O id, e.g. `20109` (Solemn Lament Yi Sang).");
string_id!(EnemyId, "Enemy id, e.g. `9567` (Butterfly of Entangled Lives::Imago).");
string_id!(SkillId, "Skill id, e.g. `1011001`.");
string_id!(UnitId, "Runtime unit handle, stable within an encounter.");

/// Sin affinity. Order matches the in-game ordering and the data files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Sin {
    Wrath,
    Lust,
    Sloth,
    Gluttony,
    Gloom,
    Pride,
    Envy,
}

impl Sin {
    pub const ALL: [Sin; 7] = [
        Sin::Wrath,
        Sin::Lust,
        Sin::Sloth,
        Sin::Gluttony,
        Sin::Gloom,
        Sin::Pride,
        Sin::Envy,
    ];

    pub fn index(self) -> usize {
        match self {
            Sin::Wrath => 0,
            Sin::Lust => 1,
            Sin::Sloth => 2,
            Sin::Gluttony => 3,
            Sin::Gloom => 4,
            Sin::Pride => 5,
            Sin::Envy => 6,
        }
    }

    pub fn parse(text: &str) -> Option<Sin> {
        Some(match text.trim().to_ascii_lowercase().as_str() {
            "wrath" => Sin::Wrath,
            "lust" => Sin::Lust,
            "sloth" => Sin::Sloth,
            "gluttony" => Sin::Gluttony,
            "gloom" => Sin::Gloom,
            "pride" => Sin::Pride,
            "envy" => Sin::Envy,
            _ => return None,
        })
    }
}

/// Physical damage type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DamageType {
    Slash,
    Pierce,
    Blunt,
}

impl DamageType {
    pub const ALL: [DamageType; 3] = [DamageType::Slash, DamageType::Pierce, DamageType::Blunt];

    pub fn index(self) -> usize {
        match self {
            DamageType::Slash => 0,
            DamageType::Pierce => 1,
            DamageType::Blunt => 2,
        }
    }

    pub fn parse(text: &str) -> Option<DamageType> {
        Some(match text.trim().to_ascii_lowercase().as_str() {
            "slash" => DamageType::Slash,
            "pierce" => DamageType::Pierce,
            "blunt" => DamageType::Blunt,
            _ => return None,
        })
    }
}

/// A skill's role, as listed in the skill slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillSlot {
    Skill1,
    Skill2,
    Skill3,
    Defense,
}

impl SkillSlot {
    pub fn parse(text: &str) -> Option<SkillSlot> {
        Some(match text.trim().to_ascii_lowercase().as_str() {
            "skill1" => SkillSlot::Skill1,
            "skill2" => SkillSlot::Skill2,
            "skill3" => SkillSlot::Skill3,
            "defense" | "def" => SkillSlot::Defense,
            _ => return None,
        })
    }
}

/// Uptie tier (1..=4). Uptie IV is the default used by high-end content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Uptie(pub u8);

impl Uptie {
    pub const IV: Uptie = Uptie(4);
    pub const III: Uptie = Uptie(3);

    pub fn index(self) -> usize {
        (self.0.clamp(1, 4) - 1) as usize
    }
}
