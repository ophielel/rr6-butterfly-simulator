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
    pub fn total(&self, key: &str) -> i32 {
        let s = self.get(key);
        s.potency + s.count
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
            level: 0,
            turns_remaining: 0,
        }
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

/// A skill typed into the dashboard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardSlot {
    pub slot: u32,
    pub skill: SkillId,
    pub from_deck: bool,
    pub target: Option<UnitId>,
}

/// The identity skill deck.  Skills are drawn from `draw`; the deck only
/// refreshes once it is empty (wiki.gg `Clash`: "Sinners pull Skills out of a
/// 'Skill Deck' ... and will only refresh after all Skills have been used").
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillDeck {
    pub draw: Vec<SkillId>,
    pub discard: Vec<SkillId>,
}

impl SkillDeck {
    pub fn new(draw: Vec<SkillId>) -> Self {
        Self { draw, discard: Vec::new() }
    }

    /// Draw the next skill.  A drawn skill counts as used, so the deck only
    /// refreshes once every card has cycled through the discard pile.
    pub fn draw_top(&mut self) -> Option<SkillId> {
        if self.draw.is_empty() {
            self.refresh();
        }
        let card = self.draw.pop()?;
        self.discard.push(card.clone());
        Some(card)
    }

    pub fn discard_skill(&mut self, skill: &SkillId) {
        self.discard.push(skill.clone());
    }

    pub fn refresh(&mut self) {
        let mut refreshed: Vec<SkillId> = self.discard.drain(..).collect();
        refreshed.extend(self.draw.drain(..));
        self.draw = refreshed;
    }

    pub fn len(&self) -> usize {
        self.draw.len() + self.discard.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn discard_pile(&self) -> Vec<SkillId> {
        self.discard.clone()
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Unit {
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
    pub deck: SkillDeck,
    pub dashboard: Vec<DashboardSlot>,
    /// E.G.O resources per sin, shared by the team (stored on the team state).
    pub ego_slots: Vec<EgoId>,
    pub alive: bool,
}

impl Unit {
    pub fn offense_level(&self) -> i32 {
        (self.level + self.offense_level_mod).max(1)
    }

    pub fn defense_level(&self) -> i32 {
        (self.level + self.defense_level_mod).max(1)
    }

    pub fn resist(&self, kind: crate::ids::DamageType) -> f64 {
        let key = match kind {
            crate::ids::DamageType::Slash => "slash",
            crate::ids::DamageType::Pierce => "pierce",
            crate::ids::DamageType::Blunt => "blunt",
        };
        self.resist_physical.get(key).copied().unwrap_or(1.0)
    }

    pub fn resist_sin(&self, sin: Sin) -> f64 {
        let key = match sin {
            Sin::Wrath => "wrath",
            Sin::Lust => "lust",
            Sin::Sloth => "sloth",
            Sin::Gluttony => "gluttony",
            Sin::Gloom => "gloom",
            Sin::Pride => "pride",
            Sin::Envy => "envy",
        };
        self.resist_sin.get(key).copied().unwrap_or(1.0)
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
    pub fn take_damage(&mut self, amount: i32) -> (i32, i32) {
        let amount = amount.max(0);
        let absorbed = self.shield.min(amount);
        self.shield -= absorbed;
        let remainder = amount - absorbed;
        let before = self.hp;
        self.hp = (self.hp - remainder).max(0);
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
    /// What happens when both skills show the same Clash Power.
    ClashTie,
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
            UnknownRule::ClashTie => {
                "clash tie resolution is not documented; default is that both sides lose the coin"
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClashTieRule {
    /// Both sides destroy the coin they just compared.
    BothLoseCoin,
    /// The attacking side wins the tie.
    AttackerWins,
    /// The defender wins the tie.
    DefenderWins,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BattleConfig {
    pub uptie: Uptie,
    pub tie_rule: ClashTieRule,
    /// `None` == UNKNOWN rule, treated as 0 with a warning entry in the log.
    pub sp_on_clash_win: Option<i32>,
    pub sp_on_clash_lose: Option<i32>,
    /// Refuse to run skills whose effect text is not fully modelled.
    pub strict_mechanics: bool,
    pub max_turns: u32,
}

impl Default for BattleConfig {
    fn default() -> Self {
        Self {
            uptie: Uptie::IV,
            tie_rule: ClashTieRule::BothLoseCoin,
            sp_on_clash_win: None,
            sp_on_clash_lose: None,
            strict_mechanics: true,
            max_turns: 30,
        }
    }
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
    pub ego_resources: BTreeMap<String, i32>,
    pub log: Vec<LogEntry>,
    pub warnings: Vec<String>,
    pub winner: Option<Winner>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmittedAction {
    pub actor: UnitId,
    pub slot: u32,
    pub skill: SkillId,
    pub target: Option<UnitId>,
    pub is_ego: bool,
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
