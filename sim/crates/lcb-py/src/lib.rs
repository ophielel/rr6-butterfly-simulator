//! PyO3 bindings (`lcb_sim`).  Thin wrapper over `lcb-core`: the Rust side owns
//! the rules, Python owns search, training and analysis.
//!
//! The training interface is `step_turn(plan)`: submit one **complete** turn plan,
//! let the simulator validate it against its own legal-action list, resolve the
//! turn, and return the `StepResult` (hashes, RNG continuation, battle log delta
//! and the turn's real statistics).  `submit`/`step` are the older incremental
//! API and stay available for compatibility.

use lcb_core::battle::Action;
use lcb_core::setup::fixed;
use lcb_core::state::{BattleConfig, BattleState, Phase, UnitKind};
use lcb_core::Simulator;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;

#[pyclass]
pub struct PySimulator {
    sim: Arc<Simulator>,
    state: Option<BattleState>,
}

#[pymethods]
impl PySimulator {
    #[new]
    #[pyo3(signature = (data_dir=None))]
    fn new(data_dir: Option<String>) -> PyResult<Self> {
        let root = data_dir.map(PathBuf::from).unwrap_or_else(default_data_dir);
        let sim = Simulator::from_data_dir(&root).map_err(to_py_err)?;
        Ok(Self {
            sim: Arc::new(sim),
            state: None,
        })
    }

    /// `reset(seed, team=None, enemies=None, strict=False, max_turns=None,
    /// enemy_hp_scale=None)` - start a fresh encounter.  Returns the initial
    /// state hash.  `enemy_hp_scale` is a **scenario** knob for the training
    /// harness (1.0 == the encounter as the data describes it).
    #[pyo3(signature = (seed, team=None, enemies=None, strict=false, max_turns=None, enemy_hp_scale=None))]
    fn reset(
        &mut self,
        seed: u64,
        team: Option<Vec<String>>,
        enemies: Option<Vec<String>>,
        strict: bool,
        max_turns: Option<u32>,
        enemy_hp_scale: Option<f64>,
    ) -> PyResult<String> {
        let team = team.unwrap_or_else(|| fixed::TEAM.iter().map(|s| s.to_string()).collect());
        let enemies = enemies.unwrap_or_else(|| {
            fixed::SECTION5_WAVE
                .iter()
                .map(|s| s.to_string())
                .collect()
        });
        let mut config = BattleConfig {
            strict_mechanics: strict,
            ..Default::default()
        };
        if let Some(turns) = max_turns {
            config.max_turns = turns;
        }
        if let Some(scale) = enemy_hp_scale {
            config.enemy_hp_scale = scale;
        }
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

    /// Legal actions for the current phase, as a JSON array.  This is the single
    /// action mask: search, training and the evaluation all use it.
    fn legal_actions(&self) -> PyResult<String> {
        let state = self.state_ref()?;
        let actions = self.sim.legal_actions(state);
        serde_json::to_string(&actions).map_err(to_py_err)
    }

    /// Submit one action without resolving the turn (lean path for the search
    /// layer: no state clone, no hashing).
    fn submit(&mut self, action_json: &str) -> PyResult<String> {
        let action: Action = serde_json::from_str(action_json).map_err(to_py_err)?;
        let mut state = self
            .state
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("call reset() first"))?;
        let result = self.sim.submit(&mut state, action);
        let out = match &result {
            Ok(()) => serde_json::json!({"ok": true, "error": null}),
            Err(err) => serde_json::json!({"ok": false, "error": err.to_string()}),
        };
        let ok = result.is_ok();
        self.state = Some(state);
        if ok {
            serde_json::to_string(&out).map_err(to_py_err)
        } else {
            // The caller is expected to abort the turn; the state is untouched
            // except for the rejected submission, which `submit` does not apply.
            serde_json::to_string(&out).map_err(to_py_err)
        }
    }

    /// Legacy incremental API: submit one action and commit when the argument is
    /// `None` / `null` / `"null"`.  Returns hashes and the transition hash.
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

