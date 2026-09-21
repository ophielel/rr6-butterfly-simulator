//! Encounter construction: turning library records into runtime units.

use crate::effects::MechanicsBook;
use crate::ids::{DamageType, EgoId, EnemyId, IdentityId, Sin, SkillId, Uptie, UnitId};
use crate::library::{EnemyRecord, IdentityRecord, Library};
use crate::state::{
    BattleConfig, BattleState, DashboardSlot, Phase, Sanity, SkillDeck, StaggerState, StatusSet,
    Unit, UnitKind,
};
use crate::rng::Rng;
use std::collections::BTreeMap;

pub const TEAM_SIZE_CAP: usize = 6;

#[derive(Debug)]
pub enum SetupError {
    UnknownIdentity(String),
    UnknownEnemy(String),
    MissingSkill(String),
    MissingField(String),
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetupError::UnknownIdentity(id) => write!(f, "unknown identity id {id}"),
            SetupError::UnknownEnemy(id) => write!(f, "unknown enemy id {id}"),
            SetupError::MissingSkill(id) => write!(f, "identity {id} has no usable skills"),
            SetupError::MissingField(what) => write!(f, "missing library field: {what}"),
        }
    }
}

impl std::error::Error for SetupError {}

fn resist_map(identity: &IdentityRecord) -> BTreeMap<String, f64> {
    let mut map = BTreeMap::new();
    for kind in DamageType::ALL {
        let key = match kind {
            DamageType::Slash => "slash",
            DamageType::Pierce => "pierce",
            DamageType::Blunt => "blunt",
        };
        map.insert(key.to_string(), identity.stats.resist_physical(kind).unwrap_or(1.0));
    }
    map
}

fn enemy_resist_map(record: &EnemyRecord) -> (BTreeMap<String, f64>, BTreeMap<String, f64>) {
    let mut physical = BTreeMap::new();
    let mut sin = BTreeMap::new();
    for kind in DamageType::ALL {
        let key = match kind {
            DamageType::Slash => "slash",
            DamageType::Pierce => "pierce",
            DamageType::Blunt => "blunt",
        };
        physical.insert(key.to_string(), 1.0);
    }
    for s in Sin::ALL {
        sin.insert(sin_key(s).to_string(), 1.0);
    }
    if let Some(part) = record.parts.first() {
        for (key, value) in &part.resist_physical {
            if let Some(v) = value {
                physical.insert(key.clone(), *v);
            }
        }
        for (key, value) in &part.resist_sin {
            if let Some(v) = value {
                sin.insert(key.clone(), *v);
            }
        }
    }
    (physical, sin)
}

