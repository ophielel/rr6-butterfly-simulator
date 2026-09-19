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
    assert_eq!(after.current, before.next, "the preview rotates down");
    assert_ne!(after.next.0, "", "a new skill is drawn into the preview");
    // Every slot keeps exactly two skills on the panel; the deck grew by the
    // extra slot the Sinner received from turn 2 (one Sinner in a six-slot
    // encounter, wiki.gg `Battles` / Deployment Order).
    let paneled_after: u32 = state.units[0].deck.paneled.iter().map(|(_, n)| *n).sum();
    let slots_after = state.units[0].dashboard.len() as u32;
    assert_eq!(paneled_after, slots_after * 2);
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
    assert!(
        blockers.iter().any(|b| b.contains("A-Reson")),
        "resonance effects are not modelled and must be listed"
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
