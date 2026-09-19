//! `lcb-cli` - command line and JSON-line driver for `lcb-core`.
//!
//! Subcommands:
//!   inspect                      - library coverage, unknown rules, strict blockers
//!   run --seed N [--turns N]     - play the fixed team vs the Imago with a
//!                                  deterministic "attack the lowest HP enemy"
//!                                  policy and print a turn-by-turn summary
//!   serve                        - read JSON commands on stdin, write JSON on
//!                                  stdout (the fallback Python transport)

use lcb_core::battle::Action;
use lcb_core::ids::SkillId;
use lcb_core::setup::fixed;
use lcb_core::state::{BattleConfig, BattleState, Phase};
use lcb_core::Simulator;
use std::io::{BufRead, Write};
use std::path::PathBuf;

fn data_root() -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--data" {
            if let Some(value) = args.next() {
                return PathBuf::from(value);
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("data")
}

fn usage() -> ! {
    eprintln!(
        "usage: lcb-cli [--data DIR] <inspect|run|serve> [--seed N] [--turns N] [--team id,id] [--enemies id,id]"
    );
    std::process::exit(2);
}

fn main() {
    let root = data_root();
    let mut mode: Option<String> = None;
    let mut seed: u64 = 1;
    let mut turns: u32 = 6;
    let mut team: Vec<String> = fixed::TEAM.iter().map(|s| s.to_string()).collect();
    let mut enemies: Vec<String> = vec![fixed::BOSS_IMAGO.to_string()];

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data" => {
                args.next();
            }
            "--seed" => seed = args.next().and_then(|v| v.parse().ok()).unwrap_or(1),
            "--turns" => turns = args.next().and_then(|v| v.parse().ok()).unwrap_or(6),
            "--team" => {
                team = args
                    .next()
                    .map(|v| v.split(',').map(|s| s.to_string()).collect())
                    .unwrap_or(team)
            }
            "--enemies" => {
                enemies = args
                    .next()
                    .map(|v| v.split(',').map(|s| s.to_string()).collect())
                    .unwrap_or(enemies)
            }
            other => {
                if mode.is_none() {
                    mode = Some(other.to_string());
                } else {
                    usage();
                }
            }
        }
    }

    let sim = match Simulator::from_data_dir(&root) {
        Ok(sim) => sim,
        Err(err) => {
            eprintln!("failed to load data from {}: {err}", root.display());
            std::process::exit(1);
        }
    };

    match mode.as_deref().unwrap_or("inspect") {
        "inspect" => inspect(&sim),
        "run" => run(&sim, seed, turns, &team, &enemies),
        "serve" => serve(&sim),
        _ => usage(),
    }
}

fn inspect(sim: &Simulator) {
    println!("data root            : {}", sim.data_root.display());
    println!("identities           : {}", sim.library.identities.len());
    println!("ego                  : {}", sim.library.egos.len());
    println!("enemies              : {}", sim.library.enemies.len());
    println!("statuses             : {}", sim.library.statuses.len());
    println!("mechanics entries    : {}", sim.mechanics.skills.len());
    println!("--- unknown rules (documented, not guessed) ---");
    for rule in sim.unknown_rules() {
        println!("  * {rule}");
    }
    let blockers = sim.strict_blockers();
    println!("--- strict-mode blockers: {} ---", blockers.len());
    for blocker in blockers.iter().take(40) {
        println!("  * {blocker}");
    }
}

/// Deterministic stand-in policy: every slot attacks the living enemy with the
/// lowest HP using the first legal skill for that slot.  Search proper lives in
/// `python/`; this only exists so the CLI can produce a replay.
fn policy(state: &BattleState, sim: &Simulator) -> Vec<Action> {
    let actions = sim.legal_actions(state);
    let mut chosen: Vec<Action> = Vec::new();
    let enemies = state.living_enemies();
    let target = enemies
        .iter()
        .min_by_key(|id| state.unit(id).map(|u| u.hp).unwrap_or(i32::MAX))
        .cloned();
    for action in actions {
        if let Action::Assign {
            actor,
            slot,
            skill,
            target: _,
        } = &action
        {
            if chosen
                .iter()
                .any(|a| matches!(a, Action::Assign { actor: a2, .. } if a2 == actor))
            {
                continue;
            }
            if let Some(target) = target.clone() {
                chosen.push(Action::Assign {
                    actor: actor.clone(),
                    slot: *slot,
                    skill: skill.clone(),
                    target,
                });
            }
        }
    }
    chosen
}

fn run(sim: &Simulator, seed: u64, turns: u32, team: &[String], enemies: &[String]) {
    let config = BattleConfig {
        strict_mechanics: false,
        ..Default::default()
    };
    let mut state = match sim.new_encounter(
        &team.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        &enemies.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        seed,
        config,
    ) {
        Ok(state) => state,
        Err(err) => {
            eprintln!("encounter setup failed: {err}");
            std::process::exit(1);
        }
    };
    for _ in 0..turns {
        if state.winner.is_some() {
            break;
        }
        print_turn(&state);
        for action in policy(&state, sim) {
            let _ = sim.submit(&mut state, action);
        }
        let before = state.log.len();
        if let Err(err) = sim.step_turn(&mut state) {
            eprintln!("turn failed: {err}");
            break;
        }
        for entry in &state.log[before..] {
            println!("    [{}] {}", entry.kind, entry.detail);
        }
    }
    println!("--- warnings ({} unique) ---", state.warnings.len());
    for warning in state.warnings.iter().take(12) {
        println!("  * {warning}");
    }
    println!("state hash: {:016x}", sim.state_hash(&state));
}

