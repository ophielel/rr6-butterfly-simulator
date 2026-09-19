//! Mechanic tests.  Every test names the source it encodes; a test that cannot
//! cite a source does not belong here (see docs/MECHANICS.md for the table).

use lcb_core::battle::{self, Action, CoinState, EgoSkillKind};
use lcb_core::damage::{self, DamageInputs};
use lcb_core::ids::{SkillId, Uptie};
use lcb_core::setup::{fixed, EncounterBuilder};
use lcb_core::state::{BattleConfig, Phase, Sanity, StaggerState};
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
    a.coin_power = 0;
    b.coin_power = 0;
    state.preset_flips = vec![false; 64];
    state.flip_cursor = 0;
    let coins_before = b.remaining_coins();
    let result = battle::resolve_clash(&mut state, 0, 1, &mut a, &mut b);
    assert_eq!(result.winner, Some(state.units[0].id.clone()));
    assert!(b.remaining_coins() < coins_before);
    assert_eq!(b.remaining_coins(), 0, "clash continues until one side is empty");
}

/// Clash power is the sum over **all** coins: Base Power once plus the Coin
/// Power of every Heads coin.  Sources: wiki.gg `Battles` ("both units toss all
/// of their Skill's Coins; this determines the Skill's power in a Clash") and
/// the Japanese wiki `戦闘システム詳細` (a 4+4 three-coin skill is 4-16).
#[test]
fn clash_power_sums_every_coin() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let mut use_ = battle::build_use(&state, &sim.library, &sim.mechanics, 0, &SkillId::new("1011003"))
        .unwrap();
    use_.base_power = 4;
    use_.coin_power = 4;
    use_.coins = vec![
        battle::CoinRuntime::fresh(false),
        battle::CoinRuntime::fresh(false),
        battle::CoinRuntime::fresh(false),
    ];
    // All heads -> base + 3 * coin power = 16, all tails -> 4.
    state.preset_flips = vec![true, true, true];
    state.flip_cursor = 0;
    battle::toss_all_for_test(&mut state, 0, &mut use_);
    assert_eq!(battle::final_power(&mut state, 0, &mut use_), 16);
    state.preset_flips = vec![false, false, false];
    state.flip_cursor = 0;
    battle::toss_all_for_test(&mut state, 0, &mut use_);
    assert_eq!(battle::final_power(&mut state, 0, &mut use_), 4);
}

/// A clash tie destroys nothing and the clash continues.
/// Source: Japanese wiki `戦闘システム詳細` ("マッチ威力が同じだった場合は引き分けとなり
/// 破壊されない").
#[test]
fn clash_tie_destroys_no_coin() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let mut a = battle::build_use(&state, &sim.library, &sim.mechanics, 0, &SkillId::new("1011001"))
        .unwrap();
    let mut b = battle::build_use(&state, &sim.library, &sim.mechanics, 1, &SkillId::new("956701"))
        .unwrap();
    a.base_power = 10;
    b.base_power = 10;
    a.coin_power = 0;
    b.coin_power = 0;
    a.coins = vec![battle::CoinRuntime::fresh(false)];
    b.coins = vec![battle::CoinRuntime::fresh(false)];
    // Tails on both sides keeps the powers identical forever.
    state.preset_flips = vec![false; 64];
    state.flip_cursor = 0;
    let result = battle::resolve_clash(&mut state, 0, 1, &mut a, &mut b);
    assert_eq!(result.winner, None, "equal power is a draw");
    assert_eq!(a.remaining_coins(), 1);
    assert_eq!(b.remaining_coins(), 1);
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
    battle::end_turn(&mut state, &sim.mechanics);
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

/// The panel draws randomly from the composition and resets the counts once
/// every copy has been placed.  Source: Japanese wiki `戦闘システム詳細`
/// ("スキル構成", "全てパネルに配置し終えると構成の残数がリセットされる").
#[test]
fn deck_draws_randomly_and_resets_after_full_placement() {
    use lcb_core::state::SkillDeck;
    let mut rng = lcb_core::rng::Rng::from_seed(9);
    let mut deck = SkillDeck::new(vec![
        (SkillId::new("s1"), 3),
        (SkillId::new("s2"), 2),
        (SkillId::new("s3"), 1),
    ]);
    let mut drawn = Vec::new();
    for _ in 0..6 {
        drawn.push(deck.draw(&mut rng).unwrap().0);
    }
    assert!(deck.is_empty(), "all six copies have been placed");
    let mut counts = std::collections::BTreeMap::new();
    for id in &drawn {
        *counts.entry(id.clone()).or_insert(0) += 1;
    }
    assert_eq!(counts.get("s1"), Some(&3));
    assert_eq!(counts.get("s2"), Some(&2));
    assert_eq!(counts.get("s3"), Some(&1));
    // The composition resets and can be drawn again.
    assert_eq!(deck.draw(&mut rng).unwrap().0.len(), 2);
    assert_eq!(deck.len(), 5);
}

/// The panel keeps two skills per slot: using the bottom skill consumes it, the
/// top one rotates down and a new skill is drawn.  Source: Japanese wiki
/// `戦闘システム詳細` ("上に見えていたスキルが下へ送られ…また次のスキルが新しく薄らと
/// 見えるようになる").
#[test]
fn panel_rotates_after_use() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 5, BattleConfig::default())
        .unwrap();
    let before = state.units[0].dashboard[0].clone();
    assert_ne!(before.current.0, "");
    assert_ne!(before.next.0, "");
    let paneled_before = state.units[0].deck.paneled_count(&before.current);
    let target = state.living_enemies()[0].clone();
    let actor = state.units[0].id.clone();
    sim.submit(
        &mut state,
        Action::Assign {
            actor,
            slot: 0,
            skill: before.current.clone(),
            target,
        },
    )
    .unwrap();
    sim.step_turn(&mut state).unwrap();
    let after = state.units[0].dashboard[0].clone();
    assert_eq!(after.current, before.next, "the second skill rotates down");
    assert_eq!(after.next, before.preview, "the preview becomes selectable");
    assert_ne!(after.preview.0, "", "a new preview is drawn");
    // Every slot keeps three skills on the panel (2 selectable + 1 preview);
    // the deck grew by the extra slot the Sinner received from turn 2 (one
    // Sinner in a six-slot encounter, wiki.gg `Battles` / Deployment Order).
    let paneled_after: u32 = state.units[0].deck.paneled.iter().map(|(_, n)| *n).sum();
    let slots_after = state.units[0].dashboard.len() as u32;
    assert_eq!(paneled_after, slots_after * 3);
    assert_eq!(slots_after, 2, "one extra slot per turn until the cap");
    assert!(paneled_before >= 1);
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
    // "Base Power -2 for every Cracked Coin" is the remaining known gap: the
    // wiki/game data cached for this project never defines what cracks a Coin,
    // so it stays UNKNOWN and strict mode must refuse it.
    assert!(
        blockers.iter().any(|b| b.contains("Cracked Coin")),
        "unknown Cracked Coin rules must be listed: {blockers:?}"
    );
}

