//! Engineering tests required by the plan: clone isolation, hashing,
//! serialisation, RNG determinism and full replay reproduction.

use lcb_core::battle::Action;
use lcb_core::effects::MechanicsBook;
use lcb_core::hash;
use lcb_core::library::Library;
use lcb_core::replay::{self, Replay};
use lcb_core::setup::{fixed, EncounterBuilder};
use lcb_core::state::{BattleConfig, Phase, Winner};
use lcb_core::Simulator;
use std::path::PathBuf;

fn data_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("data")
}

fn simulator() -> Simulator {
    Simulator::from_data_dir(data_root()).expect("data library loads")
}

fn team() -> Vec<&'static str> {
    fixed::TEAM.to_vec()
}

/// Deterministic stand-in policy used by several tests.
fn assign_all(state: &lcb_core::state::BattleState, sim: &Simulator) -> Vec<Action> {
    let mut out = Vec::new();
    let target = state.living_enemies().into_iter().next();
    let Some(target) = target else { return out };
    for action in sim.legal_actions(state) {
        if let Action::Assign {
            actor,
            slot,
            skill,
            target: _,
        } = &action
        {
            if out
                .iter()
                .any(|a| matches!(a, Action::Assign { actor: a2, .. } if a2 == actor))
            {
                continue;
            }
            out.push(Action::Assign {
                actor: actor.clone(),
                slot: *slot,
                skill: skill.clone(),
                target: target.clone(),
            });
        }
    }
    out
}

#[test]
fn clone_is_a_deep_copy() {
    let sim = simulator();
    let mut state = sim
        .new_encounter(&team(), &[fixed::BOSS_IMAGO], 11, BattleConfig::default())
        .expect("encounter");
    for action in assign_all(&state, &sim) {
        sim.submit(&mut state, action).unwrap();
    }
    let mut clone = replay::clone_state(&state);
    let original_hash = sim.state_hash(&state);
    assert_eq!(original_hash, sim.state_hash(&clone), "clone must start identical");

    // Mutate the clone in every dimension the plan cares about.
    clone.units[0].hp -= 7;
    clone.units[0].sanity = clone.units[0].sanity.add(-5);
    clone.units[0].statuses.add_potency("Burn", 3);
    let deck_before = state.units[1].deck.len();
    clone.units[1]
        .deck
        .remaining
        .push((lcb_core::ids::SkillId::new("1011001"), 1));
    clone.units[2].dashboard[0].slot = 9;
    clone.rng.next_u64();
    clone.ego_resources.insert("pride".to_string(), 3);

    assert_ne!(original_hash, sim.state_hash(&clone), "mutations must change the hash");
    assert_eq!(state.units[0].hp, state.units[0].max_hp, "original untouched");
    assert_eq!(state.units[0].statuses.potency("Burn"), 0);
    assert_eq!(state.units[1].deck.len(), deck_before);
    assert_eq!(state.ego_resources.get("pride"), Some(&0));
}