/// Attach the rules books to the units: each unit's own **Combat** Passives,
/// its Panic Type with the clauses resolved, and the E.G.O that own a Corrosion
/// Skill.
///
/// Support Passives are deliberately **not** applied: the extracted ones are
/// keyed by identity, but the team's support slots are a deck-building choice
/// this project has no data for (see docs/STATUS.md).
pub fn attach_unit_books(
    state: &mut BattleState,
    passives: &crate::effects::PassiveBook,
    panics: Option<&crate::effects::PanicBook>,
    library: &Library,
) {
    let owner_ids: Vec<String> = state
        .units
        .iter()
        .map(|unit| match &unit.kind {
            crate::state::UnitKind::Sinner { identity } => identity.0.clone(),
            crate::state::UnitKind::Abnormality { enemy, .. } => enemy.0.clone(),
        })
        .collect();
    for (index, unit) in state.units.iter_mut().enumerate() {
        let owned = passives.for_owner(&owner_ids[index]);
        unit.passive_ids = owned.iter().map(|passive| passive.id.clone()).collect();
        unit.passives = owned
            .into_iter()
            .map(|passive| passive.effects.clone())
            .collect();
        unit.corrosion_egos = unit
            .ego_slots
            .iter()
            .filter(|ego| {
                library
                    .ego(ego)
                    .map(|record| record.corrosion.is_some())
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        if let crate::state::UnitKind::Sinner { identity } = &unit.kind {
            if let Some(panics) = panics {
                let panic = panics.for_identity(&identity.0).cloned();
                unit.panic_type = Some(
                    panic
                        .as_ref()
                        .map(|p| p.r#type.clone())
                        .unwrap_or_else(|| "Panic".to_string()),
                );
                if let Some(panic) = panic {
                    unit.panic_low_morale = panic.low_morale.clone();
                    unit.panic_actions = panic.panic.clone();
                }
            }
        }
    }
}

pub fn sin_key(sin: Sin) -> &'static str {
    match sin {
        Sin::Wrath => "wrath",
        Sin::Lust => "lust",
        Sin::Sloth => "sloth",
        Sin::Gluttony => "gluttony",
        Sin::Gloom => "gloom",
        Sin::Pride => "pride",
        Sin::Envy => "envy",
    }
}

fn speed_range(identity: &IdentityRecord, uptie: Uptie) -> (i32, i32) {
    identity.stats.speed_range(uptie).unwrap_or((1, 1))
}

/// Build the skill composition from the identity's `Skill Amount` (typically
/// 3 copies of Skill 1, 2 of Skill 2 and 1 of Skill 3).  Source: wiki.gg
/// `Battles` / Skills and the Japanese wiki `戦闘システム詳細` ("スキル構成").
pub fn build_deck(identity: &IdentityRecord, uptie: Uptie) -> SkillDeck {
    let mut composition: Vec<(SkillId, u32)> = Vec::new();
    for skill in &identity.skills {
        let Some(slot) = skill.slot() else { continue };
        if slot == crate::ids::SkillSlot::Defense {
            continue;
        }
        let amount = skill.tier(uptie).and_then(|t| t.skill_amount).unwrap_or(1).max(1);
        composition.push((SkillId::new(skill.id.clone()), amount));
    }
    SkillDeck::new(composition)
}

pub struct EncounterBuilder<'a> {
    pub library: &'a Library,
    pub mechanics: &'a MechanicsBook,
    pub scripts: &'a crate::scripts::ScriptsBook,
    pub config: BattleConfig,
    pub seed: u64,
    /// Passives / Panic Types / status behaviour.  They are attached to the
    /// units **before** Turn 1 starts, otherwise the first Turn Start runs
    /// without them (the units' passives and status upkeep only exist from
    /// Turn 2 on).
    pub passives: Option<&'a crate::effects::PassiveBook>,
    pub panics: Option<&'a crate::effects::PanicBook>,
    pub statuses: Option<&'a crate::effects::StatusBook>,
}

impl<'a> EncounterBuilder<'a> {
    pub fn new(library: &'a Library, mechanics: &'a MechanicsBook) -> Self {
        static EMPTY: std::sync::OnceLock<crate::scripts::ScriptsBook> = std::sync::OnceLock::new();
        Self {
            library,
            mechanics,
            scripts: EMPTY.get_or_init(crate::scripts::ScriptsBook::default),
            config: BattleConfig::default(),
            seed: 0,
            passives: None,
            panics: None,
            statuses: None,
        }
    }

    /// Give every unit its own Combat Passives, its Panic Type and the status
    /// behaviour book, exactly as `Simulator` does for later encounters.
    pub fn books(
        mut self,
        passives: &'a crate::effects::PassiveBook,
        panics: &'a crate::effects::PanicBook,
        statuses: &'a crate::effects::StatusBook,
    ) -> Self {
        self.passives = Some(passives);
        self.panics = Some(panics);
        self.statuses = Some(statuses);
        self
    }

    pub fn scripts(mut self, scripts: &'a crate::scripts::ScriptsBook) -> Self {
        self.scripts = scripts;
        self
    }

    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    pub fn config(mut self, config: BattleConfig) -> Self {
        self.config = config;
        self
    }

    pub fn build(
        self,
        team: &[&str],
        enemies: &[&str],
    ) -> Result<BattleState, SetupError> {
        let mut units = Vec::new();
        let mut deployment = Vec::new();
        let mut warnings = Vec::new();

        for (index, raw_id) in team.iter().enumerate() {
            let id = IdentityId::new(*raw_id);
            let record = self
                .library
                .identity(&id)
                .ok_or_else(|| SetupError::UnknownIdentity(raw_id.to_string()))?;
            let unit = self.sinner_unit(record, index)?;
            deployment.push(unit.id.clone());
            units.push(unit);
        }

        for (index, raw_id) in enemies.iter().enumerate() {
            let id = EnemyId::new(*raw_id);
            let record = self
                .library
                .enemy(&id)
                .ok_or_else(|| SetupError::UnknownEnemy(raw_id.to_string()))?;
            let unit = self.enemy_unit(record, team.len() + index)?;
            deployment.push(unit.id.clone());
            units.push(unit);
        }

        if self.config.strict_mechanics {
            for unit in &units {
                if let UnitKind::Sinner { identity } = &unit.kind {
                    if let Some(record) = self.library.identity(identity) {
                        for skill in &record.skills {
                            // Keys are `id@tier`; a bare-id lookup would always
                            // miss and warn about every Skill.
                            let mech = self
                                .mechanics
                                .get_for(&SkillId::new(skill.id.clone()), self.config.uptie);
                            match mech {
                                None => warnings.push(format!(
                                    "skill {} ({}) has no mechanics entry",
                                    skill.id,
                                    skill.display_name()
                                )),
                                Some(m) => {
                                    for line in &m.unmodeled {
                                        warnings.push(format!(
                                            "skill {} unmodeled effect: {line}",
                                            skill.id
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut state = BattleState {
            status_book: None,
            seed: self.seed,
            rng: Rng::from_seed(self.seed),
            turn: 0,
            phase: Phase::Setup,
            config: self.config.clone(),
            units,
            deployment,
            actions: Vec::new(),
            defenses: Vec::new(),
            campaign: crate::state::CampaignState {
                time_stacks: crate::scripts::TimeState::ALL
                    .iter()
                    .map(|s| (s.stack_key().to_string(), 0))
                    .collect(),
                disabled_passives: Vec::new(),
                station: 0,
                pupa_hp: None,
            },
            resonance: BTreeMap::new(),
            a_resonance: BTreeMap::new(),
            clash_counts: BTreeMap::new(),
            ego_resources: Sin::ALL
                .iter()
                .map(|s| (sin_key(*s).to_string(), 0i32))
                .collect(),
            log: Vec::new(),
            warnings,
            winner: None,
            encounter_ended: false,
            defense_slots_used: Vec::new(),
            preset_flips: Vec::new(),
            flip_cursor: 0,
            slot_target: if team.len() >= TEAM_SIZE_CAP { team.len() } else { TEAM_SIZE_CAP },
        };

        // Attach the rules books before Turn 1 starts.
        if let Some(statuses) = self.statuses {
            state.status_book = Some(std::sync::Arc::new(statuses.clone()));
        }
        if let Some(passives) = self.passives {
            attach_unit_books(&mut state, passives, self.panics, self.library);
        }
        // Turn 1: one slot per Sinner, each showing the usable skill (bottom)
        // plus the already drawn follow-up (top).  Extra slots are handed out
        // from turn 2 on (wiki.gg `Battles` / Deployment Order).
        for index in 0..state.units.len() {
            if !state.units[index].kind.is_sinner() {
                continue;
            }
            let mut panel = Vec::new();
            let current = draw_for_unit(&mut state, index).unwrap_or_else(empty_skill);
            let next = draw_for_unit(&mut state, index).unwrap_or_else(empty_skill);
            let preview = draw_for_unit(&mut state, index).unwrap_or_else(empty_skill);
            panel.push(DashboardSlot::new(0, current, next, preview));
            state.units[index].dashboard = panel;
        }
        // Turn 1 starts immediately: the caller then assigns actions and commits.
        crate::battle::begin_turn(&mut state, self.library, self.mechanics, self.scripts);
        // Encounter-start Shields: the Pupa gains 1.3% of max HP, the illusory
        // butterflies a flat 333.
        for index in 0..state.units.len() {
            if let Some(percent) = state.units[index].shield_percent {
                let shield = (state.units[index].max_hp as f64 * percent / 100.0).floor() as i32;
                state.units[index].shield += shield.max(1);
            }
            if let Some(flat) = state.units[index].shield_flat {
                state.units[index].shield += flat.max(0);
            }
        }
        Ok(state)
    }

    fn sinner_unit(&self, record: &IdentityRecord, index: usize) -> Result<Unit, SetupError> {
        let uptie = self.config.uptie;
        let level = 60;
        let max_hp = record.stats.hp_at_level(level);
        let deck = build_deck(record, uptie);
        if deck.is_empty() {
            return Err(SetupError::MissingSkill(record.id.clone()));
        }
        let speed = speed_range(record, uptie);
        let resist_physical = resist_map(record);
        let resist_sin: BTreeMap<String, f64> =
            Sin::ALL.iter().map(|s| (sin_key(*s).to_string(), 1.0)).collect();
        Ok(Unit {
            planned_targets: 1,
            passive_ids: Vec::new(),
            petals_gained: 0,
            clash_any_speed_slots: Vec::new(),
            id: UnitId::new(format!("sinner-{index}-{}", record.id)),
            kind: UnitKind::Sinner {
                identity: IdentityId::new(record.id.clone()),
            },
            name: format!(
                "{} ({})",
                record.title_en.clone().unwrap_or_default(),
                record.sinner.clone().unwrap_or_default()
            ),
            level,
            hp: max_hp,
            max_hp,
            shield: 0,
            sanity: Sanity::Sane { sp: 0 },
            speed: 0,
            speed_range: speed,
            offense_level_mod: 0,
            defense_level_mod: record.stats.defense_level_mod.unwrap_or(0),
            resist_physical,
            resist_sin,
            stagger: {
                let mut stagger = StaggerState::new(record.stats.stagger_thresholds.clone());
                stagger.bind_max_hp(max_hp);
                stagger
            },
            statuses: StatusSet::default(),
            identity_skills: record.skills.iter().map(|s| SkillId::new(s.id.clone())).collect(),
            passives: Vec::new(),
            corrosion_egos: Vec::new(),
            panic_type: None,
            panic_low_morale: Vec::new(),
            panic_actions: Vec::new(),
            low_morale: false,
            panicked: false,
            panic_recovering: false,
            corroded: false,
            status_markers: Vec::new(),
            combat_end_sp_loss: Vec::new(),
            skill_kills: 0,
            suit: None,
            deck,
            dashboard: Vec::new(),
            ego_slots: ego_for_identity(&record.id),
            alive: true,
            skill_cursor: 0,
            time_state: None,
            time_threshold_flags: 0,
            enemy_slots: 1,
            acts_while_staggered: false,
            clash_count_swing: None,
            time_signature: Vec::new(),
            shield_percent: None,
            hp_floor_percent: None,
            barrier_broken: false,
            ends_encounter_on: Vec::new(),
            turn_effect_usage: BTreeMap::new(),
            pending_next_turn: Vec::new(),
            retaliate_on_hit: Vec::new(),
            segmentation: None,
            hits_taken: 0,
            segmentation_healed: Vec::new(),
            resonance_max: 0,
            resonance_of: BTreeMap::new(),
            a_reson_max: 0,
            shield_flat: None,
            ends_encounter_unless_shield_broken: false,
        })
    }

    fn enemy_unit(&self, record: &EnemyRecord, index: usize) -> Result<Unit, SetupError> {
        let level = record.level.unwrap_or(60);
        let max_hp = match (record.hp, record.hp_growth) {
            (Some(base), Some(mult)) => (base as f64 + mult * level as f64).floor() as i32,
            (Some(base), None) => base,
            _ => return Err(SetupError::MissingField(format!("enemy {} hp", record.id))),
        };
        let (resist_physical, resist_sin) = enemy_resist_map(record);
        let thresholds = record
            .parts
            .first()
            .map(|p| p.stagger_thresholds.iter().flatten().copied().collect())
            .unwrap_or_default();
        let speed_range = record
            .parts
            .first()
            .and_then(|p| p.speed.clone())
            .and_then(|s| {
                let mut it = s.split('~');
                let lo = it.next()?.trim().parse().ok()?;
                let hi = it.next()?.trim().parse().ok()?;
                Some((lo, hi))
            })
            .unwrap_or((1, 3));
        Ok(Unit {
            planned_targets: 1,
            passive_ids: Vec::new(),
            petals_gained: 0,
            clash_any_speed_slots: Vec::new(),
            id: UnitId::new(format!("enemy-{index}-{}", record.id)),
            kind: UnitKind::Abnormality {
                enemy: EnemyId::new(record.id.clone()),
                part: record.parts.first().and_then(|p| p.name.clone()),
            },
            name: record
                .name_en
                .clone()
                .unwrap_or_else(|| record.id.clone()),
            level,
            hp: max_hp,
            max_hp,
            shield: 0,
            sanity: Sanity::None,
            speed: 0,
            speed_range,
            offense_level_mod: 0,
            defense_level_mod: record
                .parts
                .first()
                .and_then(|p| p.defense_level_mod)
                .unwrap_or(0),
            resist_physical,
            resist_sin,
            stagger: {
                let mut stagger = StaggerState::new(thresholds);
                stagger.bind_max_hp(max_hp);
                stagger
            },
            statuses: StatusSet::default(),
            identity_skills: Vec::new(),
            passives: Vec::new(),
            corrosion_egos: Vec::new(),
            panic_type: None,
            panic_low_morale: Vec::new(),
            panic_actions: Vec::new(),
            low_morale: false,
            panicked: false,
            panic_recovering: false,
            corroded: false,
            status_markers: Vec::new(),
            combat_end_sp_loss: Vec::new(),
            skill_kills: 0,
            suit: None,
            deck: SkillDeck::default(),
            dashboard: Vec::new(),
            ego_slots: Vec::new(),
            alive: true,
            skill_cursor: 0,
            time_state: None,
            time_threshold_flags: 0,
            enemy_slots: self
                .scripts
                .for_enemy(&record.id)
                .map(|script| script.slots)
                .unwrap_or(1),
            acts_while_staggered: self
                .scripts
                .for_enemy(&record.id)
                .map(|script| script.acts_while_staggered)
                .unwrap_or(false),
            clash_count_swing: self
                .scripts
                .for_enemy(&record.id)
                .and_then(|script| script.clash_count_swing),
            shield_percent: self
                .scripts
                .for_enemy(&record.id)
                .and_then(|script| script.shield_percent),
            hp_floor_percent: self
                .scripts
                .for_enemy(&record.id)
                .and_then(|script| script.hp_floor_percent),
            barrier_broken: false,
            turn_effect_usage: BTreeMap::new(),
            pending_next_turn: Vec::new(),
            retaliate_on_hit: Vec::new(),
            segmentation: self
                .scripts
                .for_enemy(&record.id)
                .and_then(|script| script.segmentation.clone()),
            hits_taken: 0,
            segmentation_healed: Vec::new(),
            resonance_max: 0,
            resonance_of: BTreeMap::new(),
            a_reson_max: 0,
            shield_flat: self
                .scripts
                .for_enemy(&record.id)
                .and_then(|script| script.shield_flat),
            ends_encounter_unless_shield_broken: self
                .scripts
                .for_enemy(&record.id)
                .map(|script| script.ends_encounter_unless_shield_broken)
                .unwrap_or(false),
            ends_encounter_on: self
                .scripts
                .for_enemy(&record.id)
                .map(|script| script.ends_encounter_on.clone())
                .unwrap_or_default(),
            time_signature: self
                .scripts
                .for_enemy(&record.id)
                .map(|script| {
                    crate::scripts::TimeState::ALL
                        .iter()
                        .filter_map(|state| {
                            script
                                .states
                                .get(state.as_str())
                                .map(|entry| (*state, entry.big.clone()))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

pub fn empty_skill() -> SkillId {
    SkillId::new(String::new())
}

/// Draw one skill from the unit's composition (random among the copies not yet
/// placed on the panel).  Returns `None` when the composition is empty.
pub fn draw_for_unit(state: &mut BattleState, index: usize) -> Option<SkillId> {
    let mut deck = std::mem::take(&mut state.units[index].deck);
    let drawn = deck.draw(&mut state.rng);
    state.units[index].deck = deck;
    drawn
}

/// Default helpers for the fixed content of the plan.
pub mod fixed {
    pub const TEAM: [&str; 7] = [
        "10110", // Lobotomy E.G.O::Solemn Lament Yi Sang
        "10414", // Lobotomy E.G.O::Faint Aroma & Solitude Ryōshū
        "10813", // Jeong's Office Rep Ishmael
        "10913", // Lobotomy E.G.O::The Sword Sharpened with Tears Rodion
        "11004", // Los Mariachis Jefe Sinclair
        "11114", // LCA Udjat Vanguard Team 3 Leader Outis
        "11214", // Lobotomy E.G.O::Lamp Gregor
    ];

    pub const EGO_LOADOUT: [(&str, &str); 7] = [
        ("10110", "20109"), // Solemn Lament Yi Sang
        ("10110", "20106"), // Bygone Days Yi Sang
        ("11214", "21207"), // Solemn Lament Gregor
        ("10813", "20807"), // Bygone Days Ishmael
        ("10813", "20810"), // Tidal Elegy Ishmael
        ("11004", "21009"), // Harmony Sinclair
        ("10913", "20903"), // Rime Shank Rodion
    ];

    pub const BOSS_PUPA: &str = "9563";
    pub const BOSS_IMAGO: &str = "9567";
    pub const ILLUSORY_PAST: &str = "9564";
    pub const ILLUSORY_PRESENT: &str = "9565";
    pub const ILLUSORY_FUTURE: &str = "9566";
}

/// E.G.O ids available to a given identity (`None` = not implemented yet).
pub fn ego_for_identity(identity: &str) -> Vec<EgoId> {
    fixed::EGO_LOADOUT
        .iter()
        .filter(|(owner, _)| *owner == identity)
        .map(|(_, ego)| EgoId::new(*ego))
        .collect()
}