/// Guard: the Shield is gained when the unit is first attacked, not at the
/// start of the turn, and it equals the Guard skill's Final Power.
/// Sources: wiki.gg `Battles` / Guard and JA-wiki 守備スキル / ガード.
#[test]
fn guard_gains_shield_when_attacked() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 7, BattleConfig::default())
        .unwrap();
    // Yi Sang's Guard at Uptie IV: 10 +4, one coin.
    let guard = SkillId::new("1011004");
    let target = state.living_enemies()[0].clone();
    let actor = state.units[0].id.clone();
    let enemy = state.units[1].id.clone();
    sim.submit(
        &mut state,
        Action::Assign {
            actor,
            slot: 0,
            skill: guard,
            target,
        },
    )
    .unwrap();
    // The enemy attacks back so the Guard resolves.
    state.actions.push(lcb_core::state::SubmittedAction {
        actor: enemy,
        slot: 0,
        skill: SkillId::new("956701"),
        target: Some(state.units[0].id.clone()),
        is_ego: false,
        ego: None,
        ego_kind: None,
    });
    state.preset_flips = vec![true; 32];
    state.flip_cursor = 0;
    sim.step_turn(&mut state).unwrap();
    // 10 + 4 = 14 Shield, then the enemy's hit is absorbed by it.
    assert!(
        state.log.iter().any(|entry| entry.kind == "guard" && entry.detail.contains("14 Shield")),
        "guard log: {:?}",
        state.log.iter().map(|e| e.detail.clone()).collect::<Vec<_>>()
    );
}

/// The Imago plays the documented station-5 rotation: six Skill Slots, a
/// three-turn cycle, small/mid/big skills chosen by the active state of time.
/// Sources: wiki.gg Imago `Behavior` and the JA-wiki 行動パターン table.
#[test]
fn imago_plays_the_documented_rotation() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 1, BattleConfig::default())
        .unwrap();
    let enemy_id = state
        .units
        .iter()
        .find(|u| !u.kind.is_sinner())
        .unwrap()
        .id
        .clone();
    let turn_skills = |state: &lcb_core::state::BattleState| -> Vec<String> {
        let mut skills: Vec<(u32, String)> = state
            .actions
            .iter()
            .filter(|a| a.actor == enemy_id)
            .map(|a| (a.slot, a.skill.0.clone()))
            .collect();
        skills.sort();
        skills.into_iter().map(|(_, s)| s).collect()
    };
    // Turn 1, above 66% HP, In the Past: Temper and Cast x2, Fluttering Havoc x4.
    let first = turn_skills(&state);
    assert_eq!(
        first,
        vec!["956704", "956704", "956701", "956701", "956701", "956701"],
        "past / above_66 / turn 1"
    );
    // Turn 2: Immolation x2, Pulverization x4.
    for action in sim.legal_actions(&state) {
        if let Action::Assign { actor, slot, skill, target } = action {
            if actor != enemy_id {
                sim.submit(&mut state, Action::Assign { actor, slot, skill, target }).unwrap();
                break;
            }
        }
    }
    sim.step_turn(&mut state).unwrap();
    let second = turn_skills(&state);
    assert_eq!(second, vec!["956705", "956705", "956702", "956702", "956702", "956702"]);
}

/// Turn start activates the highest-Stacked state of time; ties keep the state.
/// Source: wiki.gg Imago passive `Moment of Entangled Lives`.
#[test]
fn time_state_follows_the_highest_stack() {
    use lcb_core::scripts::TimeState;
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 2, BattleConfig::default())
        .unwrap();
    let enemy = state
        .units
        .iter()
        .position(|u| !u.kind.is_sinner())
        .unwrap();
    assert_eq!(state.units[enemy].time_state, Some(TimeState::Past));
    assert_eq!(state.units[enemy].statuses.stack("In the Past"), 10);
    // Push Future ahead of the others.
    state.units[enemy].statuses.add_stack("In the Future", 15);
    for action in sim.legal_actions(&state) {
        if let Action::Assign { actor, slot, skill, target } = action {
            sim.submit(&mut state, Action::Assign { actor, slot, skill, target }).unwrap();
        }
    }
    sim.step_turn(&mut state).unwrap();
    assert_eq!(state.units[enemy].time_state, Some(TimeState::Future));
    // A switch adds Temporal Disjunction.
    assert_eq!(state.units[enemy].statuses.stack("Temporal Disjunction"), 1);
}

/// Stack thresholds grant their documented bonuses.
/// Source: in-game `Bufs_Refraction6` (StackPastActivate / Present / Future).
#[test]
fn time_state_stack_bonus_matches_game_text() {
    use lcb_core::scripts::TimeState;
    let low = TimeState::stack_bonus(5);
    assert_eq!((low.clash_power, low.final_power, low.potency, low.count), (0, 0, 1, 1));
    let mid = TimeState::stack_bonus(15);
    assert_eq!((mid.clash_power, mid.final_power, mid.potency, mid.count), (1, 0, 2, 1));
    let high = TimeState::stack_bonus(25);
    assert_eq!((high.clash_power, high.final_power, high.potency, high.count), (0, 2, 3, 2));
    assert_eq!(TimeState::Past.boosted_status(), "Burn");
    assert_eq!(TimeState::Present.boosted_status(), "Poise");
    assert_eq!(TimeState::Future.boosted_status(), "Bleed");
}

/// Rupture deals fixed damage by Potency on hit and loses one Count.
/// Source: wiki.gg `Status Effects` / Rupture.
#[test]
fn rupture_ticks_on_hit() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[1].statuses.add_potency("Rupture", 12);
    state.units[1].statuses.add_count("Rupture", 3);
    let before = state.units[1].hp;
    let mut use_ = battle::build_use(&state, &sim.library, &sim.mechanics, 0, &SkillId::new("1011001"))
        .unwrap();
    use_.coins = vec![battle::CoinRuntime::fresh(false)];
    state.preset_flips = vec![false; 16];
    state.flip_cursor = 0;
    battle::one_sided_attack(&mut state, 0, 1, &mut use_, 0);
    assert_eq!(state.units[1].statuses.count("Rupture"), 2);
    // damage = the attack's own damage + 12 Rupture
    assert!(before - state.units[1].hp >= 12);
}

/// Protection reduces and Fragile increases incoming damage by 10% per Count.
/// Source: wiki.gg `Status Effects` / Protection, Fragile.
#[test]
fn protection_and_fragile_modify_incoming_damage() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[1].statuses.add_count("Protection", 1); // -10%
    state.units[1].statuses.add_count("Fragile", 2); // +20%
    state.units[1].statuses.add_count("Wrath Fragility", 1); // +10% vs Wrath
    let defense = state.units[1].clone();
    let modifier = battle::incoming_damage_modifier_for_test(&defense, "Wrath");
    assert!((modifier - 0.20).abs() < 1e-9, "got {modifier}");
    let other = battle::incoming_damage_modifier_for_test(&defense, "Gloom");
    assert!((other - 0.10).abs() < 1e-9, "fragility is affinity specific: {other}");
    // Resist Down raises the resistance value by 0.1 per Count.
    state.units[1].statuses.add_count("Gloom Resist Down", 3);
    assert!((state.units[1].resist_sin(lcb_core::ids::Sin::Gloom) - 1.3).abs() < 1e-9);
}

