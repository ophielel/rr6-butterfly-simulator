//! Deterministic simulator for the fixed content of the RR6 "Butterfly of
//! Entangled Lives [羅生蝶]" problem set.
//!
//! Design rules taken from the project plan:
//! * mechanics first, and only mechanics that a source supports;
//! * anything unconfirmed is `UNKNOWN`/`NOT_IMPLEMENTED`, never guessed;
//! * simulation rules and experimental logic are separate (search lives in
//!   Python, this crate only simulates).

pub mod battle;
pub mod damage;
pub mod effects;
pub mod hash;
pub mod ids;
pub mod library;
pub mod replay;
pub mod rng;
pub mod scripts;
pub mod setup;
pub mod state;

#[cfg(test)]
pub mod testsupport;

use battle::Action;
use effects::{MechanicsBook, PanicBook, PassiveBook};
use library::{Library, LibraryError};
use setup::{EncounterBuilder, SetupError};
use ids::UnitId;
use state::{BattleConfig, BattleState};
use std::path::{Path, PathBuf};

/// Owns the data library and the mechanics book; the natural entry point for
/// embedders (including the PyO3 layer).
pub struct Simulator {
    pub library: Library,
    pub mechanics: MechanicsBook,
    /// Identity / enemy passives (Combat and Support).
    pub passives: PassiveBook,
    /// Panic Types per identity (wiki.gg `Sanity`).
    pub panics: PanicBook,
    pub scripts: scripts::ScriptsBook,
    pub data_root: PathBuf,
}

#[derive(Debug)]
pub enum SimError {
    Library(LibraryError),
    Setup(SetupError),
    Mechanics(String),
    Rule(String),
}

impl std::fmt::Display for SimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SimError::Library(e) => write!(f, "{e}"),
            SimError::Setup(e) => write!(f, "{e}"),
            SimError::Mechanics(m) => write!(f, "mechanics: {m}"),
            SimError::Rule(m) => write!(f, "rule: {m}"),
        }
    }
}

impl std::error::Error for SimError {}

impl From<LibraryError> for SimError {
    fn from(value: LibraryError) -> Self {
        SimError::Library(value)
    }
}

impl From<SetupError> for SimError {
    fn from(value: SetupError) -> Self {
        SimError::Setup(value)
    }
}

impl Simulator {
    /// Load `data/` (identities, ego, enemies, statuses) and
    /// `data/mechanics/effects.json`.
    pub fn from_data_dir(root: impl AsRef<Path>) -> Result<Self, SimError> {
        let root = root.as_ref().to_path_buf();
        let library = Library::load(&root)?;
        let mechanics = MechanicsBook::load(&root.join("mechanics").join("effects.json"))
            .map_err(SimError::Mechanics)?;
        let scripts = scripts::ScriptsBook::load(&root.join("mechanics").join("enemy_scripts.json"))
            .map_err(SimError::Mechanics)?;
        let passives = PassiveBook::load(
            &root.join("passives").join("passives.json"),
        )
        .map_err(SimError::Mechanics)?;
        let panics = PanicBook::load(
            &root.join("mechanics").join("panic_types.json"),
        )
        .map_err(SimError::Mechanics)?;
        Ok(Self {
            library,
            mechanics,
            scripts,
            passives,
            panics,
            data_root: root,
        })
    }

    pub fn new_encounter(
        &self,
        team: &[&str],
        enemies: &[&str],
        seed: u64,
        config: BattleConfig,
    ) -> Result<BattleState, SimError> {
        let mut state = EncounterBuilder::new(&self.library, &self.mechanics)
            .scripts(&self.scripts)
            .seed(seed)
            .config(config)
            .build(team, enemies)?;
        self.attach_passives(&mut state);
        Ok(state)
    }

