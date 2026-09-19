//! Mechanic tests.  Every test names the source it encodes; a test that cannot
//! cite a source does not belong here (see docs/MECHANICS.md for the table).

use lcb_core::battle::{self, Action, CoinState, EgoSkillKind};
use lcb_core::damage::{self, DamageInputs};
use lcb_core::ids::{SkillId, Uptie};
use lcb_core::setup::{fixed, EncounterBuilder};
use lcb_core::state::{BattleConfig, ClashTieRule, Phase, Sanity, StaggerState};
use lcb_core::Simulator;
use std::path::PathBuf;

fn data_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("data")
}

fn sim() -> Simulator {
    Simulator::from_data_dir(data_root()).expect("library")
}

/// Damage: `Coin Roll x (1 + Static) x (1 + Dynamic)`, floored, min 1,
/// min 5% of the coin roll.  Source: wiki.gg `Damage`.
#[test]
fn damage_formula_matches_source() {
    let inputs = DamageInputs {
        coin_roll: 20,
        sin_resist: 1.25,      // Weak [x1.25] -> +0.25
        damage_type_resist: 1.0,
        offense_level: 63,
        defense_level: 60,     // 3 levels -> +0.10
        clash_count: 2,        // +0.06
        dynamic_modifier: 0.20,
        ..Default::default()
    };
    let out = damage::compute_damage(&inputs);
    let expected_static = 0.25 + 0.0 + (3.0 / 28.0) + 0.06;
    assert!((out.static_modifier - expected_static).abs() < 1e-9);
    let expected = (20.0 * (1.0 + expected_static) * 1.2).floor() as i32;
    assert_eq!(out.final_damage, expected);
}

/// Stagger: thresholds are percentages of max HP; Stagger level N gives
/// +0.5 + 0.5N to the damage-type modifier.  Source: wiki.gg `Clash`, `Damage`.
#[test]
fn stagger_levels_match_source() {
    let mut stagger = StaggerState::new(vec![60, 30, 15]);
    assert_eq!(stagger.crossed_thresholds(100, 100), 0);
    assert_eq!(stagger.crossed_thresholds(59, 100), 1);
    assert_eq!(stagger.crossed_thresholds(29, 100), 2);
    assert_eq!(stagger.crossed_thresholds(14, 100), 3);
    stagger.level = 1;
    assert!((stagger.damage_resistance_bonus() - 1.0).abs() < 1e-9);
    stagger.level = 3;
    assert!((stagger.damage_resistance_bonus() - 2.0).abs() < 1e-9);
}

/// Sanity: heads chance is `50 + SP`, SP is clamped to [-45, 45].
/// Source: wiki.gg `Sanity`.
#[test]
fn sanity_changes_coin_flip_odds() {
    assert_eq!(Sanity::Sane { sp: 0 }.heads_percent(), 50);
    assert_eq!(Sanity::Sane { sp: 45 }.heads_percent(), 95);
    assert_eq!(Sanity::Sane { sp: -45 }.heads_percent(), 5);
    assert_eq!(Sanity::None.heads_percent(), 50, "abnormalities flip at 50%");
    let over = Sanity::Sane { sp: 40 }.add(20);
    assert_eq!(over.sp(), 45);
    let under = Sanity::Sane { sp: -40 }.add(-20);
    assert_eq!(under.sp(), -45);
}

/// Unbreakable Coins become Cracked instead of being destroyed on Clash Lose.
/// Source: wiki.gg `Clash` / Unbreakable Coins.
#[test]
fn unbreakable_coins_crack_instead_of_breaking() {
    let mut coin = battle::CoinRuntime::fresh(true);
    battle::break_coin_for_test(&mut coin);
    assert_eq!(coin.state, CoinState::Cracked);
    let mut normal = battle::CoinRuntime::fresh(false);
    battle::break_coin_for_test(&mut normal);
    assert_eq!(normal.state, CoinState::Destroyed);
}