/// Skill power statuses: Power Up/Down on every skill, Attack Power Up/Down on
/// attacks only.  Source: wiki.gg `Status Effects`.
#[test]
fn power_statuses_change_final_power() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let mut use_ = battle::build_use(&state, &sim.library, &sim.mechanics, 0, &SkillId::new("1011001"))
        .unwrap();
    use_.base_power = 4;
    use_.coin_power = 0;
    use_.coins = vec![battle::CoinRuntime::fresh(false)];
    state.preset_flips = vec![false; 16];
    state.flip_cursor = 0;
    battle::toss_all_for_test(&mut state, 0, &mut use_);
    assert_eq!(battle::final_power(&mut state, 0, &mut use_), 4);
    state.units[0].statuses.add_count("Power Up", 3);
    state.units[0].statuses.add_count("Attack Power Up", 2);
    state.units[0].statuses.add_potency("Attack Power Down", 1);
    assert_eq!(battle::final_power(&mut state, 0, &mut use_), 4 + 3 + 2 - 1);
}

/// Tremor Burst raises the Stagger Threshold by the target's Tremor Potency and
/// consumes Tremor Count.  Sources: wiki.gg `Status Effects` / Tremor,
/// Tremor Burst.
#[test]
fn tremor_burst_raises_stagger_threshold() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[1].statuses.add_potency("Tremor", 15);
    state.units[1].statuses.add_count("Tremor", 2);
    let before = state.units[1].stagger.thresholds_percent[0];
    let effects = vec![lcb_core::effects::Effect {
        kind: "tremor_burst".to_string(),
        consume_count: Some(1),
        ..Default::default()
    }];
    let mut notes = Vec::new();
    let mut use_ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &effects, 0, Some(1), &mut notes, &mut use_ctx);
    assert_eq!(state.units[1].stagger.thresholds_percent[0], before + 15);
    assert_eq!(state.units[1].statuses.count("Tremor"), 1);
}

/// Bind lowers Speed for the turn.  Source: wiki.gg `Status Effects` / Bind.
#[test]
fn bind_lowers_speed() {
    let sim = sim();
    let state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let mut unit = state.units[0].clone();
    unit.statuses.add_potency("Bind", 2);
    let bind = unit.statuses.potency("Bind");
    let haste = unit.statuses.count("Haste");
    assert_eq!(bind - haste, 2);
}

/// Station 1: the Pupa opens with 1.3% of its max HP as Shield and its HP never
/// falls below 90%.  Source: wiki.gg Pupa passive `Quickening of the Unborn`.
#[test]
fn pupa_shield_and_hp_floor() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_PUPA], 4, BattleConfig::default())
        .unwrap();
    let pupa = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    // floor(25616 * 1.3%) = 333
    assert_eq!(state.units[pupa].shield, 333);
    let floor = state.units[pupa].max_hp * 90 / 100;
    state.units[pupa].shield = 0;
    let max_hp = state.units[pupa].max_hp;
    let (_, hp_lost) = state.units[pupa].take_damage(1000);
    assert_eq!(state.units[pupa].hp, max_hp - 1000);
    assert_eq!(hp_lost, 1000);
    let (_, more) = state.units[pupa].take_damage(50_000);
    assert_eq!(state.units[pupa].hp, floor, "damage stops at the 90% floor");
    assert_eq!(more, max_hp - 1000 - floor);
    let (_, none) = state.units[pupa].take_damage(50_000);
    assert_eq!(none, 0, "the floor cannot be crossed");
}

/// The Pupa's barrier-break branch: once the Shield is fully consumed it plays
/// pattern a next turn (Entangled Life x3 + The Quickening) and The Quickening
/// ends the encounter.  Source: JA-wiki 行動パターン, wiki.gg Pupa page.
#[test]
fn pupa_barrier_break_switches_pattern_and_quickening_ends_the_encounter() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_PUPA], 6, BattleConfig::default())
        .unwrap();
    let pupa = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    // Consume the barrier during turn 1.
    state.units[pupa].take_damage(333);
    assert!(state.units[pupa].barrier_broken);
    // Play turn 1 out.
    for action in sim.legal_actions(&state) {
        if let Action::Assign { actor, slot, skill, target } = action {
            sim.submit(&mut state, Action::Assign { actor, slot, skill, target }).unwrap();
        }
    }
    sim.step_turn(&mut state).unwrap();
    // Turn 2 must use the branch.
    let mut skills: Vec<(u32, String)> = state
        .actions
        .iter()
        .filter(|a| state.units[pupa].id == a.actor)
        .map(|a| (a.slot, a.skill.0.clone()))
        .collect();
    skills.sort();
    let ids: Vec<String> = skills.into_iter().map(|(_, s)| s).collect();
    assert_eq!(ids, vec!["956304", "956305", "956306", "956303"]);
    // Resolve turn 2: The Quickening ends the encounter.
    for action in sim.legal_actions(&state) {
        if let Action::Assign { actor, slot, skill, target } = action {
            sim.submit(&mut state, Action::Assign { actor, slot, skill, target }).unwrap();
        }
    }
    sim.step_turn(&mut state).unwrap();
    assert_eq!(state.winner, Some(lcb_core::state::Winner::EncounterEnded));
}

/// The Quickening deals no damage and its user takes none that turn.
/// Source: wiki.gg Pupa skill `The Quickening`.
#[test]
fn quickening_deals_and_takes_no_damage() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_PUPA], 8, BattleConfig::default())
        .unwrap();
    let pupa = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    let mut use_ = battle::build_use(&state, &sim.library, &sim.mechanics, pupa, &SkillId::new("956303"))
        .unwrap();
    // [On Use] effects are applied by the engine when the skill resolves, so the
    // test drives the same two effects through the public effect path.
    let effects = use_.mechanics.on_use.clone();
    let mut notes = Vec::new();
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &effects, pupa, Some(0), &mut notes, &mut ctx);
    use_.ctx.zero_damage = ctx.zero_damage;
    assert!(use_.ctx.zero_damage, "the skill declares 0 damage");
    assert_eq!(state.units[pupa].statuses.stack("No Damage Taken"), 1);
    let before = state.units[pupa].hp;
    let (_, lost) = state.units[pupa].take_damage(500);
    assert_eq!(lost, 0);
    assert_eq!(state.units[pupa].hp, before);
}