    /// **The training step.**  `plan` is a JSON array of actions; every action is
    /// validated against the simulator's own legal-action list, the plan has to
    /// cover every unit that can act, and the turn only resolves when the whole
    /// plan has been accepted (an illegal or partial plan changes nothing and is
    /// reported in `error`).
    ///
    /// Returns the `StepResult`:
    /// ```json
    /// {"ok": true, "error": null, "turn": 2, "phase": "AwaitingActions",
    ///  "winner": null, "state_hash_before": "...", "state_hash_after": "...",
    ///  "transition_hash": "...", "search_key": "...", "seed": 7,
    ///  "rng_before": {"draw_count": 12, "state": [...]},
    ///  "rng_after": {...}, "plan": [...], "log": [{"turn":1,"kind":"...","detail":"..."}],
    ///  "stats": {"damage_to_enemies": 40, ...}}
    /// ```
    fn step_turn(&mut self, plan_json: &str) -> PyResult<String> {
        let plan: Vec<Action> = serde_json::from_str(plan_json).map_err(to_py_err)?;
        let mut state = self
            .state
            .take()
            .ok_or_else(|| PyRuntimeError::new_err("call reset() first"))?;
        let before = state.clone();
        let log_from = before.log.len();
        let outcome = self.sim.submit_plan(&mut state, plan.clone());
        let (ok, error) = match &outcome {
            Ok(()) => (true, None),
            Err(err) => (false, Some(err.to_string())),
        };
        let result = if ok {
            let transition = self.sim.transition_hash(&before, &plan, &state);
            serde_json::json!({
                "ok": true,
                "error": null,
                "turn": state.turn,
                "phase": format!("{:?}", state.phase),
                "winner": state.winner,
                "state_hash_before": format!("{:016x}", self.sim.state_hash(&before)),
                "state_hash_after": format!("{:016x}", self.sim.state_hash(&state)),
                "transition_hash": format!("{transition:016x}"),
                "search_key": format!("{:016x}", self.sim.search_key(&state)),
                "seed": state.seed,
                "rng_before": serde_json::to_value(&before.rng).unwrap_or(serde_json::Value::Null),
                "rng_after": serde_json::to_value(&state.rng).unwrap_or(serde_json::Value::Null),
                "plan": &plan,
                "log": &state.log[log_from.min(state.log.len())..],
                "stats": serde_json::to_value(&state.turn_stats).unwrap_or(serde_json::Value::Null),
            })
        } else {
            serde_json::json!({
                "ok": false,
                "error": error,
                "turn": before.turn,
                "phase": format!("{:?}", before.phase),
                "winner": before.winner,
                "state_hash_before": format!("{:016x}", self.sim.state_hash(&before)),
                "state_hash_after": format!("{:016x}", self.sim.state_hash(&state)),
                "log": [],
                "stats": serde_json::Value::Null,
            })
        };
        self.state = Some(state);
        serde_json::to_string(&result).map_err(to_py_err)
    }

    /// Full state as JSON (analysis / debugging); hashes stay authoritative.
    fn state_json(&self) -> PyResult<String> {
        serde_json::to_string(self.state_ref()?).map_err(to_py_err)
    }

