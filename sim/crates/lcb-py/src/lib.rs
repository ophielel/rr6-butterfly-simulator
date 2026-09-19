//! PyO3 bindings (`lcb_sim`).  Thin wrapper over `lcb-core`: the Rust side owns
//! the rules, Python owns search and analysis.

use lcb_core::battle::Action;
use lcb_core::setup::fixed;
use lcb_core::state::{BattleConfig, BattleState};
use lcb_core::Simulator;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use std::path::PathBuf;

#[pyclass]
pub struct PySimulator {
    sim: Simulator,
    state: Option<BattleState>,
}

#[pymethods]
impl PySimulator {
    #[new]
    #[pyo3(signature = (data_dir=None))]
    fn new(data_dir: Option<String>) -> PyResult<Self> {
        let root = data_dir.map(PathBuf::from).unwrap_or_else(default_data_dir);
        let sim = Simulator::from_data_dir(&root).map_err(to_py_err)?;
        Ok(Self { sim, state: None })
    }

    /// `reset(seed, team=None, enemies=None, strict=False)` - start a fresh
    /// encounter. Returns the initial state hash.
    #[pyo3(signature = (seed, team=None, enemies=None, strict=false))]
    fn reset(
        &mut self,
        seed: u64,
        team: Option<Vec<String>>,
        enemies: Option<Vec<String>>,
        strict: bool,
    ) -> PyResult<String> {
        let team = team.unwrap_or_else(|| fixed::TEAM.iter().map(|s| s.to_string()).collect());
        let enemies = enemies.unwrap_or_else(|| vec![fixed::BOSS_IMAGO.to_string()]);
        let config = BattleConfig {
            strict_mechanics: strict,
            ..Default::default()
        };
        let team_refs: Vec<&str> = team.iter().map(|s| s.as_str()).collect();
        let enemy_refs: Vec<&str> = enemies.iter().map(|s| s.as_str()).collect();
        let state = self
            .sim
            .new_encounter(&team_refs, &enemy_refs, seed, config)
            .map_err(to_py_err)?;
        let hash = format!("{:016x}", self.sim.state_hash(&state));
        self.state = Some(state);
        Ok(hash)
    }

    /// Legal actions for the current phase, as a JSON array.
    fn legal_actions(&self) -> PyResult<String> {
        let state = self.state_ref()?;
        let actions = self.sim.legal_actions(state);
        serde_json::to_string(&actions).map_err(to_py_err)
    }

    /// Submit one action (JSON string) or commit the turn when `action_json` is
    /// `None`/`null`/`"null"`. Returns a JSON result with hashes.
    fn step(&mut self, action_json: Option<String>) -> PyResult<String> {
        let mut state = self
            .state
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("call reset() first"))?;
        let before = state.clone();
        let mut submitted: Vec<Action> = Vec::new();
        let outcome = (|| -> Result<(), String> {
            if let Some(json) = action_json.as_ref() {
                if json.trim() != "null" {
                    let action: Action = serde_json::from_str(json).map_err(|e| e.to_string())?;
                    let commit = matches!(action, Action::Commit);
                    self.sim
                        .submit(&mut state, action.clone())
                        .map_err(|e| e.to_string())?;
                    submitted.push(action);
                    if !commit {
                        return Ok(());
                    }
                }
            }
            self.sim.step_turn(&mut state).map_err(|e| e.to_string())
        })();
        let transition = self.sim.transition_hash(&before, &submitted, &state);
        let state_hash = format!("{:016x}", self.sim.state_hash(&state));
        let result = serde_json::json!({
            "turn": state.turn,
            "phase": format!("{:?}", state.phase),
            "winner": state.winner,
            "state_hash": state_hash,
            "transition_hash": format!("{transition:016x}"),
            "ok": outcome.is_ok(),
            "error": outcome.err(),
        });
        self.state = Some(state);
        serde_json::to_string(&result).map_err(to_py_err)
    }

    /// Full state as JSON (analysis / debugging); hashes stay authoritative.
    fn state_json(&self) -> PyResult<String> {
        serde_json::to_string(self.state_ref()?).map_err(to_py_err)
    }

    fn state_hash(&self) -> PyResult<String> {
        Ok(format!("{:016x}", self.sim.state_hash(self.state_ref()?)))
    }

    /// Deep copy of the state; the returned simulator is independent.
    fn clone_state(&self) -> PyResult<PySimulator> {
        let state = self.state_ref()?.clone();
        Ok(PySimulator {
            sim: Simulator::from_data_dir(&self.sim.data_root).map_err(to_py_err)?,
            state: Some(state),
        })
    }

    /// Rules known to be unimplemented / unsourced in this build.
    fn unknown_rules(&self) -> Vec<String> {
        self.sim.unknown_rules_owned()
    }

    /// Passive clauses this project has not modelled yet.
    fn passive_gaps(&self) -> Vec<String> {
        self.sim.passive_gaps().iter().map(|s| s.to_string()).collect()
    }

    fn strict_blockers(&self) -> Vec<String> {
        self.sim.strict_blockers()
    }

    fn __repr__(&self) -> String {
        match &self.state {
            Some(state) => format!(
                "<PySimulator turn={} phase={:?} winner={:?}>",
                state.turn, state.phase, state.winner
            ),
            None => "<PySimulator (uninitialised)>".to_string(),
        }
    }
}

impl PySimulator {
    fn state_ref(&self) -> PyResult<&BattleState> {
        self.state
            .as_ref()
            .ok_or_else(|| PyRuntimeError::new_err("call reset() first"))
    }
}

fn default_data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("data")
}

fn to_py_err<E: std::fmt::Display>(err: E) -> PyErr {
    PyRuntimeError::new_err(err.to_string())
}

#[pymodule]
fn lcb_sim(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PySimulator>()?;
    m.add("TEAM", fixed::TEAM.to_vec())?;
    m.add("BOSS_IMAGO", fixed::BOSS_IMAGO)?;
    m.add("BOSS_PUPA", fixed::BOSS_PUPA)?;
    Ok(())
}