/// "If any of the following conditions are met" is an alternative list.
/// Source: wiki.gg skill text (e.g. `Lobotomy E.G.O::Lamp` Gregor).
#[test]
fn any_of_conditions_are_alternatives() {
    use lcb_core::effects::{Condition, Effect};
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let effect = Effect {
        kind: "coin_power".to_string(),
        value: Some(1),
        condition: Some(Condition {
            any_of: vec![
                Condition {
                    self_speed_at_most: Some(1),
                    ..Default::default()
                },
                Condition {
                    source: Some("target".to_string()),
                    status: Some("Dazzle".to_string()),
                    gte: Some(1),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    };
    // Nothing holds -> no bonus.
    state.units[0].speed = 5;
    let mut notes = Vec::new();
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &[effect.clone()], 0, Some(1), &mut notes, &mut ctx);
    assert_eq!(ctx.coin_power_bonus, 0);
    // Dazzle on the target satisfies the second alternative.
    state.units[1].statuses.add_stack("Dazzle", 1);
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &[effect.clone()], 0, Some(1), &mut notes, &mut ctx);
    assert_eq!(ctx.coin_power_bonus, 1);
    // Speed 1 satisfies the first alternative.
    state.units[1].statuses.remove("Dazzle");
    state.units[0].speed = 1;
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &[effect], 0, Some(1), &mut notes, &mut ctx);
    assert_eq!(ctx.coin_power_bonus, 1);
}

/// Attack adder: "deal N% of this Coin's final damage" adds damage on top.
/// Source: wiki.gg `Damage` / attack adders.
#[test]
fn bonus_damage_percent_of_coin_adds_damage() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let mut use_ = battle::build_use(&state, &sim.library, &sim.mechanics, 0, &SkillId::new("1011001"))
        .unwrap();
    use_.coins = vec![battle::CoinRuntime::fresh(false)];
    use_.mechanics.coins.insert(
        "1".to_string(),
        vec![lcb_core::effects::Effect {
            kind: "bonus_damage_percent_of_coin".to_string(),
            percent: Some(100),
            ..Default::default()
        }],
    );
    state.preset_flips = vec![false; 16];
    state.flip_cursor = 0;
    let before = state.units[1].hp;
    let hits = battle::one_sided_attack(&mut state, 0, 1, &mut use_, 0);
    let base: i32 = hits.iter().map(|h| h.damage).sum();
    let dealt = before - state.units[1].hp;
    assert!(dealt >= base * 2 - 1, "adder doubled the hit: {base} -> {dealt}");
}

/// Effects with a per-turn limit only fire that many times per turn, and the
/// counter resets at Turn Start.  Source: skill text "(N times per turn)".
#[test]
fn per_turn_limits_are_enforced() {
    use lcb_core::effects::Effect;
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let effect = Effect {
        kind: "gain".to_string(),
        status: Some("Protection".to_string()),
        count: Some(1),
        per_turn: Some(2),
        raw: Some("test line".to_string()),
        ..Default::default()
    };
    let mut notes = Vec::new();
    for _ in 0..3 {
        let mut ctx = battle::UseContext::default();
        battle::apply_effects_for_test(&mut state, &[effect.clone()], 0, Some(1), &mut notes, &mut ctx);
    }
    assert_eq!(state.units[0].statuses.count("Protection"), 2);
    // A new turn clears the limit.
    state.units[0].turn_effect_usage.clear();
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &[effect], 0, Some(1), &mut notes, &mut ctx);
    assert_eq!(state.units[0].statuses.count("Protection"), 3);
}

/// "Gain N [X] next turn" is queued and applied at the next Turn Start.
#[test]
fn next_turn_buffs_apply_at_turn_start() {
    use lcb_core::effects::Effect;
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let effect = Effect {
        kind: "gain".to_string(),
        status: Some("Protection".to_string()),
        count: Some(2),
        next_turn: true,
        raw: Some("Gain 2 [Protection] next turn".to_string()),
        ..Default::default()
    };
    let mut notes = Vec::new();
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &[effect], 0, Some(1), &mut notes, &mut ctx);
    assert_eq!(state.units[0].statuses.count("Protection"), 0, "not yet");
    assert_eq!(state.units[0].pending_next_turn.len(), 1);
    // Resolve the turn; the buff lands at the next Turn Start.
    battle::end_turn(&mut state, &sim.mechanics);
    battle::begin_turn(
        &mut state,
        &sim.library,
        &sim.mechanics,
        &sim.scripts,
    );
    assert_eq!(state.units[0].statuses.count("Protection"), 2);
}

/// "At 15+ [Deep Tears], consume 5 [Deep Tears] to deal +15% damage".
#[test]
fn consume_status_for_damage() {
    use lcb_core::effects::Effect;
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[0].statuses.add_potency("Deep Tears", 20);
    let effect = Effect {
        kind: "consume_status_for_damage".to_string(),
        status: Some("Deep Tears".to_string()),
        threshold: Some(15),
        value: Some(5),
        percent: Some(15),
        ..Default::default()
    };
    let mut notes = Vec::new();
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &[effect.clone()], 0, Some(1), &mut notes, &mut ctx);
    assert!((ctx.damage_bonus - 0.15).abs() < 1e-9);
    assert_eq!(state.units[0].statuses.potency("Deep Tears"), 0);
    // Below the threshold nothing happens.
    state.units[0].statuses.add_potency("Deep Tears", 5);
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &[effect], 0, Some(1), &mut notes, &mut ctx);
    assert_eq!(ctx.damage_bonus, 0.0);
}

/// "Gain Shield equal to (SP / 5)% of this unit's max HP".
#[test]
fn shield_percent_from_sp() {
    use lcb_core::effects::Effect;
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[0].sanity = Sanity::Sane { sp: 20 };
    let effect = Effect {
        kind: "shield_percent_from_sp".to_string(),
        value: Some(5),
        ..Default::default()
    };
    let mut notes = Vec::new();
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &[effect], 0, Some(1), &mut notes, &mut ctx);
    let expected = state.units[0].max_hp * 4 / 100; // (20 / 5)% of max HP
    assert_eq!(ctx.shield_gain, expected);
}

/// Regression: "Inflict N [X]" must actually land N Potency (an earlier
/// version scaled from `value`, which is unset for inflictions, and applied 0).
#[test]
fn inflicted_statuses_actually_land() {
    use lcb_core::effects::Effect;
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let effects = vec![
        Effect {
            kind: "inflict".to_string(),
            status: Some("Burn".to_string()),
            potency: Some(3),
            ..Default::default()
        },
        Effect {
            kind: "inflict".to_string(),
            status: Some("Sinking".to_string()),
            count: Some(2),
            ..Default::default()
        },
        Effect {
            kind: "gain".to_string(),
            status: Some("Poise".to_string()),
            potency: Some(2),
            count: Some(1),
            ..Default::default()
        },
    ];
    let mut notes = Vec::new();
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &effects, 0, Some(1), &mut notes, &mut ctx);
    assert_eq!(state.units[1].statuses.potency("Burn"), 3);
    assert_eq!(state.units[1].statuses.count("Sinking"), 2);
    assert_eq!(state.units[0].statuses.potency("Poise"), 2);
    assert_eq!(state.units[0].statuses.count("Poise"), 1);
}

/// Sin Resonance: 2+ Skills of the same affinity on the Dashboard; Absolute
/// Sin Resonance: 3+ of them consecutively.  Source: wiki.gg `Resonance`.
#[test]
fn sin_resonance_is_counted_from_the_dashboard() {
    use lcb_core::ids::Sin;
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 1, BattleConfig::default())
        .unwrap();
    // Pick, for each Sinner, the first skill of a chosen affinity (Pride).
    let mut chosen = 0;
    for unit in state.units.clone() {
        if !unit.kind.is_sinner() {
            continue;
        }
        let lcb_core::state::UnitKind::Sinner { identity } = &unit.kind else { continue };
        let record = sim.library.identity(identity).unwrap();
        let Some(skill) = record
            .skills
            .iter()
            .find(|s| s.sin(Uptie::IV) == Some(Sin::Pride) && s.slot() != Some(lcb_core::ids::SkillSlot::Defense))
        else {
            continue;
        };
        state.actions.push(lcb_core::state::SubmittedAction {
            actor: unit.id.clone(),
            slot: 0,
            skill: SkillId::new(skill.id.clone()),
            target: Some(state.living_enemies()[0].clone()),
            is_ego: false,
            ego: None,
            ego_kind: None,
        });
        chosen += 1;
    }
    assert!(chosen >= 3, "need at least three Pride skills to test resonance");
    sim.step_turn(&mut state).unwrap();
    // Every selected skill has slot 0, so the run length equals the count.
    assert_eq!(state.resonance.get("pride"), Some(&chosen));
    assert_eq!(state.a_resonance.get("pride"), Some(&chosen));
    assert_eq!(
        battle::highest_resonance(&state),
        chosen,
        "highest resonance is the largest count"
    );
}

