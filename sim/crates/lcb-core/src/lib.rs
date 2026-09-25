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
use effects::{MechanicsBook, PanicBook, PassiveBook, StatusBook};
use ids::EgoId;
use ids::SkillId;
use ids::UnitId;
use library::{Library, LibraryError};
use setup::{EncounterBuilder, SetupError};
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
    /// Behaviour of each status (parsed from its own text).
    pub statuses: StatusBook,
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

/// Canonical JSON encoding of an action, used to compare a submitted action with
/// the simulator's own legal-action list.
fn serialize_action(action: &Action) -> String {
    serde_json::to_string(action).unwrap_or_default()
}

impl Simulator {
    /// Load `data/` (identities, ego, enemies, statuses) and
    /// `data/mechanics/effects.json`.
    pub fn from_data_dir(root: impl AsRef<Path>) -> Result<Self, SimError> {
        let root = root.as_ref().to_path_buf();
        let library = Library::load(&root)?;
        let mechanics = MechanicsBook::load(&root.join("mechanics").join("effects.json"))
            .map_err(SimError::Mechanics)?;
        let scripts =
            scripts::ScriptsBook::load(&root.join("mechanics").join("enemy_scripts.json"))
                .map_err(SimError::Mechanics)?;
        let passives = PassiveBook::load(&root.join("passives").join("passives.json"))
            .map_err(SimError::Mechanics)?;
        let panics = PanicBook::load(&root.join("mechanics").join("panic_types.json"))
            .map_err(SimError::Mechanics)?;
        let statuses = StatusBook::load(&root.join("mechanics").join("status_effects.json"))
            .map_err(SimError::Mechanics)?;
        Ok(Self {
            library,
            mechanics,
            scripts,
            passives,
            panics,
            statuses,
            data_root: root,
        })
    }

    fn strict_core_preflight(&self, state: &BattleState) -> Vec<String> {
        let mut blockers = Vec::new();
        let critical_egos = ["20106", "20903"];
        for unit in &state.units {
            for ego in &unit.ego_slots {
                if !critical_egos.contains(&ego.as_str()) {
                    continue;
                }
                for kind in ["awakening", "corrosion"] {
                    let id = SkillId::new(format!("{}.{}", ego.as_str(), kind));
                    match self.mechanics.get_for(&id, state.config.uptie) {
                        None => {
                            blockers.push(format!("missing critical mechanics {}", id.as_str()))
                        }
                        Some(mechanics) => {
                            blockers.extend(
                                mechanics
                                    .unmodeled
                                    .iter()
                                    .map(|line| format!("{}: {}", id.as_str(), line)),
                            );
                            let has_sinking = |potency: i32, count: i32| {
                                let mut effects = mechanics.coins.values().flatten();
                                let has_potency = effects.clone().any(|effect| {
                                    effect.kind == "inflict"
                                        && effect.status.as_deref() == Some("Sinking")
                                        && effect.potency == Some(potency)
                                });
                                let has_count = effects.any(|effect| {
                                    effect.kind == "inflict"
                                        && effect.status.as_deref() == Some("Sinking")
                                        && effect.count == Some(count)
                                });
                                has_potency && has_count
                            };
                            if ego.as_str() == "20106" && kind == "awakening"
                                && !mechanics.attack_end.iter().any(|effect| {
                                    effect.kind == "inflict_random_each"
                                        && effect.status.as_deref() == Some("Sinking")
                                        && effect.value == Some(6)
                                        && effect.multiplier_f == Some(1.5)
                                })
                            {
                                blockers.push("20106.awakening: missing 6 + 1.5 Gloom Sinking events".into());
                            }
                            if ego.as_str() == "20903" && !has_sinking(
                                if kind == "awakening" { 5 } else { 10 },
                                if kind == "awakening" { 5 } else { 8 },
                            ) {
                                blockers.push(format!("{}: missing fixed Sinking Potency/Count", id.as_str()));
                            }
                        },
                    }
                }
            }
        }
        for ego in ["20106", "20903"] {
            let record = self.library.ego(&EgoId::new(ego));
            for kind in ["awakening", "corrosion"] {
                let weight = record.and_then(|entry| {
                    if kind == "awakening" {
                        entry.awakening.as_ref()
                    } else {
                        entry.corrosion.as_ref()
                    }
                }).and_then(|skill| skill.attack_weight);
                if ego == "20903" && weight != Some(3) {
                    blockers.push(format!("{}.{}: expected Attack Weight 3, got {:?}", ego, kind, weight));
                }
            }
        }
        blockers.sort();
        blockers.dedup();
        blockers
    }

    pub fn new_encounter(
        &self,
        team: &[&str],
        enemies: &[&str],
        seed: u64,
        config: BattleConfig,
    ) -> Result<BattleState, SimError> {
        let state = EncounterBuilder::new(&self.library, &self.mechanics)
            .books(&self.passives, &self.panics, &self.statuses)
            .scripts(&self.scripts)
            .seed(seed)
            .config(config)
            .build(team, enemies)?;
        if state.config.strict_mechanics {
            let blockers = self.strict_core_preflight(&state);
            if !blockers.is_empty() {
                return Err(SetupError::StrictBlockers(blockers).into());
            }
        }
        Ok(state)
    }

