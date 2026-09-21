//! Battle state.
//!
//! Types are modelled so that inapplicable states are unrepresentable where the
//! game's rules allow it - an Abnormality cannot hold Sanity, a unit without a
//! Stagger Threshold cannot be staggered, and so on.

use crate::ids::{EgoId, EnemyId, IdentityId, Sin, SkillId, Uptie, UnitId};
use crate::rng::{heads_chance, Rng};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Sanity Points are capped at +/-45 (wiki.gg `Sanity`).
pub const SP_LIMIT: i32 = 45;

/// A unit either has Sanity or it does not; there is no "has_sp: false".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sanity {
    /// Abnormalities and other unsapient units: coins always flip at 50%.
    None,
    Sane { sp: i32 },
}

impl Sanity {
    pub fn sp(self) -> i32 {
        match self {
            Sanity::None => 0,
            Sanity::Sane { sp } => sp,
        }
    }

    pub fn heads_percent(self) -> i32 {
        heads_chance(self.sp())
    }

    pub fn add(self, delta: i32) -> Sanity {
        match self {
            Sanity::None => Sanity::None,
            Sanity::Sane { sp } => Sanity::Sane {
                sp: (sp + delta).clamp(-SP_LIMIT, SP_LIMIT),
            },
        }
    }

    pub fn set(self, value: i32) -> Sanity {
        match self {
            Sanity::None => Sanity::None,
            Sanity::Sane { .. } => Sanity::Sane {
                sp: value.clamp(-SP_LIMIT, SP_LIMIT),
            },
        }
    }
}

/// Potency / Count / Stack triple.  Most statuses use Potency + Count; a few use
/// Stack only; quantities that do not apply stay at 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusInstance {
    pub potency: i32,
    pub count: i32,
    pub stack: i32,
}