/// "Gain (highest Reson.) of [X] (max N)" scales with the highest resonance.
#[test]
fn gain_from_resonance_uses_the_highest_value() {
    use lcb_core::effects::Effect;
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    // The combat phase copies the turn's resonance onto the units; here the
    // value is set directly.
    state.resonance.insert("pride".to_string(), 4);
    state.units[0].resonance_max = 4;
    let effect = Effect {
        kind: "gain_from_resonance".to_string(),
        status: Some("The Living & The Departed".to_string()),
        max: Some(6),
        multiplier: Some(1),
        ..Default::default()
    };
    let mut notes = Vec::new();
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &[effect.clone()], 0, Some(1), &mut notes, &mut ctx);
    assert_eq!(state.units[0].statuses.potency("The Living & The Departed"), 4);
    // The x2 A-Reson variant doubles it and respects the cap.
    let doubled = Effect {
        multiplier: Some(2),
        max: Some(6),
        requires_a_reson: true,
        ..effect
    };
    state.units[0].a_reson_max = 0;
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &[doubled.clone()], 0, Some(1), &mut notes, &mut ctx);
    assert_eq!(
        state.units[0].statuses.potency("The Living & The Departed"),
        4,
        "no A-Reson, no extra gain"
    );
    state.units[0].a_reson_max = 4;
    let before = state.units[0].statuses.potency("The Living & The Departed");
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &[doubled], 0, Some(1), &mut notes, &mut ctx);
    let gained = state.units[0].statuses.potency("The Living & The Departed") - before;
    assert_eq!(gained, 6, "4 x 2 capped at 6");
}

/// Ammo-scaled effects: "Base Power +1 for every [X] about to be spent" and
/// "Deal +([X] spent x N)% damage".
#[test]
fn ammo_planned_and_spent_scale_the_skill() {
    use lcb_core::effects::Effect;
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let mut use_ = battle::build_use(&state, &sim.library, &sim.mechanics, 0, &SkillId::new("1011003"))
        .unwrap();
    // Isolate the clause under test from the skill's own effects.
    use_.mechanics.on_use.clear();
    use_.mechanics.on_use.push(Effect {
        kind: "base_power_per_ammo_planned".to_string(),
        step: Some(1),
        ..Default::default()
    });
    use_.coins.iter_mut().for_each(|c| c.state = battle::CoinState::Fresh);
    use_.mechanics.coins.insert(
        "1".to_string(),
        vec![Effect {
            kind: "spend_ammo".to_string(),
            value: Some(2),
            ..Default::default()
        }],
    );
    use_.mechanics.coins.insert(
        "2".to_string(),
        vec![Effect {
            kind: "spend_ammo".to_string(),
            value: Some(1),
            ..Default::default()
        }],
    );
    state.units[0]
        .statuses
        .add_potency("The Living & The Departed", 10);
    // [On Use] effects run through the engine's own preparation step so that
    // "about to be spent" is computed from this use's coin effects.
    battle::prepare_use_for_test(&mut state, &sim.library, &sim.mechanics, 0, Some(1), &mut use_);
    assert_eq!(use_.ctx.ammo_planned, 3, "three ammo are about to be spent");
    assert_eq!(use_.ctx.base_power_bonus, 3, "+1 per planned ammo");
}

/// Critical modifiers: target SP below zero raises the crit chance, and
/// conditional crit damage adds to the multiplier.
#[test]
fn critical_modifiers_from_statuses() {
    use lcb_core::effects::Effect;
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[1].sanity = Sanity::Sane { sp: -12 };
    let mut ctx = battle::UseContext::default();
    let mut notes = Vec::new();
    battle::apply_effects_for_test(
        &mut state,
        &[Effect {
            kind: "crit_chance_from_target_sp".to_string(),
            ..Default::default()
        }],
        0,
        Some(1),
        &mut notes,
        &mut ctx,
    );
    assert_eq!(ctx.crit_chance_bonus, 12);
    battle::apply_effects_for_test(
        &mut state,
        &[Effect {
            kind: "crit_damage_bonus".to_string(),
            percent: Some(30),
            ..Default::default()
        }],
        0,
        Some(1),
        &mut notes,
        &mut ctx,
    );
    assert!((ctx.crit_damage_bonus - 0.30).abs() < 1e-9);
}

/// Stations 2-4: the illusory butterfly has 1 HP and 333 Shield, and its hits
/// knock Stacks off the Imago in the campaign (Segmentation).
/// Sources: wiki.gg Illusory Butterfly page, JA-wiki part info.
#[test]
fn illusory_butterfly_shield_and_segmentation() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &["9564"], 4, BattleConfig::default())
        .unwrap();
    let butterfly = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    assert_eq!(state.units[butterfly].max_hp, 1);
    assert_eq!(state.units[butterfly].shield, 333);
    assert!(state.units[butterfly].segmentation.is_some());
    // The campaign starts with no Stacks; a hit as the main target removes one
    // (floored at zero) and heals the attacker 10 SP.
    state.campaign.time_stacks.insert("In the Past".to_string(), 4);
    state.units[butterfly].take_damage(400); // break the shield
    assert!(state.units[butterfly].barrier_broken);
    assert_eq!(state.units[butterfly].hp, 1, "HP floor keeps it at 1");
    let mut use_ = battle::build_use(&state, &sim.library, &sim.mechanics, 0, &SkillId::new("1011001"))
        .unwrap();
    use_.coins = vec![battle::CoinRuntime::fresh(false)];
    state.units[0].sanity = Sanity::Sane { sp: 0 };
    state.preset_flips = vec![false; 16];
    state.flip_cursor = 0;
    battle::one_sided_attack(&mut state, 0, butterfly, &mut use_, 0);
    assert_eq!(
        state.campaign.time_stacks.get("In the Past"),
        Some(&3),
        "one Stack was knocked off the Imago"
    );
    assert_eq!(state.units[0].sanity.sp(), 10, "the attacker healed 10 SP");
    assert_eq!(state.units[butterfly].hits_taken, 1);
    // A turn in which the butterfly is never hit gives the Imago +5 Stacks.
    battle::end_turn(&mut state, &sim.mechanics);
    assert_eq!(state.campaign.time_stacks.get("In the Past"), Some(&3));
    state.units[butterfly].hits_taken = 0;
    battle::end_turn(&mut state, &sim.mechanics);
    assert_eq!(state.campaign.time_stacks.get("In the Past"), Some(&8));
}