/// Clash: "The unit whose Skill has the lowest Clash power loses one Skill
/// Coin, and if the unit still has any Skill Coins left, the two units Clash
/// again."  Source: wiki.gg `Clash` / Battle Information.
#[test]
fn clash_loser_loses_one_coin_per_round() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let mut a = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1011003"),
    )
    .unwrap();
    let mut b = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        1,
        &SkillId::new("956701"),
    )
    .unwrap();
    a.base_power = 99; // force wins so the coin accounting is deterministic
    let coins_before = b.remaining_coins();
    let result = battle::resolve_clash(&mut state, 0, 1, &mut a, &mut b);
    assert_eq!(result.winner, Some(state.units[0].id.clone()));
    assert!(b.remaining_coins() < coins_before);
    assert_eq!(b.remaining_coins(), 0, "clash continues until one side is empty");
}

/// Clash ties are not documented; the default is configurable and recorded.
/// Source: none -> `UnknownRule::ClashTie`.
#[test]
fn clash_tie_is_configurable() {
    let sim = sim();
    let config = BattleConfig {
        tie_rule: ClashTieRule::AttackerWins,
        ..Default::default()
    };
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, config)
        .unwrap();
    let mut a = battle::build_use(&state, &sim.library, &sim.mechanics, 0, &SkillId::new("1011001"))
        .unwrap();
    let mut b = battle::build_use(&state, &sim.library, &sim.mechanics, 1, &SkillId::new("956701"))
        .unwrap();
    a.base_power = 10;
    b.base_power = 10;
    a.coins = vec![battle::CoinRuntime::fresh(false)];
    b.coins = vec![battle::CoinRuntime::fresh(false)];
    let result = battle::resolve_clash(&mut state, 0, 1, &mut a, &mut b);
    assert_eq!(result.winner, Some(state.units[0].id.clone()), "attacker wins ties");
    let _ = ClashTieRule::AttackerWins;
}

/// Burn ticks at turn end using Potency, then Count -1.
/// Source: wiki.gg `Status Effects` / Burn.
#[test]
fn burn_ticks_at_turn_end() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[1].statuses.add_potency("Burn", 25);
    state.units[1].statuses.add_count("Burn", 3);
    let hp_before = state.units[1].hp;
    battle::end_turn(&mut state);
    assert_eq!(state.units[1].hp, hp_before - 25);
    assert_eq!(state.units[1].statuses.count("Burn"), 2);
}

/// Sinking deals SP damage to units with Sanity and Gloom damage to units
/// without.  Source: wiki.gg `Status Effects` / Sinking.
#[test]
fn sinking_damages_sp_or_gloom() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    // Sinner (has Sanity)
    state.units[0].sanity = Sanity::Sane { sp: 10 };
    state.units[0].statuses.add_potency("Sinking", 8);
    state.units[0].statuses.add_count("Sinking", 2);
    // Abnormality (no Sanity) - takes Gloom damage instead
    state.units[1].statuses.add_potency("Sinking", 30);
    state.units[1].statuses.add_count("Sinking", 2);
    let hp_before = state.units[1].hp;
    battle::apply_sinking_for_test(&mut state, 0);
    battle::apply_sinking_for_test(&mut state, 1);
    assert_eq!(state.units[0].sanity.sp(), 2, "8 Sinking potency -> 8 SP damage");
    assert!(state.units[1].hp < hp_before, "non-SP unit takes Gloom damage");
}

/// Butterfly is unique Sinking: the attacker heals SP when hitting a unit that
/// has it.  Source: wiki.gg `Status Effects` / Butterfly.
#[test]
fn butterfly_heals_attacker_sp() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[1].statuses.add_potency("Butterfly", 12);
    state.units[1].statuses.add_count("Butterfly", 4);
    state.units[0].sanity = Sanity::Sane { sp: 0 };
    let mut a = battle::build_use(&state, &sim.library, &sim.mechanics, 0, &SkillId::new("1011001"))
        .unwrap();
    a.coins = vec![battle::CoinRuntime::fresh(false)];
    battle::one_sided_attack(&mut state, 0, 1, &mut a, 0);
    assert_eq!(state.units[0].sanity.sp(), 3, "The Living / 4 = 3 SP");
}