fn print_turn(state: &BattleState) {
    println!("turn {} (phase {:?})", state.turn, state.phase);
    for unit in &state.units {
        println!(
            "  {:<34} HP {}/{} SP {} speed {} {}",
            unit.name,
            unit.hp,
            unit.max_hp,
            unit.sanity.sp(),
            unit.speed,
            if unit.is_staggered() { "STAGGERED" } else { "" }
        );
    }
}

// --------------------------------------------------------------------------- //
// JSON line protocol (fallback transport for the Python environment)
// --------------------------------------------------------------------------- //

fn serve(sim: &Simulator) {
    let stdin = std::io::stdin();
    let mut state: Option<BattleState> = None;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let request: serde_json::Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(err) => {
                println!("{}", serde_json::json!({"error": err.to_string()}));
                continue;
            }
        };
        let cmd = request.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
        let response = match cmd {
            "reset" => {
                let seed = request.get("seed").and_then(|v| v.as_u64()).unwrap_or(1);
                let team: Vec<String> = request
                    .get("team")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_else(|| fixed::TEAM.iter().map(|s| s.to_string()).collect());
                let enemies: Vec<String> = request
                    .get("enemies")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                    .unwrap_or_else(|| vec![fixed::BOSS_IMAGO.to_string()]);
                let config = BattleConfig {
                    strict_mechanics: request
                        .get("strict")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    ..Default::default()
                };
                match sim.new_encounter(
                    &team.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                    &enemies.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                    seed,
                    config,
                ) {
                    Ok(new_state) => {
                        let hash = sim.state_hash(&new_state);
                        state = Some(new_state);
                        serde_json::json!({"ok": true, "state_hash": format!("{hash:016x}")})
                    }
                    Err(err) => serde_json::json!({"error": err.to_string()}),
                }
            }
            "legal_actions" => match &state {
                Some(state) => {
                    let actions = sim.legal_actions(state);
                    serde_json::json!({"ok": true, "actions": actions})
                }
                None => serde_json::json!({"error": "not initialised"}),
            },
            "step" => match &mut state {
                Some(state) => {
                    let before = state.clone();
                    if let Some(action) = request.get("action") {
                        match serde_json::from_value::<Action>(action.clone()) {
                            Ok(action) => {
                                if let Err(err) = sim.submit(state, action) {
                                    println!("{}", serde_json::json!({"error": err.to_string()}));
                                    continue;
                                }
                            }
                            Err(err) => {
                                println!("{}", serde_json::json!({"error": err.to_string()}));
                                continue;
                            }
                        }
                    }
                    match sim.step_turn(state) {
                        Ok(()) => {
                            let transition = sim.transition_hash(&before, &[], state);
                            serde_json::json!({
                                "ok": true,
                                "turn": state.turn,
                                "state_hash": format!("{:016x}", sim.state_hash(state)),
                                "transition_hash": format!("{transition:016x}"),
                                "winner": state.winner,
                                "phase": format!("{:?}", state.phase),
                            })
                        }
                        Err(err) => serde_json::json!({"error": err.to_string()}),
                    }
                }
                None => serde_json::json!({"error": "not initialised"}),
            },
            "load_state" => match request.get("state").cloned() {
                Some(value) => match serde_json::from_value::<BattleState>(value) {
                    Ok(restored) => {
                        let hash = sim.state_hash(&restored);
                        state = Some(restored);
                        serde_json::json!({"ok": true, "state_hash": format!("{hash:016x}")})
                    }
                    Err(err) => serde_json::json!({"error": err.to_string()}),
                },
                None => serde_json::json!({"error": "missing state"}),
            },
            "state" => match &state {
                Some(state) => serde_json::json!({"ok": true, "state": state}),
                None => serde_json::json!({"error": "not initialised"}),
            },
            "clone" => match state.clone() {
                Some(current) => {
                    let cloned = lcb_core::replay::clone_state(&current);
                    let same = sim.state_hash(&cloned) == sim.state_hash(&current);
                    state = Some(cloned);
                    serde_json::json!({"ok": true, "isolated": same})
                }
                None => serde_json::json!({"error": "not initialised"}),
            },
            "state_hash" => match &state {
                Some(state) => {
                    serde_json::json!({"ok": true, "state_hash": format!("{:016x}", sim.state_hash(state))})
                }
                None => serde_json::json!({"error": "not initialised"}),
            },
            "unknown_rules" => serde_json::json!({"ok": true, "rules": sim.unknown_rules()}),
            "strict_blockers" => serde_json::json!({"ok": true, "blockers": sim.strict_blockers()}),
            "skill_ids" => {
                let ids: Vec<SkillId> = sim
                    .library
                    .identities
                    .values()
                    .flat_map(|i| i.skills.iter().map(|s| SkillId::new(s.id.clone())))
                    .collect();
                serde_json::json!({"ok": true, "skills": ids})
            }
            "" => continue,
            other => serde_json::json!({"error": format!("unknown command {other}")}),
        };
        println!("{response}");
        let _ = std::io::stdout().flush();
    }
    let _ = Phase::Setup;
}