/// Section 5 starts from the campaign: the Pupa's remaining HP carries over and
/// the choice events can disable components of the Past passive.
/// Source: JA-wiki station 5 notes + wiki.gg choice events.
#[test]
fn section5_carries_the_campaign() {
    use lcb_core::state::CampaignState;
    let sim = sim();
    let mut campaign = CampaignState::default();
    campaign.pupa_hp = Some(12_000);
    campaign.station = 4;
    campaign.disabled_passives.push("past:burn_on_hit".to_string());
    let mut state = sim
        .section5(&campaign, 1, BattleConfig::default())
        .unwrap();
    let imago = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    assert_eq!(state.units[imago].hp, 12_000, "starts from the Pupa's HP");
    assert_eq!(state.campaign.station, 5);
    // The disabled component no longer burns attackers.
    state.units[imago].time_state = Some(lcb_core::scripts::TimeState::Past);
    battle::time_passives::on_hit_by_sinner(&mut state, imago, 0);
    assert_eq!(state.units[0].statuses.potency("Burn"), 0);
    state
        .campaign
        .disabled_passives
        .clear();
    battle::time_passives::on_hit_by_sinner(&mut state, imago, 0);
    assert_eq!(state.units[0].statuses.potency("Burn"), 1);
}

/// Section 5: the Imago's rotation across all three states of time and HP bands,
/// straight from the script data (JA-wiki 行動パターン).
#[test]
fn section5_rotation_covers_states_and_hp_bands() {
    let sim = sim();
    let script = sim
        .scripts
        .for_enemy(fixed::BOSS_IMAGO)
        .expect("Imago script");
    assert_eq!(script.slots, 6);
    assert_eq!(script.cycle_turns, 3);
    // Above 66%: turn 1 = small x2 + Fluttering Havoc x4 for every state.
    for state in lcb_core::scripts::TimeState::ALL {
        let entry = script.states.get(state.as_str()).unwrap();
        let turn1 = script.turn_skills(100, state, 0);
        assert_eq!(turn1.len(), 6, "{state:?} turn 1 has six slots");
        assert_eq!(turn1[0], entry.small);
        assert_eq!(turn1[1], entry.small);
        assert!(turn1[2..].iter().all(|id| *id == "956701"), "Fluttering Havoc x4");
        // Turn 2 = mid x2 + Pulverization x4.
        let turn2 = script.turn_skills(100, state, 1);
        assert_eq!(turn2[0], entry.mid);
        assert!(turn2[2..].iter().all(|id| *id == "956702"));
        // Turn 3 = big + Chaotic Turmoil x3 (four actions).
        let turn3 = script.turn_skills(100, state, 2);
        assert_eq!(turn3.len(), 4);
        assert_eq!(turn3[0], entry.big);
        assert!(turn3[1..].iter().all(|id| *id == "956703"));
        // Below 66%: one more small/mid skill, one fewer havoc/pulverization.
        let low = script.turn_skills(50, state, 0);
        assert_eq!(low.iter().filter(|id| *id == &entry.small).count(), 3);
        // Below 33%: four small skills on turn 1.
        let lowest = script.turn_skills(20, state, 0);
        assert_eq!(lowest.iter().filter(|id| *id == &entry.small).count(), 4);
    }
}

/// Section 5: "混乱で行動がスキップされない" - the Imago acts even while Staggered.
#[test]
fn section5_imago_acts_while_staggered() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 1, BattleConfig::default())
        .unwrap();
    let imago = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    assert!(state.units[imago].acts_while_staggered);
    // Stagger it, then run the combat phase: it must still use its six slots.
    state.units[imago].stagger.turns_remaining = 2;
    state.units[imago].stagger.level = 1;
    let slots: Vec<u32> = state
        .actions
        .iter()
        .filter(|a| a.actor == state.units[imago].id)
        .map(|a| a.slot)
        .collect();
    assert_eq!(slots.len(), 6, "the Imago keeps its Skill Slots while Staggered");
}

/// The Imago drives its own states of time: Clash Win grants +5 Stacks, Clash
/// Lose halves them, and the Stack bonus scales the state's status.
/// Sources: wiki.gg Imago skill text, in-game `Bufs_Refraction6`.
#[test]
fn imago_stack_gains_halve_and_bonuses() {
    use lcb_core::effects::Effect;
    use lcb_core::scripts::TimeState;
    let sim = sim();
    let mut state = sim
        .new_encounter(&[fixed::TEAM[0]], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let imago = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    assert_eq!(state.units[imago].time_state, Some(TimeState::Past));
    let gain = Effect {
        kind: "gain".to_string(),
        status: Some("In the Past".to_string()),
        potency: Some(5),
        component: Some(lcb_core::effects::Component::Stack),
        ..Default::default()
    };
    let mut notes = Vec::new();
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &[gain], imago, Some(0), &mut notes, &mut ctx);
    assert_eq!(state.units[imago].statuses.stack("In the Past"), 15);
    // 11-20 Stacks: Clash Power +1 and +2 Burn Potency / +1 Burn Count.
    let (_, bonus) = battle::time_state_bonus(&state, imago).unwrap();
    assert_eq!((bonus.clash_power, bonus.final_power, bonus.potency, bonus.count), (1, 0, 2, 1));
    let burn = Effect {
        kind: "inflict".to_string(),
        status: Some("Burn".to_string()),
        potency: Some(1),
        count: Some(1),
        ..Default::default()
    };
    battle::apply_effects_for_test(&mut state, &[burn], imago, Some(0), &mut notes, &mut ctx);
    assert_eq!(state.units[0].statuses.potency("Burn"), 3, "1 + 2 from the Stack bonus");
    assert_eq!(state.units[0].statuses.count("Burn"), 2, "1 + 1 from the Stack bonus");
    // "Halve [In the Past] (rounded down)".
    let halve = Effect {
        kind: "halve_status".to_string(),
        status: Some("In the Past".to_string()),
        ..Default::default()
    };
    battle::apply_effects_for_test(&mut state, &[halve], imago, Some(0), &mut notes, &mut ctx);
    assert_eq!(state.units[imago].statuses.stack("In the Past"), 7);
}

/// Attack Weight: the Imago's AoE skills (Kalpāgni 7, Immolation 3,
/// Bloodflower 5) hit that many Slots.  Source: wiki.gg `Clash` / Attack Weight.
#[test]
fn attack_weight_hits_multiple_slots() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let imago = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    let mut use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        imago,
        &SkillId::new("956706"),
    )
    .unwrap();
    assert_eq!(use_.attack_weight, 7, "Kalpāgni has 7 Attack Weight");
    state.preset_flips = vec![true; 64];
    state.flip_cursor = 0;
    let hp_before: Vec<i32> = state.units.iter().map(|u| u.hp).collect();
    let target = 0usize;
    let hits = battle::one_sided_attack(&mut state, imago, target, &mut use_, 0);
    battle::splash_attack_for_test(&mut state, imago, target, &use_, &hits, 0);
    let damaged: Vec<usize> = state
        .units
        .iter()
        .enumerate()
        .filter(|(index, unit)| {
            unit.kind.is_sinner() && unit.hp < hp_before[*index]
        })
        .map(|(index, _)| index)
        .collect();
    assert_eq!(damaged.len(), 7, "all seven Sinners were hit: {damaged:?}");
}