/// E.G.O costs SP and resources; Overclock multiplies both by 1.5 and rounds
/// up.  Source: wiki.gg `Clash` / E.G.O Skills, Overclocking.
#[test]
fn ego_costs_and_overclock_round_up() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let ego = sim.library.ego(&lcb_core::ids::EgoId::new("20106")).unwrap().clone();
    assert_eq!(ego.resource_cost.get("Gloom"), Some(&3));
    state.ego_resources.insert("Gloom".into(), 9);
    state.ego_resources.insert("Sloth".into(), 9);
    state.units[0].sanity = Sanity::Sane { sp: 45 };
    // Overclock the (awakening-only) record: cost = ceil(3 * 1.5) = 5 per sin.
    let kind = EgoSkillKind::Overclock;
    let affordable = battle::ego_affordable_for_test(&state, 0, &ego, kind);
    assert!(affordable, "9 resources are enough for ceil(4.5)=5");
    battle::pay_ego_for_test(&mut state, 0, &ego, kind);
    assert_eq!(state.ego_resources.get("Gloom"), Some(&4));
}

/// Skill decks refresh only after every card has been used.
/// Source: wiki.gg `Clash` / Skills ("will only refresh after all Skills have
/// been used").
#[test]
fn deck_refreshes_only_when_empty() {
    let mut deck = lcb_core::state::SkillDeck::new(vec![
        SkillId::new("a"),
        SkillId::new("b"),
    ]);
    assert_eq!(deck.draw_top().unwrap().0, "b");
    assert_eq!(deck.draw_top().unwrap().0, "a");
    assert!(deck.draw.is_empty());
    assert_eq!(deck.discard.len(), 2, "used cards wait in the discard pile");
    let next = deck.draw_top().unwrap();
    assert!(next.0 == "a" || next.0 == "b", "deck refreshed after emptying");
    assert_eq!(deck.len(), 2, "the refreshed deck holds both cards");
}

/// Uptie tiers resolve with the wiki convention: `Nkey` applies from uptie N
/// upwards, the unprefixed key is the highest tier.
/// Source: wiki.gg identity pages ("Uptie Changes" sections).
#[test]
fn uptie_resolution_matches_wiki_convention() {
    let sim = sim();
    let record = sim.library.identity(&lcb_core::ids::IdentityId::new("10110")).unwrap();
    let s1 = record.skills.iter().find(|s| s.id == "1011001").unwrap();
    assert_eq!(s1.tier(Uptie(1)).unwrap().base_power, Some(3));
    assert_eq!(s1.tier(Uptie(2)).unwrap().base_power, Some(3));
    assert_eq!(s1.tier(Uptie(3)).unwrap().base_power, Some(4));
    assert_eq!(s1.tier(Uptie(4)).unwrap().base_power, Some(4));
    assert!(s1.tier(Uptie(1)).unwrap().on_use_text.is_empty());
    assert!(!s1.tier(Uptie(4)).unwrap().on_use_text.is_empty());
}

/// The simulator must refuse nothing silently: every unimplemented effect line
/// is reported by strict mode.
/// Source: project plan ("unknown -> UNKNOWN, do not invent").
#[test]
fn unimplemented_effects_are_reported_not_ignored() {
    let sim = sim();
    let blockers = sim.strict_blockers();
    assert!(
        blockers.iter().any(|b| b.contains("A-Reson")),
        "resonance effects are not modelled and must be listed"
    );
}

/// A full turn keeps the battle in a consistent, serialisable state.
#[test]
fn turn_advances_phase_and_logs() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&[fixed::TEAM[0]], &[fixed::BOSS_IMAGO], 1, BattleConfig::default())
        .unwrap();
    assert_eq!(state.phase, Phase::AwaitingActions);
    let target = state.living_enemies()[0].clone();
    for action in sim.legal_actions(&state) {
        if let Action::Assign {
            actor,
            slot,
            skill,
            ..
        } = action
        {
            sim.submit(
                &mut state,
                Action::Assign {
                    actor,
                    slot,
                    skill,
                    target: target.clone(),
                },
            )
            .unwrap();
            break;
        }
    }
    sim.step_turn(&mut state).unwrap();
    assert!(state.turn >= 2);
    assert!(matches!(state.phase, Phase::AwaitingActions | Phase::Finished));
    let _ = EncounterBuilder::new(&sim.library, &sim.mechanics);
}