impl StatusInstance {
    pub fn is_empty(&self) -> bool {
        self.potency == 0 && self.count == 0 && self.stack == 0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSet {
    map: BTreeMap<String, StatusInstance>,
}

impl StatusSet {
    pub fn get(&self, key: &str) -> StatusInstance {
        self.map.get(key).copied().unwrap_or_default()
    }

    pub fn potency(&self, key: &str) -> i32 {
        self.get(key).potency
    }

    pub fn count(&self, key: &str) -> i32 {
        self.get(key).count
    }

    pub fn stack(&self, key: &str) -> i32 {
        self.get(key).stack
    }

    /// Potency + Count, used by several skill conditions ("both [Butterfly]").
    /// Every value the status carries.  A single-value status may store its
    /// amount in Stack ("LCA Fracture Round", "Bullet - Solitude"), so a plain
    /// "how much does this unit have" must include it.
    pub fn total(&self, key: &str) -> i32 {
        let s = self.get(key);
        s.potency + s.count + s.stack
    }

    pub fn add_potency(&mut self, key: &str, delta: i32) {
        let entry = self.map.entry(key.to_string()).or_default();
        entry.potency = (entry.potency + delta).max(0);
        if entry.is_empty() {
            self.map.remove(key);
        }
    }

    pub fn add_count(&mut self, key: &str, delta: i32) {
        let entry = self.map.entry(key.to_string()).or_default();
        entry.count = (entry.count + delta).max(0);
        if entry.is_empty() {
            self.map.remove(key);
        }
    }

    pub fn set_stack(&mut self, key: &str, value: i32) {
        let entry = self.map.entry(key.to_string()).or_default();
        entry.stack = value.max(0);
        if entry.is_empty() {
            self.map.remove(key);
        }
    }

    /// Potency + Count, used by the Past passive's "10+ (Burn Potency + Count)".
    pub fn total_of(&self, key: &str) -> i32 {
        let s = self.get(key);
        s.potency + s.count
    }

    pub fn add_stack(&mut self, key: &str, delta: i32) {
        let entry = self.map.entry(key.to_string()).or_default();
        entry.stack = (entry.stack + delta).max(0);
        if entry.is_empty() {
            self.map.remove(key);
        }
    }

    pub fn set(&mut self, key: &str, value: StatusInstance) {
        if value.is_empty() {
            self.map.remove(key);
            return;
        }
        self.map.insert(key.to_string(), value);
    }

    pub fn remove(&mut self, key: &str) {
        self.map.remove(key);
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.map.keys()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &StatusInstance)> {
        self.map.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Stagger thresholds are stored as percentages of max HP (wiki.gg `Clash`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaggerState {
    /// Percent-of-max-HP thresholds, descending, as printed in game.
    pub thresholds_percent: Vec<i32>,
    /// The same lines as absolute HP, which is what the simulation moves.
    ///
    /// "[Tremor Burst] raises the target's Stagger Threshold by [Tremor]
    /// Potency" is an **HP** amount, not percentage points, so the runtime works
    /// in HP (a 10 Potency burst moves the line by 10 HP, not by 10% of max HP).
    #[serde(default)]
    pub thresholds_hp: Vec<i32>,
    /// How many of the lines are gone: "混乱区間はステージ中復活しない" - a line
    /// the unit has crossed never comes back
    /// (JA-wiki `戦闘システム詳細` / 混乱状態についての詳しい仕様).
    #[serde(default)]
    pub consumed: usize,
    /// How many thresholds have been crossed (0 = not staggered).
    pub level: u8,
    /// Turns of Stagger remaining; a staggered unit cannot act.
    pub turns_remaining: u8,
}

impl StaggerState {
    pub const MAX_LEVEL: u8 = 3;

    pub fn new(thresholds_percent: Vec<i32>) -> Self {
        Self {
            thresholds_percent,
            thresholds_hp: Vec::new(),
            consumed: 0,
            level: 0,
            turns_remaining: 0,
        }
    }

    /// Resolve the printed percentages into absolute HP lines.
    pub fn bind_max_hp(&mut self, max_hp: i32) {
        if self.thresholds_hp.is_empty() {
            self.thresholds_hp = self
                .thresholds_percent
                .iter()
                .map(|percent| max_hp * percent / 100)
                .collect();
        }
    }

    /// The absolute HP of a Stagger line, falling back to the printed percent
    /// when the unit was never bound to a max HP.
    pub fn threshold_hp(&self, index: usize) -> Option<i32> {
        if let Some(value) = self.thresholds_hp.get(index) {
            return Some(*value);
        }
        self.thresholds_percent.get(index).copied()
    }

    /// Move the first line that is still in play by `amount` HP (positive makes
    /// the unit easier to Stagger, negative harder).
    pub fn shift_first_threshold(&mut self, amount: i32) -> Option<i32> {
        if self.thresholds_hp.is_empty() {
            if let Some(first) = self.thresholds_percent.first_mut() {
                *first = (*first + amount).max(0);
                return Some(*first);
            }
            return None;
        }
        let index = self.consumed.min(self.thresholds_hp.len().saturating_sub(1));
        let value = self.thresholds_hp.get_mut(index)?;
        *value = (*value + amount).max(0);
        Some(*value)
    }

    /// How many of the lines still in play the unit is currently below.
    pub fn remaining_crossed(&self, hp: i32) -> u8 {
        let mut crossed = 0u8;
        for index in self.consumed..self.thresholds_hp.len() {
            if hp <= self.thresholds_hp[index] {
                crossed += 1;
            } else {
                break;
            }
        }
        crossed
    }

    pub fn is_staggered(&self) -> bool {
        self.turns_remaining > 0
    }

    /// Resistance bonus from being staggered (wiki.gg `Damage`).
    pub fn damage_resistance_bonus(&self) -> f64 {
        match self.level {
            0 => 0.0,
            n => 0.5 + 0.5 * n as f64,
        }
    }

    /// Number of thresholds currently surpassed at `hp` / `max_hp`.
    pub fn crossed_thresholds(&self, hp: i32, max_hp: i32) -> u8 {
        if !self.thresholds_hp.is_empty() {
            let mut crossed = 0u8;
            for (index, value) in self.thresholds_hp.iter().enumerate() {
                if hp <= *value {
                    crossed = (index as u8 + 1).min(Self::MAX_LEVEL);
                }
            }
            return crossed;
        }
        let mut crossed = 0u8;
        for (index, threshold) in self.thresholds_percent.iter().enumerate() {
            if *threshold < 0 {
                continue;
            }
            let value = max_hp * threshold / 100;
            if hp <= value {
                crossed = (index as u8 + 1).min(Self::MAX_LEVEL);
            }
        }
        crossed
    }
}

/// One dashboard slot.
///
/// The panel shows two skills per slot: the bottom one (`current`) is the skill
/// that will be used, and the top one (`next`) is the already-drawn follow-up.
/// Sources: wiki.gg `Battles` (Skill Dashboard) and the Japanese wiki
/// `Wiki管理/戦闘指南/戦闘システム詳細` ("スキルを使用すると、そのスキルがパネルから
/// 除外される。そして上に見えていたスキルが下へ送られ…また次のスキルが新しく薄らと
/// 見えるようになる。").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardSlot {
    pub slot: u32,
    /// Bottom row: the skill used this turn.
    pub current: SkillId,
    /// Second visible skill: also selectable this turn.
    pub next: SkillId,
    /// The faint preview above them (drawn, not selectable yet).
    pub preview: SkillId,
    pub target: Option<UnitId>,
    /// Set when the player swapped the bottom skill for a defense skill or an
    /// E.G.O this turn (the original skill is consumed either way).
    pub converted: bool,
    /// The bottom card a defense skill / E.G.O replaced this turn.  The panel
    /// keeps it so the rotation can consume the right card: "使った守備スキルは
    /// 守備スキルに変更した元のスキルごとパネルから消え、次のターンには新しい
    /// スキルがパネルに追加される" (JA-wiki 守備スキル).
    #[serde(default)]
    pub replaced: Option<SkillId>,
}

impl DashboardSlot {
    pub fn new(slot: u32, current: SkillId, next: SkillId, preview: SkillId) -> Self {
        Self {
            slot,
            current,
            next,
            preview,
            target: None,
            converted: false,
            replaced: None,
        }
    }
}

/// The identity skill composition ("Skill Amount" copies per skill).
///
/// The panel is filled from one shared composition per identity, regardless of
/// how many slots the unit has.  Draws are random among the copies that have not
/// been placed on the panel yet, and the counts reset once every copy has been
/// placed.  Source: Japanese wiki `戦闘システム詳細` ("スキル構成",
/// "継ぎ足しの優先度は下段→上段", "全てパネルに配置し終えると構成の残数がリセットされる").
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillDeck {
    /// Copies of each skill, from `Skill Amount` (typically 3 / 2 / 1).
    pub composition: Vec<(SkillId, u32)>,
    /// Copies not yet placed on the panel.
    pub remaining: Vec<(SkillId, u32)>,
    /// Copies currently displayed on the panel.
    pub paneled: Vec<(SkillId, u32)>,
}

impl SkillDeck {
    pub fn new(composition: Vec<(SkillId, u32)>) -> Self {
        let composition: Vec<(SkillId, u32)> =
            composition.into_iter().filter(|(_, n)| *n > 0).collect();
        Self {
            remaining: composition.clone(),
            composition,
            paneled: Vec::new(),
        }
    }

    fn total(entries: &[(SkillId, u32)]) -> u32 {
        entries.iter().map(|(_, n)| *n).sum()
    }

    /// Refill the draw pool once every copy of the composition has been placed
    /// on the panel.  The panel keeps what it is showing: with several slots the
    /// same skill can legitimately appear more often than its composition count.
    pub fn reset(&mut self) {
        self.remaining = self.composition.clone();
    }

    /// Draw one skill for the panel, excluding nothing but the copies whose
    /// count is already exhausted.  Uses the battle RNG, so the draw is part of
    /// the recorded state.
    pub fn draw(&mut self, rng: &mut Rng) -> Option<SkillId> {
        if Self::total(&self.remaining) == 0 {
            self.reset();
        }
        let total = Self::total(&self.remaining);
        if total == 0 {
            return None;
        }
        let mut pick = rng.below(total);
        let mut chosen: Option<SkillId> = None;
        for (skill, count) in self.remaining.iter_mut() {
            if pick < *count {
                *count -= 1;
                chosen = Some(skill.clone());
                break;
            }
            pick -= *count;
        }
        self.remaining.retain(|(_, n)| *n > 0);
        let skill = chosen?;
        for (entry, count) in self.paneled.iter_mut() {
            if entry == &skill {
                *count += 1;
                return Some(skill);
            }
        }
        self.paneled.push((skill.clone(), 1));
        Some(skill)
    }

    /// Remove one copy of `skill` from the panel because it was used.
    pub fn consume(&mut self, skill: &SkillId) {
        for (entry, count) in self.paneled.iter_mut() {
            if entry == skill && *count > 0 {
                *count -= 1;
                break;
            }
        }
        self.paneled.retain(|(_, n)| *n > 0);
    }

    pub fn len(&self) -> usize {
        Self::total(&self.remaining) as usize
    }

    pub fn is_empty(&self) -> bool {
        Self::total(&self.remaining) == 0
    }

    pub fn paneled_count(&self, skill: &SkillId) -> u32 {
        self.paneled
            .iter()
            .find(|(entry, _)| entry == skill)
            .map(|(_, count)| *count)
            .unwrap_or(0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum UnitKind {
    Sinner { identity: IdentityId },
    Abnormality { enemy: EnemyId, part: Option<String> },
}

impl UnitKind {
    pub fn is_sinner(&self) -> bool {
        matches!(self, UnitKind::Sinner { .. })
    }
}

fn one() -> usize {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Unit {
    /// How many units this unit's current Skill use targets ("When attacking
    /// just a single target" conditions read this).
    #[serde(default = "one")]
    pub planned_targets: usize,
    pub id: UnitId,
    pub kind: UnitKind,
    pub name: String,
    pub level: i32,
    pub hp: i32,
    pub max_hp: i32,
    pub shield: i32,
    pub sanity: Sanity,
    pub speed: i32,
    pub speed_range: (i32, i32),
    pub offense_level_mod: i32,
    pub defense_level_mod: i32,
    /// Physical resistances (x1.0 == Normal).
    pub resist_physical: BTreeMap<String, f64>,
    /// Sin resistances, only meaningful for units that have them.
    pub resist_sin: BTreeMap<String, f64>,
    pub stagger: StaggerState,
    pub statuses: StatusSet,
    /// Targets this unit's most recent Skill killed (for "[Attack End] If 1 or
    /// more targets are killed" clauses).
    #[serde(default)]
    pub skill_kills: i32,
    /// Suit currently in this unit's Hand (Hanafuda markers).
    #[serde(default)]
    pub suit: Option<String>,
    /// "[Attack End] For N turns, lose X SP at Combat End".
    #[serde(default)]
    pub combat_end_sp_loss: Vec<(i32, i32)>,
    /// Marker statuses that have no numeric value of their own: the Tremor
    /// amplitudes ("Amplitude Conversion: Tremor - Decay"), the Suit in this
    /// unit's hand, and the like (wiki.gg `Status Effects`).
    #[serde(default)]
    pub status_markers: Vec<String>,
    pub deck: SkillDeck,
    pub dashboard: Vec<DashboardSlot>,
    /// Panic Type of this unit (wiki.gg `Sanity`) and its resolved clauses.
    #[serde(default)]
    pub panic_type: Option<String>,
    #[serde(default)]
    pub panic_low_morale: Vec<crate::effects::Effect>,
    #[serde(default)]
    pub panic_actions: Vec<crate::effects::Effect>,
    /// Low Morale / Panic for the current turn (set at Turn Start).
    #[serde(default)]
    pub low_morale: bool,
    #[serde(default)]
    pub panicked: bool,
    /// A Sinner at -45 SP who owns a Corrosion Skill is forced into E.G.O
    /// Corrosion instead of Panicking (wiki.gg `Sanity`).
    #[serde(default)]
    pub corroded: bool,
    /// A Sinner at -45 SP was Panicked or Corroded last turn: SP resets to 0.
    #[serde(default)]
    pub panic_recovering: bool,
    /// Passives of this unit (its own combat passives plus the team's support
    /// passives), evaluated for continuous modifiers and phase triggers.
    #[serde(default)]
    pub passives: Vec<crate::effects::SkillMechanics>,
    /// The ids of the passives above, so engine code can look for a specific
    /// rule ("this unit may redirect attacks to itself regardless of Speed").
    #[serde(default)]
    pub passive_ids: Vec<String>,
    /// Status amounts as they were at Combat Start, for "for every 2 [Piercing
    /// Sword] this unit had at Combat Start" clauses.
    #[serde(default)]
    pub combat_start_statuses: std::collections::BTreeMap<String, i32>,
    /// True while a passive's Unopposed Attack is resolving, so that attack's own
    /// Attack End cannot start another one.
    #[serde(default)]
    pub follow_up_active: bool,
    /// Hanafuda: the Hand is converted at the **next** Turn Start
    /// ("Next turn, convert the Hand on self to the one corresponding to the used
    /// Skill's Affinity").
    #[serde(default)]
    pub pending_suit_conversion: bool,
    /// `Petals` gained this turn ("can be gained up to 15 Stacks per turn").
    #[serde(default)]
    pub petals_gained: i32,
    /// Skill Slots whose Skill says "Can Clash with this Skill regardless of
    /// Speed": a Sinner may chain to them even when slower.
    #[serde(default)]
    pub clash_any_speed_slots: Vec<u32>,
    /// Every Skill of this unit's identity kit, including the defense Skill
    /// (their "[Turn Start] / [Turn End]" clauses resolve for the unit).
    #[serde(default)]
    pub identity_skills: Vec<SkillId>,
    /// E.G.O resources per sin, shared by the team (stored on the team state).
    pub ego_slots: Vec<EgoId>,
    /// The E.G.O of this unit that own a Corrosion Skill (for forced Corrosion
    /// at -45 SP).
    #[serde(default)]
    pub corrosion_egos: Vec<EgoId>,
    pub alive: bool,
    /// Where an enemy is in its (stand-in) skill cycle.
    #[serde(default)]
    pub skill_cursor: u32,
    /// Imago: which of In the Past / In the Present / In the Future is active.
    #[serde(default)]
    pub time_state: Option<crate::scripts::TimeState>,
    /// Bit flags for the 66% / 33% Stack grants (they happen once each).
    #[serde(default)]
    pub time_threshold_flags: u8,
    /// Number of Skill Slots this enemy has (script driven for the Imago).
    #[serde(default)]
    pub enemy_slots: u32,
    /// Some enemies act even while Staggered (JA table, station 5).
    #[serde(default)]
    pub acts_while_staggered: bool,
    /// Script knob: random Clash Power swing after N clashes with one target.
    #[serde(default)]
    pub clash_count_swing: Option<crate::scripts::ClashCountSwing>,
    /// The state-exclusive "big" skill of each state of time; these carry the
    /// Past/Present/Future passive bonuses.
    #[serde(default)]
    pub time_signature: Vec<(crate::scripts::TimeState, String)>,
    /// Encounter start: Shield as a percentage of max HP (the Pupa's 1.3%).
    #[serde(default)]
    pub shield_percent: Option<f64>,
    /// Encounter start: a flat Shield (the illusory butterflies' 333).
    #[serde(default)]
    pub shield_flat: Option<i32>,
    /// "End the Encounter" only while the Shield held.
    #[serde(default)]
    pub ends_encounter_unless_shield_broken: bool,
    /// HP floor as a percentage of max HP (the Pupa's "HP does not fall below
    /// 90%").
    #[serde(default)]
    pub hp_floor_percent: Option<i32>,
    /// Set once the Shield was fully consumed while it had one.
    #[serde(default)]
    pub barrier_broken: bool,
    /// Skills whose Attack End ends the encounter.
    #[serde(default)]
    pub ends_encounter_on: Vec<String>,
    /// Per-turn usage counters for effects with a "(N times per turn)" limit,
    /// keyed by the effect's source line.
    #[serde(default)]
    pub turn_effect_usage: BTreeMap<String, i32>,
    /// Effects queued for the start of the next turn
    /// ("Gain 2 Protection next turn").
    #[serde(default)]
    pub pending_next_turn: Vec<PendingStatus>,
    /// "When hit while this unit has Shield, inflict N [X] against the attacker"
    /// (defense-skill passive, active for the turn).
    #[serde(default)]
    pub retaliate_on_hit: Vec<RetaliateOnHit>,
    /// "The X - Segmentation" data when this unit is an illusory butterfly.
    #[serde(default)]
    pub segmentation: Option<crate::scripts::Segmentation>,
    /// Hits taken as a main target this turn (Segmentation counts one per Coin).
    #[serde(default)]
    pub hits_taken: i32,
    /// Sinners already healed by Segmentation this turn.
    #[serde(default)]
    pub segmentation_healed: Vec<String>,
    /// Highest Sin Resonance / Absolute Sin Resonance for this turn (copied from
    /// the battle state so conditions can read it).
    #[serde(default)]
    pub resonance_max: i32,
    /// Per-sin Resonance counts for this turn ("At 3+ (Gloom Reson.)").
    #[serde(default)]
    pub resonance_of: BTreeMap<String, i32>,
    #[serde(default)]
    pub a_reson_max: i32,
}

/// Cross-station state for the Refraction Railway encounter chain.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CampaignState {
    /// Stacks of `In the Past / In the Present / In the Future` on the Imago.
    pub time_stacks: BTreeMap<String, i32>,
    /// Passive components disabled by the earlier stations' choice events.
    pub disabled_passives: Vec<String>,
    /// Which station the campaign is at (1 = Pupa ... 5 = Imago).
    pub station: i32,
    /// The Pupa's remaining HP when station 1 ended (Section 5 starts from it).
    #[serde(default)]
    pub pupa_hp: Option<i32>,
}

impl CampaignState {
    pub fn is_disabled(&self, key: &str) -> bool {
        self.disabled_passives.iter().any(|entry| entry == key)
    }
}

/// A retaliation registered by a defense skill for the current turn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetaliateOnHit {
    pub status: String,
    pub potency: i32,
}

/// A status application queued for the next Turn Start.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingStatus {
    pub status: String,
    pub potency: i32,
    pub count: i32,
    /// Stack-based statuses ("Inflict 3 [Blue Sand] next turn") keep their
    /// Stack through the delay.
    #[serde(default)]
    pub stack: i32,
}

impl Unit {
    /// Offense Level including `Offense Level Up/Down`
    /// (wiki.gg `Status Effects`: "increases based on the effect's Potency").
    pub fn offense_level(&self) -> i32 {
        let up = self.statuses.potency("Offense Level Up");
        let down = self.statuses.potency("Offense Level Down");
        (self.level + self.offense_level_mod + up - down).max(1)
    }

    pub fn defense_level(&self) -> i32 {
        let up = self.statuses.potency("Defense Level Up");
        let down = self.statuses.potency("Defense Level Down");
        (self.level + self.defense_level_mod + up - down).max(1)
    }

    /// Physical resistance, including `<Type> Resist Down` (0.1 per Count).
    pub fn resist(&self, kind: crate::ids::DamageType) -> f64 {
        let (key, name) = match kind {
            crate::ids::DamageType::Slash => ("slash", "Slash"),
            crate::ids::DamageType::Pierce => ("pierce", "Pierce"),
            crate::ids::DamageType::Blunt => ("blunt", "Blunt"),
        };
        let base = self.resist_physical.get(key).copied().unwrap_or(1.0);
        // The status is measured by Count per the wiki text, but the localisation
        // dump words some of them as Stack; read whichever component was filled.
        let key = format!("{name} Resist Down");
        let down = self.statuses.count(&key) + self.statuses.potency(&key) + self.statuses.stack(&key);
        (base + 0.1 * down as f64).max(0.0)
    }

    /// Sin resistance, including `<Sin> Resist Down` (0.1 per Count,
    /// wiki.gg `Status Effects` / Gloom Resist Down).
    pub fn resist_sin(&self, sin: Sin) -> f64 {
        let (key, name) = match sin {
            Sin::Wrath => ("wrath", "Wrath"),
            Sin::Lust => ("lust", "Lust"),
            Sin::Sloth => ("sloth", "Sloth"),
            Sin::Gluttony => ("gluttony", "Gluttony"),
            Sin::Gloom => ("gloom", "Gloom"),
            Sin::Pride => ("pride", "Pride"),
            Sin::Envy => ("envy", "Envy"),
        };
        let base = self.resist_sin.get(key).copied().unwrap_or(1.0);
        // The status is measured by Count per the wiki text, but the localisation
        // dump words some of them as Stack; read whichever component was filled.
        let key = format!("{name} Resist Down");
        let down = self.statuses.count(&key) + self.statuses.potency(&key) + self.statuses.stack(&key);
        (base + 0.1 * down as f64).max(0.0)
    }

    /// Damage the unit deals (dynamic modifier): `Damage Up` +10% per Count,
    /// `Damage Down` -10% per Count (both capped at 10).
    pub fn outgoing_damage_modifier(&self) -> f64 {
        let up = self.statuses.count("Damage Up").min(10);
        let down = self.statuses.count("Damage Down").min(10);
        (up - down) as f64 * 0.10
    }

    pub fn hp_percent(&self) -> i32 {
        if self.max_hp <= 0 {
            return 0;
        }
        (self.hp * 100) / self.max_hp
    }

    pub fn is_staggered(&self) -> bool {
        self.stagger.is_staggered()
    }

    pub fn add_poise(&mut self, potency: i32, count: i32) {
        self.statuses.add_potency("Poise", potency);
        self.statuses.add_count("Poise", count);
    }

    /// Apply damage to Shield first, then HP. Returns (shield lost, hp lost).
    /// Units with an HP floor (the Pupa's "HP does not fall below 90%") cannot
    /// be taken below it by damage.
    pub fn take_damage(&mut self, amount: i32) -> (i32, i32) {
        // "Does not take damage for this turn" (The Quickening).
        if self.statuses.stack("No Damage Taken") > 0 {
            return (0, 0);
        }
        let amount = amount.max(0);
        let absorbed = self.shield.min(amount);
        self.shield -= absorbed;
        if absorbed > 0 && self.shield == 0 {
            self.barrier_broken = true;
        }
        let remainder = amount - absorbed;
        let floor = match self.hp_floor_percent {
            Some(percent) => (self.max_hp * percent / 100).max(1),
            None => 0,
        };
        let before = self.hp;
        // The floor never heals a unit that is already below it.
        let effective_floor = floor.min(before);
        self.hp = (self.hp - remainder).max(effective_floor);
        if self.hp == 0 {
            self.alive = false;
        }
        (absorbed, before - self.hp)
    }

    pub fn heal(&mut self, amount: i32) {
        self.hp = (self.hp + amount.max(0)).min(self.max_hp);
    }
}

/// Which rules the engine had to guess or does not model at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnknownRule {
    /// The wiki documents that the SP factors changed but not the current
    /// values, so SP gain/loss on a clash is caller-supplied.
    SanityGainOnClash,
    /// Coin-flip RNG algorithm of the client is not observable.
    CoinFlipRng,
}

impl UnknownRule {
    pub fn text(self) -> &'static str {
        match self {
            UnknownRule::SanityGainOnClash => {
                "SP gain/loss per clash is not documented for the current game version; \
                 configure BattleConfig::sp_on_clash_win / sp_on_clash_lose"
            }
            UnknownRule::CoinFlipRng => {
                "client RNG algorithm unknown; the simulator uses a documented substitute RNG"
            }
        }
    }
}

/// How enemies choose their skill.  The real rotations are documented on the
/// wiki in a notation this project does not treat as unambiguous, so the default
/// is an explicit stand-in (see docs/MECHANICS.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnemyPolicy {
    /// Always the first skill listed for the unit.
    FirstListed,
    /// Walk the unit's skill list in order, cycling.
    Cyclic,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BattleConfig {
    pub uptie: Uptie,
    #[serde(default = "default_enemy_policy")]
    pub enemy_policy: EnemyPolicy,
    /// `None` == UNKNOWN rule, treated as 0 with a warning entry in the log.
    pub sp_on_clash_win: Option<i32>,
    pub sp_on_clash_lose: Option<i32>,
    /// Refuse to run skills whose effect text is not fully modelled.
    pub strict_mechanics: bool,
    /// Sanity a Sinner starts the encounter with.  The in-game value for the
    /// fixed content is 0, but a campaign carries SP between stations, so it is
    /// configurable (45 == the upper clamp).
    #[serde(default = "default_starting_sp")]
    pub starting_sp: i32,
    pub max_turns: u32,
    /// Focused encounter (an Abnormality fight).  Only there can a Sinner whose
    /// Speed is higher than an enemy Slot's redirect that enemy Skill onto
    /// itself ("幻想体戦では、速度で勝っている敵のスキルの使用先を変更することが
    /// できる", JA-wiki 戦闘システム詳細); in normal battles the Clash partners are
    /// decided by the game's own front-to-back matching.
    #[serde(default)]
    pub focused_encounter: bool,
    /// Imago: the state of time the encounter starts in.  The wiki ties it to
    /// the choice events of the previous stations, so it is configurable; the
    /// default is the first listed state.
    #[serde(default = "default_initial_time_state")]
    pub initial_time_state: crate::scripts::TimeState,
}

pub fn default_starting_sp() -> i32 {
    45
}

fn default_initial_time_state() -> crate::scripts::TimeState {
    crate::scripts::TimeState::Past
}

impl Default for BattleConfig {
    fn default() -> Self {
        Self {
            uptie: Uptie::IV,
            enemy_policy: EnemyPolicy::Cyclic,
            sp_on_clash_win: None,
            sp_on_clash_lose: None,
            strict_mechanics: true,
            starting_sp: default_starting_sp(),
            focused_encounter: true,
            max_turns: 30,
            initial_time_state: crate::scripts::TimeState::Past,
        }
    }
}

fn default_enemy_policy() -> EnemyPolicy {
    EnemyPolicy::Cyclic
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Setup,
    TurnStart,
    AwaitingActions,
    Combat,
    TurnEnd,
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Winner {
    Sinners,
    Enemies,
    Draw,
    /// The stage ended by a skill effect ("End the Encounter") rather than by a
    /// wipe; Refraction Railway stations resolve this way.
    EncounterEnded,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub turn: u32,
    pub kind: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BattleState {
    pub seed: u64,
    pub rng: Rng,
    pub turn: u32,
    pub phase: Phase,
    pub config: BattleConfig,
    pub units: Vec<Unit>,
    /// Deployment order (Sinners are deployed first).
    pub deployment: Vec<UnitId>,
    pub actions: Vec<SubmittedAction>,
    /// Defense skills active this turn (guards, evades, counters).
    #[serde(default)]
    pub defenses: Vec<crate::battle::ActiveDefense>,
    /// Slots whose Skill was replaced by a defense Skill this turn (they rotate
    /// like any other used Slot: the defense and the card it replaced both leave
    /// the panel).
    #[serde(default)]
    pub defense_slots_used: Vec<(usize, u32)>,
    pub ego_resources: BTreeMap<String, i32>,
    /// Campaign bookkeeping shared by the stations (the Imago's Stacks of time
    /// are carried from stations 2-4 into Section 5).
    #[serde(default)]
    pub campaign: CampaignState,
    /// Sin Resonance for the current turn: sin key -> number of Skills of that
    /// affinity selected on the Dashboard.  Source: wiki.gg `Resonance`.
    #[serde(default)]
    pub resonance: BTreeMap<String, i32>,
    /// Absolute Sin Resonance: sin key -> longest run of consecutive Skills of
    /// that affinity on the Dashboard (3+ is A-Reson).
    #[serde(default)]
    pub a_resonance: BTreeMap<String, i32>,
    /// Clashes fought between two units, keyed `"actor|target"`; the Imago's
    /// "Causality that Threads ..." passive reads it.
    #[serde(default)]
    pub clash_counts: BTreeMap<String, i32>,
    pub log: Vec<LogEntry>,
    pub warnings: Vec<String>,
    pub winner: Option<Winner>,
    /// Set when a skill with "End the Encounter" resolved (Refraction Railway
    /// stations end this way rather than by a wipe).
    #[serde(default)]
    pub encounter_ended: bool,
    /// Total Skill Slots the encounter grows towards (focused encounters use
    /// the maximum number of deployable Sinners).
    #[serde(default)]
    pub slot_target: usize,
    /// Scripted coin results, consumed before the RNG.  Used to reproduce a
    /// recorded fight (golden tests) and to pin flips in mechanic tests.
    #[serde(default)]
    pub preset_flips: Vec<bool>,
    #[serde(default)]
    pub flip_cursor: usize,    /// Behaviour of every status (loaded from `status_effects.json`), so that
    /// status text like "[Turn Start] gain 1 [X] for every Stack" can run.
    #[serde(default)]
    #[serde(skip)]
    pub status_book: Option<std::sync::Arc<crate::effects::StatusBook>>,
    /// Offense/Defense Level a Skill gains from its Sin Resonance chain, keyed
    /// `<unit id>|<slot>` (wiki.gg `Sin Resonance`).
    #[serde(default)]
    pub resonance_levels: std::collections::BTreeMap<String, i32>,

}

impl BattleState {
    /// Flip a coin: scripted results win, otherwise the RNG is used with the
    /// unit's heads chance.  Every flip advances `flip_cursor`, so a replay is
    /// exact regardless of the generator.
    /// A uniform roll in `0..=range` from the replayable RNG stream (used by
    /// "take 4 ~ 8 HP damage" and "gain 1 ~ 2 [X]" clauses).
    pub fn roll_inclusive(&mut self, range: i32) -> i32 {
        if range <= 0 {
            return 0;
        }
        if self.flip_cursor < self.preset_flips.len() {
            self.flip_cursor += 1;
            return range / 2;
        }
        self.rng.below(range as u32 + 1) as i32
    }

    pub fn flip(&mut self, percent: i32) -> bool {
        if self.flip_cursor < self.preset_flips.len() {
            let value = self.preset_flips[self.flip_cursor];
            self.flip_cursor += 1;
            return value;
        }
        self.rng.flip_percent(percent)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmittedAction {
    pub actor: UnitId,
    pub slot: u32,
    pub skill: SkillId,
    pub target: Option<UnitId>,
    pub is_ego: bool,
    /// The enemy Skill Slot this action chains to (focused encounters): a Skill
    /// may redirect an enemy Skill whose Speed it beats, or one already aimed at
    /// it, and the Clash is then formed against that Slot.
    #[serde(default)]
    pub enemy_slot: Option<u32>,
    /// True when the action used the **top** card of the Slot (its `next`
    /// skill).  The panel is not modified on submit; the used card is consumed
    /// when the turn resolves, and the other card stays available.
    #[serde(default)]
    pub used_top: bool,
    #[serde(default)]
    pub ego: Option<EgoId>,
    #[serde(default)]
    pub ego_kind: Option<crate::battle::EgoSkillKind>,
}

impl BattleState {
    pub fn unit(&self, id: &UnitId) -> Option<&Unit> {
        self.units.iter().find(|u| &u.id == id)
    }

    pub fn unit_mut(&mut self, id: &UnitId) -> Option<&mut Unit> {
        self.units.iter_mut().find(|u| &u.id == id)
    }

    pub fn index_of(&self, id: &UnitId) -> Option<usize> {
        self.units.iter().position(|u| &u.id == id)
    }

    pub fn living_sinners(&self) -> Vec<UnitId> {
        self.units
            .iter()
            .filter(|u| u.alive && u.kind.is_sinner())
            .map(|u| u.id.clone())
            .collect()
    }

    pub fn living_enemies(&self) -> Vec<UnitId> {
        self.units
            .iter()
            .filter(|u| u.alive && !u.kind.is_sinner())
            .map(|u| u.id.clone())
            .collect()
    }

    pub fn push_log(&mut self, kind: &str, detail: impl Into<String>) {
        self.log.push(LogEntry {
            turn: self.turn,
            kind: kind.to_string(),
            detail: detail.into(),
        });
    }
}