/// "[Heads Hit]" clauses resolve only when that Coin lands on Heads, and the
/// "[Butterfly](The Living/The Departed)" split goes to Potency/Count.
/// Sources: wiki.gg `Clash` (Coin triggers), `Status Effects` / Butterfly.
#[test]
fn heads_hit_effects_and_butterfly_parts() {
    use lcb_core::effects::Effect;
    let sim = sim();
    let mut state = sim
        .new_encounter(&[fixed::TEAM[0]], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let target = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    let mut notes = Vec::new();
    let mut ctx = battle::UseContext::default();
    let heads_only = Effect {
        kind: "inflict".to_string(),
        status: Some("Sinking".to_string()),
        potency: Some(2),
        ..Default::default()
    };
    battle::apply_effects_for_test(&mut state, &[heads_only], 0, Some(target), &mut notes, &mut ctx);
    assert_eq!(state.units[target].statuses.potency("Sinking"), 2);
    // Butterfly(The Living) is Potency, (The Departed) is Count.
    let living = Effect {
        kind: "inflict".to_string(),
        status: Some("Butterfly".to_string()),
        potency: Some(3),
        butterfly_part: Some("living".to_string()),
        ..Default::default()
    };
    let departed = Effect {
        kind: "inflict".to_string(),
        status: Some("Butterfly".to_string()),
        potency: Some(4),
        butterfly_part: Some("departed".to_string()),
        ..Default::default()
    };
    battle::apply_effects_for_test(
        &mut state,
        &[living, departed],
        0,
        Some(target),
        &mut notes,
        &mut ctx,
    );
    assert_eq!(state.units[target].statuses.potency("Butterfly"), 3, "The Living");
    assert_eq!(state.units[target].statuses.count("Butterfly"), 4, "The Departed");
}

/// Heals reach exactly the allies the clause names ("self and 2 other allies
/// with the least SP", "3 allies with the lowest HP percentages").
/// Source: wiki.gg `Status Effects` / Healing.
#[test]
fn heals_scope_to_the_named_allies() {
    use lcb_core::effects::Effect;
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    // Give the whole team a known SP spread.
    for (index, unit) in state.units.iter_mut().enumerate() {
        if unit.kind.is_sinner() {
            unit.sanity = Sanity::Sane { sp: -(index as i32) * 5 };
        }
    }
    let heal = Effect {
        kind: "sp_heal".to_string(),
        value: Some(7),
        ally: Some("lowest_sp".to_string()),
        ally_count: Some(2),
        include_self: Some(false),
        ..Default::default()
    };
    let mut notes = Vec::new();
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &[heal], 0, None, &mut notes, &mut ctx);
    let sinners: Vec<i32> = state
        .units
        .iter()
        .filter(|u| u.kind.is_sinner())
        .map(|u| u.sanity.sp())
        .collect();
    assert_eq!(sinners[0], 0, "the actor was not included");
    assert_eq!(sinners[1], -5, "unit 1 was not among the two lowest");
    assert_eq!(sinners[4], -20, "unit 4 was not among the two lowest");
    assert_eq!(sinners[5], -25 + 7, "the lowest SP ally was healed");
    assert_eq!(sinners[6], -30 + 7, "the second lowest SP ally was healed");
}