    /// Compact observation for the encoders: everything that can affect a
    /// decision, without the battle log (whose length must not leak into the
    /// features).
    fn observation_json(&self) -> PyResult<String> {
        let state = self.state_ref()?;
        let units: Vec<serde_json::Value> = state
            .units
            .iter()
            .map(|unit| {
                let (kind, identity, enemy, part) = match &unit.kind {
                    UnitKind::Sinner { identity } => {
                        ("sinner", Some(identity.0.clone()), None, None)
                    }
                    UnitKind::Abnormality { enemy, part } => {
                        ("enemy", None, Some(enemy.0.clone()), part.clone())
                    }
                };
                let statuses: serde_json::Map<String, serde_json::Value> = unit
                    .statuses
                    .iter()
                    .map(|(name, instance)| {
                        (
                            name.clone(),
                            serde_json::json!({
                                "potency": instance.potency,
                                "count": instance.count,
                                "stack": instance.stack,
                            }),
                        )
                    })
                    .collect();
                let dashboard: Vec<serde_json::Value> = unit
                    .dashboard
                    .iter()
                    .map(|slot| {
                        serde_json::json!({
                            "slot": slot.slot,
                            "current": slot.current.0,
                            "next": slot.next.0,
                            "preview": slot.preview.0,
                            "target": slot.target,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "id": unit.id.0,
                    "name": unit.name,
                    "kind": kind,
                    "identity": identity,
                    "enemy": enemy,
                    "part": part,
                    "level": unit.level,
                    "hp": unit.hp,
                    "max_hp": unit.max_hp,
                    "shield": unit.shield,
                    "alive": unit.alive,
                    "speed": unit.speed,
                    "speed_range": [unit.speed_range.0, unit.speed_range.1],
                    "staggered": unit.is_staggered(),
                    "stagger_turns": unit.stagger.turns_remaining,
                    "stagger_level": unit.stagger.level,
                    "stagger_thresholds": unit.stagger.thresholds_hp,
                    "resist_sin": unit.resist_sin,
                    "resist_physical": unit.resist_physical,
                    "sp": match unit.sanity {
                        lcb_core::state::Sanity::Sane { sp } => Some(sp),
                        lcb_core::state::Sanity::None => None,
                    },
                    "low_morale": unit.low_morale,
                    "panicked": unit.panicked,
                    "corroded": unit.corroded,
                    "statuses": statuses,
                    "dashboard": dashboard,
                    "ego_slots": unit.ego_slots.iter().map(|e| e.0.clone()).collect::<Vec<_>>(),
                    "passive_ids": unit.passive_ids,
                    "skill_kills": unit.skill_kills,
                })
            })
            .collect();
        let observation = serde_json::json!({
            "turn": state.turn,
            "phase": format!("{:?}", state.phase),
            "max_turns": state.config.max_turns,
            "seed": state.seed,
            "rng": serde_json::to_value(&state.rng).unwrap_or(serde_json::Value::Null),
            "winner": state.winner,
            "encounter_ended": state.encounter_ended,
            "slot_target": state.slot_target,
            "ego_resources": state.ego_resources,
            "resonance": state.resonance,
            "a_resonance": state.a_resonance,
            "campaign": serde_json::to_value(&state.campaign).unwrap_or(serde_json::Value::Null),
            "units": units,
        });
        serde_json::to_string(&observation).map_err(to_py_err)
    }

    fn state_hash(&self) -> PyResult<String> {
        Ok(format!("{:016x}", self.sim.state_hash(self.state_ref()?)))
    }

    /// Transposition key: the state hash without the log/warnings/statistics.
    fn search_key(&self) -> PyResult<String> {
        Ok(format!("{:016x}", self.sim.search_key(self.state_ref()?)))
    }

    /// Deep copy of the state; the returned simulator is independent and shares
    /// the (immutable) rule books with this one.
    fn clone_state(&self) -> PyResult<PySimulator> {
        let state = self.state_ref()?.clone();
        Ok(PySimulator {
            sim: Arc::clone(&self.sim),
            state: Some(state),
        })
    }

    /// Rules known to be unimplemented / unsourced in this build.
    fn unknown_rules(&self) -> Vec<String> {
        self.sim.unknown_rules_owned()
    }

    /// Status clauses this project has not modelled yet (fixed content only).
    fn status_gaps(&self) -> Vec<String> {
        self.sim.status_gaps()
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
    m.add("SECTION5_WAVE", fixed::SECTION5_WAVE.to_vec())?;
    m.add(
        "EGO_LOADOUT",
        fixed::EGO_LOADOUT
            .iter()
            .map(|(identity, ego)| vec![identity.to_string(), ego.to_string()])
            .collect::<Vec<_>>(),
    )?;
    m.add("PHASES", vec![format!("{:?}", Phase::AwaitingActions)])?;
    Ok(())
}
