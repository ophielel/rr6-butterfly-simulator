//! Enemy action-pattern scripts.
//!
//! The Imago's rotation is documented turn by turn on the wikis (see
//! `tools/build_enemy_scripts.py`).  Scripts are data: the engine only decides
//! *which* turn of the cycle is being played and which time state is active.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The Imago's three states of time (wiki.gg `Butterfly of Entangled Lives`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TimeState {
    Past,
    Present,
    Future,
}

impl TimeState {
    pub const ALL: [TimeState; 3] = [TimeState::Past, TimeState::Present, TimeState::Future];

    /// Status key holding this state's Stack on the unit.
    pub fn stack_key(self) -> &'static str {
        match self {
            TimeState::Past => "In the Past",
            TimeState::Present => "In the Present",
            TimeState::Future => "In the Future",
        }
    }

    /// Status the state's skills inflict or gain extra of.
    pub fn boosted_status(self) -> &'static str {
        match self {
            TimeState::Past => "Burn",
            TimeState::Present => "Poise",
            TimeState::Future => "Bleed",
        }
    }

    pub fn parse(text: &str) -> Option<TimeState> {
        Some(match text.trim().to_ascii_lowercase().as_str() {
            "past" => TimeState::Past,
            "present" => TimeState::Present,
            "future" => TimeState::Future,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TimeState::Past => "past",
            TimeState::Present => "present",
            TimeState::Future => "future",
        }
    }

    /// Bonus granted by the state's Stack (in-game `Bufs_Refraction6` text):
    /// 0-10: +1 Potency and Count · 11-20: Clash Power +1, +2 Potency/+1 Count ·
    /// 21-30: Final Power +2, +3 Potency/+2 Count.
    pub fn stack_bonus(stack: i32) -> StackBonus {
        match stack {
            s if s <= 10 => StackBonus { clash_power: 0, final_power: 0, potency: 1, count: 1 },
            s if s <= 20 => StackBonus { clash_power: 1, final_power: 0, potency: 2, count: 1 },
            _ => StackBonus { clash_power: 0, final_power: 2, potency: 3, count: 2 },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackBonus {
    pub clash_power: i32,
    pub final_power: i32,
    pub potency: i32,
    pub count: i32,
}

/// "The Past/Present/Future - Segmentation" (stations 2-4 and the Imago's
/// stack bookkeeping).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Segmentation {
    /// Which state of time this illusion's hits take Stacks from.
    pub stack_status: String,
    pub stack_loss_per_hit: i32,
    pub gain_if_not_hit: i32,
    pub attacker_sp_heal: i32,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct ClashCountSwing {
    pub threshold: i32,
    pub divisor: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptBranch {
    /// Only `barrier_broken` is defined so far.
    pub when: String,
    pub turns: Vec<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptBand {
    /// `above_66` / `below_66` / `below_33`
    pub band: String,
    /// One entry per turn of the cycle; each entry lists the skills of the
    /// unit's Skill Slots in order.
    pub turns: Vec<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateScript {
    pub small: String,
    pub mid: String,
    pub big: String,
    pub bands: Vec<ScriptBand>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnemyScript {
    pub enemy_id: String,
    pub slots: u32,
    pub cycle_turns: u32,
    #[serde(default)]
    pub station: i32,
    #[serde(default)]
    pub shared: BTreeMap<String, String>,
    #[serde(default)]
    pub states: BTreeMap<String, StateScript>,
    /// Stateless patterns (e.g. the Pupa): one entry per turn of the cycle.
    #[serde(default)]
    pub turns: Vec<Vec<String>>,
    /// Optional branch played instead of `turns` once its condition holds.
    #[serde(default)]
    pub branch: Option<ScriptBranch>,
    /// Encounter start: a flat Shield (the illusory butterflies' 333).
    #[serde(default)]
    pub shield_flat: Option<i32>,
    /// Encounter start: Shield as a percentage of max HP.
    #[serde(default)]
    pub shield_percent: Option<f64>,
    /// "End the Encounter" only applies while the Shield still held; if it broke,
    /// the encounter runs one more turn instead.
    #[serde(default)]
    pub ends_encounter_unless_shield_broken: bool,
    /// "The Past - Segmentation": hits knock Stacks off the Imago.
    #[serde(default)]
    pub segmentation: Option<Segmentation>,
    /// HP cannot drop below this percentage of max HP.
    #[serde(default)]
    pub hp_floor_percent: Option<i32>,
    /// Skills whose Attack End ends the encounter.
    #[serde(default)]
    pub ends_encounter_on: Vec<String>,
    /// Station 5: the unit is not skipped by Stagger.
    #[serde(default)]
    pub acts_while_staggered: bool,
    /// "Causality that Threads the Past, the Present, and the Future":
    /// from `threshold` clashes against the same target, Clash Power swings by
    /// `clash_count / divisor` in a random direction.
    #[serde(default)]
    pub clash_count_swing: Option<ClashCountSwing>,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub source: Vec<String>,
    #[serde(default)]
    pub ja_table: String,
}

impl EnemyScript {
    /// Health band name for a unit at `hp_percent`.
    pub fn band_name(hp_percent: i32) -> &'static str {
        if hp_percent > 66 {
            "above_66"
        } else if hp_percent > 33 {
            "below_66"
        } else {
            "below_33"
        }
    }

    /// Skills used by the unit's slots on `cycle_index` of the cycle.
    /// `branch_active` selects the barrier-broken branch when the script has one.
    pub fn turn_skills_with_branch(
        &self,
        hp_percent: i32,
        state: TimeState,
        cycle_index: u32,
        branch_active: bool,
    ) -> Vec<String> {
        if branch_active {
            if let Some(branch) = &self.branch {
                if let Some(turn) = branch.turns.first() {
                    return turn.clone();
                }
            }
        }
        if self.states.is_empty() {
            if self.turns.is_empty() {
                return Vec::new();
            }
            let index = (cycle_index as usize) % self.turns.len();
            return self.turns[index].clone();
        }
        self.turn_skills(hp_percent, state, cycle_index)
    }

    /// State-based rotation lookup (the Imago).
    pub fn turn_skills(
        &self,
        hp_percent: i32,
        state: TimeState,
        cycle_index: u32,
    ) -> Vec<String> {
        let band_name = Self::band_name(hp_percent);
        let Some(state_script) = self.states.get(state.as_str()) else {
            return Vec::new();
        };
        let Some(band) = state_script.bands.iter().find(|b| b.band == band_name) else {
            return Vec::new();
        };
        if band.turns.is_empty() {
            return Vec::new();
        }
        let index = (cycle_index as usize) % band.turns.len();
        band.turns[index].clone()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScriptsBook {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub skills: BTreeMap<String, EnemyScript>,
}

impl ScriptsBook {
    pub fn load(path: &Path) -> Result<ScriptsBook, String> {
        if !path.exists() {
            return Ok(ScriptsBook::default());
        }
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn for_enemy(&self, enemy_id: &str) -> Option<&EnemyScript> {
        self.skills
            .values()
            .find(|script| script.enemy_id == enemy_id)
    }
}