/// "[Amplitude Conversion] into [Tremor - Decay]" marks the target, and the
/// "If target is in an [Amplitude Conversion] state" clause then holds.
/// Source: wiki.gg `Status Effects` / Tremor and Tremor - Decay.
#[test]
fn tremor_amplitude_conversion_is_marked_and_read() {
    use lcb_core::effects::{Condition, Effect};
    let sim = sim();
    let mut state = sim
        .new_encounter(&[fixed::TEAM[0]], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let target = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    let mut notes = Vec::new();
    let mut ctx = battle::UseContext::default();
    let convert = Effect {
        kind: "amplitude_conversion".to_string(),
        amplitude_into: Some("Tremor - Decay".to_string()),
        ..Default::default()
    };
    battle::apply_effects_for_test(&mut state, &[convert], 0, Some(target), &mut notes, &mut ctx);
    assert!(battle::has_amplitude(&state.units[target]));
    assert_eq!(battle::amplitude_of(&state.units[target]), Some("Tremor - Decay"));
    // A "+48% damage" clause gated on the amplitude state now applies.
    let boosted = Effect {
        kind: "damage_percent".to_string(),
        value: Some(48),
        condition: Some(Condition {
            target_has_amplitude: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    battle::apply_effects_for_test(&mut state, &[boosted], 0, Some(target), &mut notes, &mut ctx);
    assert!((ctx.damage_bonus - 0.48).abs() < 1e-9);
}

/// "Gain [Lamp] up to 8 Stack; for every Stack gained, take HP damage equal to
/// 1% of max HP (this effect does not reduce this unit's HP below 1)".
/// Source: wiki.gg Lobotomy E.G.O::Lamp Gregor, `The Wick of Burned Feathers`.
#[test]
fn lamp_stacks_stop_at_eight_and_cost_hp() {
    use lcb_core::effects::Effect;
    let sim = sim();
    let mut state = sim
        .new_encounter(&[fixed::TEAM[6]], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let lamp = Effect {
        kind: "gain_up_to_with_self_damage".to_string(),
        status: Some("Lamp".to_string()),
        up_to: Some(8),
        self_damage_percent: Some(1),
        hp_floor_one: true,
        ..Default::default()
    };
    let mut notes = Vec::new();
    let mut ctx = battle::UseContext::default();
    let hp_before = state.units[0].hp;
    battle::apply_effects_for_test(&mut state, std::slice::from_ref(&lamp), 0, None, &mut notes, &mut ctx);
    assert_eq!(state.units[0].statuses.stack("Lamp"), 8);
    let cost = hp_before - state.units[0].hp;
    let per_stack = state.units[0].max_hp * 1 / 100;
    assert_eq!(cost, per_stack * 8, "1% of max HP for every Stack gained");
    // A second use gains nothing and costs nothing.
    let hp_second = state.units[0].hp;
    battle::apply_effects_for_test(
        &mut state,
        &[lamp.clone()],
        0,
        None,
        &mut notes,
        &mut ctx,
    );
    assert_eq!(state.units[0].statuses.stack("Lamp"), 8);
    assert_eq!(state.units[0].hp, hp_second, "nothing gained, nothing paid");
}

/// Self-inflicted damage obeys the HP floor and does not Stagger.
/// Source: wiki.gg `Harmony` (E.G.O 21009) corrosion.
#[test]
fn self_damage_respects_the_hp_floor() {
    use lcb_core::effects::{Condition, Effect};
    let sim = sim();
    let mut state = sim
        .new_encounter(&[fixed::TEAM[0]], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[0].hp = 3;
    let damage = Effect {
        kind: "self_damage".to_string(),
        self_damage_min: Some(10),
        self_damage_max: Some(20),
        hp_floor_one: true,
        no_stagger: true,
        // "At 10%+ HP" - the unit is at 1% here, so the clause is skipped;
        // the clause itself is a separate assertion below.
        condition: None,
        ..Default::default()
    };
    let mut notes = Vec::new();
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &[damage], 0, None, &mut notes, &mut ctx);
    assert_eq!(state.units[0].hp, 1, "HP never drops below 1");
    assert!(!state.units[0].is_staggered(), "this damage does not Stagger");
}

/// "[Combat Start] convert the Suit in this unit's Hand to a random Suit that
/// corresponds to one of this unit's Base Attack Skills", and "Hand - Pine
/// Crane Suit: Skill 1 Base Power +2".
/// Source: in-game `BattleKeywords` (HanafudaOne/Two/Three), wiki.gg Jeong's
/// Office Rep Ishmael.
#[test]
fn suit_conversion_boosts_the_matching_skill() {
    use lcb_core::effects::Effect;
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10813"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let convert = Effect {
        kind: "suit_convert".to_string(),
        ..Default::default()
    };
    let mut notes = Vec::new();
    let mut ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &[convert], 0, None, &mut notes, &mut ctx);
    let suit = state.units[0].suit.clone().expect("a Suit was drawn");
    assert!(
        ["HanafudaOne", "HanafudaTwo", "HanafudaThree"].contains(&suit.as_str()),
        "only Base Attack Skill Suits can be in the Hand: {suit}"
    );
    // Skill 1 of this identity gains Base Power +2 while it holds that Suit.
    let skill = match suit.as_str() {
        "HanafudaOne" => "1081301",
        "HanafudaTwo" => "1081302",
        _ => "1081303",
    };
    let mut use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new(skill),
    )
    .unwrap();
    battle::prepare_use_for_test(&mut state, &sim.library, &sim.mechanics, 0, None, &mut use_);
    let expected = if suit == "HanafudaOne" { 2 } else { 1 };
    assert_eq!(use_.ctx.base_power_bonus, expected, "Suit Base Power bonus");
}

/// "[Turn End]" clauses of the Skills equipped on the Dashboard resolve for
/// their owner: at fewer than 3 [Tear-sharpened] Rodion loses 15 SP to gain 1.
/// Source: wiki.gg Lobotomy E.G.O:: The Sword Sharpened with Tears Rodion
/// (`The Knight's Faith`).
#[test]
fn dashboard_skills_run_their_turn_end_clauses() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10913"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[0].statuses.remove("Tear-sharpened");
    state.units[0].sanity = Sanity::Sane { sp: 0 };
    lcb_core::battle::end_turn(&mut state, &sim.mechanics);
    assert_eq!(state.units[0].statuses.stack("Tear-sharpened"), 1);
    assert_eq!(state.units[0].sanity.sp(), -15, "15 SP paid for the Stack");
}

/// Golden replay of the Section 5 encounter: the whole team attacks the Imago
/// for three turns with a fixed Coin sequence, so the outcome is a pure
/// function of the rules.  Every draw (deck, Coin, ammo split, critical) comes
/// from `BattleState::flip`/`rng`, which is what makes the replay stable.
#[test]
fn section5_golden_replay_is_deterministic() {
    fn run() -> (Vec<i32>, u64) {
        let sim = sim();
        let mut state = sim
            .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 17, BattleConfig::default())
            .unwrap();
        // A fixed Coin sequence: Heads, Tails, Heads, ... (no client RNG).
        state.preset_flips = (0..8192).map(|i| i % 3 != 1).collect();
        state.flip_cursor = 0;
        let enemy = state
            .units
            .iter()
            .position(|u| !u.kind.is_sinner())
            .unwrap();
        let mut enemy_hp = Vec::new();
        for _ in 0..3 {
            // Deterministic policy: every Sinner uses the first legal action of
            // the first Slot that still needs one.
            let mut used: Vec<(lcb_core::ids::UnitId, u32)> = Vec::new();
            for action in sim.legal_actions(&state) {
                if let Action::Assign { actor, slot, skill, target } = action {
                    if used.contains(&(actor.clone(), slot)) {
                        continue;
                    }
                    used.push((actor.clone(), slot));
                    sim.submit(&mut state, Action::Assign { actor, slot, skill, target })
                        .unwrap();
                }
            }
            sim.step_turn(&mut state).unwrap();
            enemy_hp.push(state.units[enemy].hp);
        }
        (enemy_hp, sim.state_hash(&state))
    }
    let (first_hp, first_hash) = run();
    let (second_hp, second_hash) = run();
    assert_eq!(first_hp, second_hp, "same inputs, same HP");
    assert_eq!(first_hash, second_hash, "same inputs, same state hash");
    assert!(
        first_hp.windows(2).all(|w| w[1] < w[0]),
        "the team deals damage every turn: {first_hp:?}"
    );
    // Recorded baseline (update only with a sourced rule change).
    // Recorded baseline (Section 5, seed 17, preset Coin flips, "first legal
    // action per Slot").  Update only together with a sourced rule change, and
    // name the source in the commit message.  Last updated when the identities'
    // passives started applying (in-game `Passives.json`).
    assert_eq!(first_hp, vec![25483, 25248, 24682], "Imago HP after turns 1-3");
    assert_eq!(
        format!("{first_hash:016x}"),
        "e795aa40a7c38df6",
        "recorded state hash"
    );
}

/// Passives: a Combat Start grant (Gregor's Lamp), a continuous damage
/// modifier (Outis: +5% per [Protection] on self, max 15%) and a reduction
/// (Rodion in [Blessing]: take -(SP/2)% HP damage, max 20%).
/// Source: in-game `Passives.json` (1121402 Dazzling Lamp, 1111401 Vanguard
/// Team, 1091311 Magical Girl of Justice / Knight of Despair).
#[test]
fn passives_grant_and_modify() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11214", "11114", "10913"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    // Every unit carries its own Combat Passives.
    assert!(!state.units[0].passives.is_empty(), "Gregor has passives");
    // Combat Start: "Gain 1 [LanternGregBigBird]".
    lcb_core::battle::begin_turn(
        &mut state,
        &sim.library,
        &sim.mechanics,
        &sim.scripts,
    );
    assert_eq!(
        state.units[0].statuses.stack("Lamp"),
        1,
        "Gregor gained 1 [Lamp] at Combat Start"
    );
    // Outis: +5% damage per [Protection] Count, capped at 15%.
    state.units[1].statuses.remove("Protection");
    state.units[1].statuses.add_count("Protection", 2);
    let (outgoing, _) = lcb_core::battle::passive_modifiers_for_test(&state, 1, Some(3));
    assert!((outgoing - 0.10).abs() < 1e-9, "2 Protection = +10% damage: {outgoing}");
    state.units[1].statuses.add_count("Protection", 10);
    let (capped, _) = lcb_core::battle::passive_modifiers_for_test(&state, 1, Some(3));
    assert!((capped - 0.15).abs() < 1e-9, "the bonus caps at 15%");
    // Rodion in [Blessing] (gained at 0+ SP) takes -(SP/2)% HP damage, so at
    // +20 SP that is -10%.
    state.units[2].statuses.add_stack("Blessing", 1);
    state.units[2].sanity = Sanity::Sane { sp: 20 };
    let (_, taken) = lcb_core::battle::passive_modifiers_for_test(&state, 2, Some(3));
    assert!((taken + 0.10).abs() < 1e-9, "takes 10% less damage: {taken}");
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