    /// Give every unit the passives it fights with: its own Combat Passives and
    /// the team's Support Passives (wiki.gg `Passives`).
    fn attach_passives(&self, state: &mut state::BattleState) {
        let supports: Vec<crate::effects::SkillMechanics> = self
            .passives
            .supports()
            .into_iter()
            .map(|p| p.effects.clone())
            .collect();
        let owner_ids: Vec<String> = state
            .units
            .iter()
            .map(|unit| match &unit.kind {
                crate::state::UnitKind::Sinner { identity } => identity.0.clone(),
                crate::state::UnitKind::Abnormality { enemy, .. } => enemy.0.clone(),
            })
            .collect();
        for (index, unit) in state.units.iter_mut().enumerate() {
            let owner = String::new();
            let _ = owner;
            let mut effects: Vec<crate::effects::SkillMechanics> = self
                .passives
                .for_owner(&owner_ids[index])
                .into_iter()
                .map(|p| p.effects.clone())
                .collect();
            if unit.kind.is_sinner() {
                effects.extend(supports.clone());
            }
            unit.passives = effects;
            if let crate::state::UnitKind::Sinner { identity } = &unit.kind {
                let panic = self.panics.for_identity(&identity.0).cloned();
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

    /// Section 5: the Imago, carrying what the earlier stations left behind.
    ///
    /// The JA wiki's station-5 notes: the fight starts with the HP the Pupa had
    /// in station 1 (excluding its Shield) and with 10 Stacks of each state of
    /// time; the choice events of stations 2-4 disable components of the
    /// Past/Present/Future passives.
    pub fn section5(
        &self,
        campaign: &state::CampaignState,
        seed: u64,
        config: BattleConfig,
    ) -> Result<BattleState, SimError> {
        let mut built = self.new_encounter(
            &setup::fixed::TEAM,
            &[setup::fixed::BOSS_IMAGO],
            seed,
            config,
        )?;
        built.campaign = campaign.clone();
        built.campaign.station = 5;
        if let Some(hp) = campaign.pupa_hp {
            for unit in built.units.iter_mut() {
                if !unit.kind.is_sinner() {
                    unit.hp = hp.clamp(1, unit.max_hp);
                }
            }
        }
        Ok(built)
    }

    pub fn legal_actions(&self, state: &BattleState) -> Vec<Action> {
        battle::legal_actions(state, &self.library)
    }

    pub fn submit(&self, state: &mut BattleState, action: Action) -> Result<(), SimError> {
        battle::submit(state, action).map_err(SimError::Rule)
    }

    /// Commit the actions assigned for this turn: resolves clashes and attacks,
    /// runs turn end, then starts the next turn (so the state is again in
    /// `AwaitingActions`, which is what the Python `step()` API expects).
    pub fn step_turn(&self, state: &mut BattleState) -> Result<(), SimError> {
        if state.winner.is_some() || state.phase == state::Phase::Finished {
            return Ok(());
        }
        state.phase = state::Phase::Combat;
        battle::resolve_combat(state, &self.library, &self.mechanics);
        battle::end_turn(state, &self.mechanics);
        if state.winner.is_some() {
            return Ok(());
        }
        if state.turn >= state.config.max_turns {
            state.winner = Some(state::Winner::Draw);
            state.phase = state::Phase::Finished;
            return Ok(());
        }
        battle::begin_turn(state, &self.library, &self.mechanics, &self.scripts);
        Ok(())
    }

    pub fn state_hash(&self, state: &BattleState) -> u64 {
        hash::state_hash(state)
    }

    /// Hash covering the previous state, every action submitted this turn and
    /// the resulting state.
    pub fn transition_hash(
        &self,
        previous: &BattleState,
        actions: &[Action],
        next: &BattleState,
    ) -> u64 {
        #[derive(serde::Serialize)]
        struct Transition<'a> {
            previous: u64,
            actions: &'a [Action],
            next: u64,
        }
        let value = Transition {
            previous: hash::state_hash(previous),
            actions,
            next: hash::state_hash(next),
        };
        hash::state_hash(&value)
    }

    /// Target lookup helper used by the search layer.
    pub fn unit_id(&self, state: &BattleState, name: &str) -> Option<UnitId> {
        state.units.iter().find(|u| u.name == name).map(|u| u.id.clone())
    }

    /// The rules this build knows it does not implement, for reporting.
    pub fn unknown_rules(&self) -> Vec<&'static str> {
        vec![
            state::UnknownRule::SanityGainOnClash.text(),
            state::UnknownRule::CoinFlipRng.text(),
        ]
    }

    /// The same list as owned strings, including the Passive clauses that are
    /// not modelled yet (the Python binding and the CLI use this form).
    pub fn unknown_rules_owned(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .unknown_rules()
            .iter()
            .map(|s| s.to_string())
            .collect();
        out.extend(self.passive_gaps());
        out.extend(self.panics.gaps());
        out.sort();
        out.dedup();
        out
    }

    /// Passive clauses this project has not modelled, per passive id.
    pub fn passive_gaps(&self) -> Vec<String> {
        let mut out = Vec::new();
        for passive in self.passives.passives.values() {
            for clause in &passive.effects.unmodeled {
                out.push(format!("passive {}: {clause}", passive.id));
            }
        }
        out.sort();
        out.dedup();
        out
    }

    pub fn strict_blockers(&self) -> Vec<String> {
        let mut out = self.library.strict_blockers();
        for (id, mech) in &self.mechanics.skills {
            for line in &mech.unmodeled {
                out.push(format!("mechanics {id}: {line}"));
            }
        }
        out
    }
}