    /// Attach the rules books to a freshly built encounter (Turn 2 onwards this
    /// is already done by the builder, but `section5` and re-loads need it).
    fn attach_passives(&self, state: &mut state::BattleState) {
        state.status_book = Some(std::sync::Arc::new(self.statuses.clone()));
        setup::attach_unit_books(state, &self.passives, Some(&self.panics), &self.library);
    }

    pub fn section5(
        &self,
        campaign: &state::CampaignState,
        seed: u64,
        config: BattleConfig,
    ) -> Result<BattleState, SimError> {
        // The Section 5 wave is the Imago together with its three Illusory
        // Butterfly allies.
        let mut built = self.new_encounter(
            &setup::fixed::TEAM,
            &setup::fixed::SECTION5_WAVE,
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

    /// Submit a **complete** turn plan and resolve it.
    ///
    /// Every action is validated against the simulator's own legal-action list
    /// (the same list the search and the policy use as their mask), the plan has
    /// to cover every unit that can still act, and nothing is committed if any
    /// of that fails: the state is left exactly as it was.  This is the atomic
    /// `reset -> choose a full plan -> one resolution` step the plan requires.
    pub fn submit_plan(&self, state: &mut BattleState, plan: Vec<Action>) -> Result<(), SimError> {
        if state.winner.is_some() || state.phase == state::Phase::Finished {
            return Err(SimError::Rule("the encounter is over".to_string()));
        }
        let snapshot = state.clone();
        let outcome = self.submit_plan_inner(state, plan);
        if outcome.is_err() {
            *state = snapshot;
        }
        outcome
    }

    fn submit_plan_inner(
        &self,
        state: &mut BattleState,
        plan: Vec<Action>,
    ) -> Result<(), SimError> {
        let mut submitted: Vec<UnitId> = Vec::new();
        for action in plan.iter() {
            if matches!(action, Action::Commit) {
                return Err(SimError::Rule(
                    "a plan must not contain Commit; the plan itself resolves the turn".to_string(),
                ));
            }
            let actor = match action {
                Action::Assign { actor, .. }
                | Action::Engage { actor, .. }
                | Action::UseEgo { actor, .. } => actor.clone(),
                Action::Commit => unreachable!(),
            };
            if submitted.contains(&actor) {
                return Err(SimError::Rule(format!("duplicate action for {actor}")));
            }
            let legal = self.legal_actions(state);
            let key = serialize_action(action);
            if !legal.iter().any(|a| serialize_action(a) == key) {
                return Err(SimError::Rule(format!("illegal action in plan: {key}")));
            }
            self.submit(state, action.clone())?;
            submitted.push(actor);
        }
        // Every unit that can still act must have acted, or the plan is partial.
        for action in self.legal_actions(state) {
            let actor = match &action {
                Action::Assign { actor, .. }
                | Action::Engage { actor, .. }
                | Action::UseEgo { actor, .. } => actor.clone(),
                Action::Commit => continue,
            };
            if !submitted.contains(&actor) {
                return Err(SimError::Rule(format!(
                    "incomplete plan: {actor} has no action"
                )));
            }
        }
        self.step_turn(state)
    }

    /// Commit the actions assigned for this turn: resolves clashes and attacks,
    /// runs turn end, then starts the next turn (so the state is again in
    /// `AwaitingActions`, which is what the Python `step()` API expects).
    pub fn step_turn(&self, state: &mut BattleState) -> Result<(), SimError> {
        if state.winner.is_some() || state.phase == state::Phase::Finished {
            return Ok(());
        }
        // Per-turn statistics describe the turn that is about to resolve.
        state.turn_stats = state::TurnStats::default();
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

    /// Hash for search / transposition (ignores the log, warnings and the
    /// per-turn statistics, which cannot affect the future).
    pub fn search_key(&self, state: &BattleState) -> u64 {
        hash::search_key(state)
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
        state
            .units
            .iter()
            .find(|u| u.name == name)
            .map(|u| u.id.clone())
    }

    /// The rules this build knows it does not implement, for reporting.

    /// Passive clauses this project has not modelled, per passive id.
    pub fn unknown_rules(&self) -> Vec<&'static str> {
        vec![
            state::UnknownRule::SanityGainOnClash.text(),
            state::UnknownRule::CoinFlipRng.text(),
        ]
    }

    /// The same list as owned strings, including the Passive clauses that are
    /// not modelled yet (the Python binding and the CLI use this form).
    pub fn unknown_rules_owned(&self) -> Vec<String> {
        let mut out: Vec<String> = self.unknown_rules().iter().map(|s| s.to_string()).collect();
        out.extend(self.passive_gaps());
        out.extend(self.panics.gaps());
        out.extend(self.status_gaps());
        out.sort();
        out.dedup();
        out
    }

    /// Status clauses this project has not modelled, limited to the statuses the
    /// fixed content references.
    pub fn status_gaps(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (name, entry) in &self.statuses.statuses {
            if !entry.used_by_fixed_content {
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
