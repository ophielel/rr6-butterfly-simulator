//! Replays: a seed, the fixed content ids, and the actions submitted each turn,
//! plus a hash chain that proves a re-simulation reproduced the same states.

use crate::battle::Action;
use crate::hash;
use crate::library::Library;
use crate::state::{BattleConfig, Phase};
use serde::{Deserialize, Serialize};

/// One committed turn: everything that was submitted, and the hash it produced.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplayTurn {
    pub turn: u32,
    pub actions: Vec<Action>,
    pub state_hash: u64,
    pub transition_hash: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Replay {
    pub seed: u64,
    pub config: BattleConfig,
    pub team: Vec<String>,
    pub enemies: Vec<String>,
    pub steps: Vec<ReplayTurn>,
}

impl Replay {
    pub fn new(seed: u64, config: BattleConfig, team: &[&str], enemies: &[&str]) -> Self {
        Self {
            seed,
            config,
            team: team.iter().map(|s| s.to_string()).collect(),
            enemies: enemies.iter().map(|s| s.to_string()).collect(),
            steps: Vec::new(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    /// Re-run the replay and check every recorded hash.
    pub fn verify(&self, sim: &crate::Simulator) -> Result<(), String> {
        let team: Vec<&str> = self.team.iter().map(|s| s.as_str()).collect();
        let enemies: Vec<&str> = self.enemies.iter().map(|s| s.as_str()).collect();
        let mut state = sim
            .new_encounter(&team, &enemies, self.seed, self.config.clone())
            .map_err(|e| e.to_string())?;
        for (index, step) in self.steps.iter().enumerate() {
            if state.phase == Phase::Finished {
                return Err(format!("step {index}: replay continues after the battle ended"));
            }
            let before = state.clone();
            for action in &step.actions {
                sim.submit(&mut state, action.clone()).map_err(|e| e.to_string())?;
            }
            sim.step_turn(&mut state).map_err(|e| e.to_string())?;
            let next = hash::state_hash(&state);
            if next != step.state_hash {
                return Err(format!(
                    "step {index}: state hash mismatch (replay {:016x}, simulated {:016x})",
                    step.state_hash, next
                ));
            }
            let transition = sim.transition_hash(&before, &step.actions, &state);
            if transition != step.transition_hash {
                return Err(format!("step {index}: transition hash mismatch"));
            }
        }
        Ok(())
    }
}

/// Record a committed turn into the replay.
pub fn record(
    sim: &crate::Simulator,
    replay: &mut Replay,
    before: &crate::state::BattleState,
    actions: &[Action],
    after: &crate::state::BattleState,
) {
    let transition = sim.transition_hash(before, actions, after);
    replay.steps.push(ReplayTurn {
        turn: before.turn,
        actions: actions.to_vec(),
        state_hash: hash::state_hash(after),
        transition_hash: transition,
    });
}

/// Deep-copy a state (used by the Python `clone_state()` API and by tests that
/// assert clone isolation).
pub fn clone_state(state: &crate::state::BattleState) -> crate::state::BattleState {
    state.clone()
}

/// A fingerprint of the exact library a replay was produced against.
pub fn library_fingerprint(library: &Library) -> u64 {
    let ids: Vec<&String> = library
        .identities
        .keys()
        .chain(library.egos.keys())
        .chain(library.enemies.keys())
        .chain(library.statuses.keys())
        .collect();
    hash::state_hash(&ids)
}