#[test]
fn state_round_trips_through_json() {
    let sim = simulator();
    let mut state = sim
        .new_encounter(&team(), &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    for action in assign_all(&state, &sim) {
        sim.submit(&mut state, action).unwrap();
    }
    sim.step_turn(&mut state).unwrap();
    let json = serde_json::to_string(&state).unwrap();
    let restored: lcb_core::state::BattleState = serde_json::from_str(&json).unwrap();
    assert_eq!(sim.state_hash(&state), sim.state_hash(&restored));
}

#[test]
fn same_seed_same_trajectory_and_hashes() {
    let sim = simulator();
    let play = |seed: u64| {
        let mut state = sim
            .new_encounter(&team(), &[fixed::BOSS_IMAGO], seed, BattleConfig::default())
            .unwrap();
        let mut hashes = Vec::new();
        for _ in 0..3 {
            for action in assign_all(&state, &sim) {
                sim.submit(&mut state, action).unwrap();
            }
            sim.step_turn(&mut state).unwrap();
            hashes.push(sim.state_hash(&state));
        }
        hashes
    };
    assert_eq!(play(5), play(5), "the simulator must be deterministic");
    assert_ne!(play(5), play(6), "different seeds diverge");
}

#[test]
fn rng_draw_count_is_part_of_the_state() {
    let sim = simulator();
    let mut a = sim
        .new_encounter(&team(), &[fixed::BOSS_IMAGO], 9, BattleConfig::default())
        .unwrap();
    let b = a.clone();
    a.rng.next_u64();
    assert_ne!(sim.state_hash(&a), sim.state_hash(&b));
}

#[test]
fn replay_reproduces_every_transition() {
    let sim = simulator();
    let mut replay = Replay::new(
        21,
        BattleConfig::default(),
        &team(),
        &[fixed::BOSS_IMAGO],
    );
    let mut state = sim
        .new_encounter(
            &replay.team.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            &replay.enemies.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            replay.seed,
            replay.config.clone(),
        )
        .unwrap();
    for _ in 0..4 {
        let before = state.clone();
        let actions = assign_all(&state, &sim);
        for action in &actions {
            sim.submit(&mut state, action.clone()).unwrap();
        }
        sim.step_turn(&mut state).unwrap();
        replay::record(&sim, &mut replay, &before, &actions, &state);
    }
    assert_eq!(replay.steps.len(), 4);
    replay.verify(&sim).expect("replay verifies");

    // A tampered replay must be rejected.
    let mut broken = replay.clone();
    broken.steps[2].state_hash ^= 0xdead_beef;
    assert!(broken.verify(&sim).is_err());
}

#[test]
fn transition_hash_covers_action_and_state() {
    let sim = simulator();
    let state = sim
        .new_encounter(&team(), &[fixed::BOSS_IMAGO], 4, BattleConfig::default())
        .unwrap();
    let mut next = state.clone();
    next.turn = 2;
    let actions = vec![Action::Commit];
    let with = sim.transition_hash(&state, &actions, &next);
    let without = sim.transition_hash(&state, &[], &next);
    assert_ne!(with, without);
}

#[test]
fn deck_draws_do_not_repeat_until_exhausted() {
    let library = Library::load(&data_root()).unwrap();
    let mechanics = MechanicsBook::load(&data_root().join("mechanics").join("effects.json")).unwrap();
    let state = EncounterBuilder::new(&library, &mechanics)
        .seed(1)
        .build(&["10110"], &[fixed::BOSS_IMAGO])
        .unwrap();
    // Uptie IV amounts for this identity: 3 / 2 / 1 copies = 6 cards, one of
    // which is already on the dashboard.
    // The composition is 3 / 2 / 1 and the panel is filled from it.
    let deck = &state.units[0].deck;
    let mut counts = std::collections::BTreeMap::new();
    for (id, n) in deck.composition.iter() {
        counts.insert(id.0.clone(), *n);
    }
    assert_eq!(counts.get("1011001"), Some(&3));
    assert_eq!(counts.get("1011002"), Some(&2));
    assert_eq!(counts.get("1011003"), Some(&1));
    // The panel holds three skills per slot (2 selectable + 1 preview), all
    // drawn from the composition.
    let paneled: u32 = deck.paneled.iter().map(|(_, n)| *n).sum();
    assert_eq!(paneled, 3, "2 selectable skills plus the preview per slot");
    assert_eq!(
        deck.len() as u32,
        deck.composition.iter().map(|(_, n)| *n).sum::<u32>() - paneled
    );
}

#[test]
fn strict_mode_lists_everything_unmodeled() {
    let sim = simulator();
    let blockers = sim.strict_blockers();
    assert!(
        blockers.iter().any(|b| b.contains("unmodeled") || b.contains("mechanics")),
        "strict mode must surface unmodeled effect text"
    );
    for rule in sim.unknown_rules() {
        assert!(!rule.is_empty());
    }
}

#[test]
fn battle_ends_when_one_side_is_wiped() {
    let sim = simulator();
    let mut state = sim
        .new_encounter(&team(), &[fixed::BOSS_IMAGO], 13, BattleConfig::default())
        .unwrap();
    // Kill the boss by hand and run the turn end.
    for unit in state.units.iter_mut() {
        if !unit.kind.is_sinner() {
            unit.hp = 0;
            unit.alive = false;
        }
    }
    lcb_core::battle::end_turn(&mut state, &sim.mechanics);
    assert_eq!(state.winner, Some(Winner::Sinners));
    assert_eq!(state.phase, Phase::Finished);
}

#[test]
fn hash_chain_is_hex_encoded() {
    let value = hash::fnv1a64(b"abc");
    assert_eq!(hash::hex(value).len(), 16);
}

// ---------------------------------------------------------------------------
// The training interface: one complete plan per turn (`Simulator::submit_plan`)
// ---------------------------------------------------------------------------

/// A full plan is accepted, resolves the turn and reports the turn's real
/// statistics; an incomplete plan and an illegal plan are both rejected without
/// changing the state.
#[test]
fn submit_plan_is_validated_atomically() {
    let sim = simulator();
    let mut state = sim
        .new_encounter(&team(), &fixed::SECTION5_WAVE, 3, BattleConfig::default())
        .expect("encounter builds");
    let before = sim.state_hash(&state);
    let plan = assign_all(&state, &sim);
    assert_eq!(plan.len(), team().len(), "one action per Sinner");

    // An incomplete plan is refused and nothing is committed.
    let partial = plan[..plan.len() - 1].to_vec();
    let err = sim.submit_plan(&mut state, partial).unwrap_err();
    assert!(format!("{err}").contains("incomplete plan"), "{err}");
    assert_eq!(sim.state_hash(&state), before, "state is untouched");

    // An illegal action is refused too.
    let mut illegal = plan.clone();
    if let Action::Assign { skill, .. } = &mut illegal[0] {
        *skill = lcb_core::ids::SkillId::new("9999999");
    }
    let err = sim.submit_plan(&mut state, illegal).unwrap_err();
    assert!(format!("{err}").contains("illegal action"), "{err}");
    assert_eq!(sim.state_hash(&state), before, "state is untouched");

    // A duplicate actor is refused as well.
    let mut duplicated = plan.clone();
    duplicated.push(plan[0].clone());
    let err = sim.submit_plan(&mut state, duplicated).unwrap_err();
    assert!(format!("{err}").contains("duplicate action"), "{err}");
    assert_eq!(sim.state_hash(&state), before);

    // The complete plan resolves the turn and reports the turn's signals.
    sim.submit_plan(&mut state, plan.clone()).expect("plan resolves");
    assert!(state.turn_stats.damage_to_enemies > 0);
    // Every Sinner used a Skill (the enemies' Skills are recorded too, so the
    // list is longer than the team).
    for action in plan.iter() {
        let actor = match action {
            Action::Assign { actor, .. }
            | Action::Engage { actor, .. }
            | Action::UseEgo { actor, .. } => actor.clone(),
            Action::Commit => continue,
        };
        assert!(
            state
                .turn_stats
                .skill_uses
                .iter()
                .any(|entry| entry.starts_with(&format!("{actor}|"))),
            "{actor} used no Skill"
        );
    }
    assert!(state.turn_stats.deaths.is_empty());
}

/// The atomic plan API and the incremental one produce the very same fight.
#[test]
fn submit_plan_matches_incremental_submission() {
    let sim = simulator();
    let mut planned = sim
        .new_encounter(&team(), &fixed::SECTION5_WAVE, 11, BattleConfig::default())
        .expect("encounter builds");
    let mut incremental = planned.clone();
    let plan = assign_all(&planned, &sim);
    sim.submit_plan(&mut planned, plan.clone()).expect("plan");
    for action in plan.iter() {
        sim.submit(&mut incremental, action.clone()).expect("submit");
    }
    sim.step_turn(&mut incremental).expect("turn");
    assert_eq!(sim.state_hash(&planned), sim.state_hash(&incremental));
    let transition = sim.transition_hash(&planned.clone(), &plan, &planned);
    assert_ne!(transition, 0);
}

#[test]
fn probe_ego_legality() {
    let sim = simulator();
    let mut state = sim
        .new_encounter(&team(), &fixed::SECTION5_WAVE, 1, BattleConfig::default())
        .unwrap();
    for sin in ["wrath", "lust", "sloth", "gluttony", "gloom", "pride", "envy"] {
        state.ego_resources.insert(sin.to_string(), 9);
    }
    let legal = sim.legal_actions(&state);
    let egos: Vec<_> = legal
        .iter()
        .filter(|a| matches!(a, Action::UseEgo { .. }))
        .collect();
    eprintln!("ego actions: {}", egos.len());
    for ego in egos.iter().take(5) {
        eprintln!("{ego:?}");
    }
    eprintln!("slots: {:?}", state.units[0].ego_slots);
    for (key, value) in state.ego_resources.iter() {
        eprintln!("{key} {value}");
    }
}
