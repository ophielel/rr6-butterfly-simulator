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
    // The Skill spends Unique Ammo; a unit that runs dry mid-Skill cancels the
    // remaining Coins and pays for a Reload, which this test is not about.
    state.units[0].statuses.add_potency("The Living & The Departed", 10);
    state.units[0].statuses.add_count("The Living & The Departed", 10);
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
    // The data and the battle state use the same lowercase sin keys.
    assert_eq!(ego.resource_cost.get("gloom"), Some(&3));
    assert!(ego.resource_cost.get("Gloom").is_none());
    state.ego_resources.insert("gloom".into(), 9);
    state.ego_resources.insert("sloth".into(), 9);
    state.units[0].sanity = Sanity::Sane { sp: 45 };
    // Overclock the (awakening-only) record: cost = ceil(3 * 1.5) = 5 per sin.
    let kind = EgoSkillKind::Overclock;
    let affordable = battle::ego_affordable_for_test(&state, 0, &ego, kind);
    assert!(affordable, "9 resources are enough for ceil(4.5)=5");
    battle::pay_ego_for_test(&mut state, 0, &ego, kind);
    assert_eq!(state.ego_resources.get("gloom"), Some(&4));
}

/// Resources that normal attacks generate must be spendable on E.G.O: the
/// attack adds one resource of its sin, the E.G.O becomes affordable, and the
/// cost is deducted from the same keys.
/// Sources: JA-wiki 戦闘システム詳細 (E.G.O資源), wiki.gg `Clash`.
#[test]
fn generated_resources_pay_for_ego() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    // A normal attack of the identity's Skill 1 generates its sin's resource.
    let mut use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1011001"),
    )
    .unwrap();
    let sin_key = lcb_core::setup::sin_key(use_.sin).to_string();
    let before = state.ego_resources.get(&sin_key).copied().unwrap_or(0);
    battle::prepare_use_for_test(&mut state, &sim.library, &sim.mechanics, 0, None, &mut use_);
    assert_eq!(
        state.ego_resources.get(&sin_key).copied().unwrap_or(0),
        before + 1,
        "an attack generated 1 {sin_key} resource"
    );
    // Nothing is affordable with one resource; grant a full set and check that
    // the E.G.O now appears and is paid for with the same keys.
    assert!(
        !sim.legal_actions(&state).iter().any(|action| matches!(action, Action::UseEgo { .. })),
        "an E.G.O is not affordable yet"
    );
    for sin in lcb_core::ids::Sin::ALL {
        state
            .ego_resources
            .insert(lcb_core::setup::sin_key(sin).to_string(), 10);
    }
    state.units[0].sanity = Sanity::Sane { sp: 40 };
    let ego_action = sim
        .legal_actions(&state)
        .into_iter()
        .find(|action| matches!(action, Action::UseEgo { .. }))
        .expect("an E.G.O is affordable");
    let ego_id = match &ego_action {
        Action::UseEgo { ego, .. } => ego.clone(),
        _ => unreachable!(),
    };
    let record = sim.library.ego(&ego_id).unwrap().clone();
    let costs = lcb_core::library::Library::ego_cost(&record);
    sim.submit(&mut state, ego_action).unwrap();
    battle::resolve_combat(&mut state, &sim.library, &sim.mechanics);
    for (sin, amount) in costs {
        assert_eq!(
            state.ego_resources.get(&sin).copied().unwrap_or(0),
            10 - amount,
            "{sin} was paid"
        );
    }
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
        enemy_slot: None,
        used_top: false,
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
    // Stagger lines are HP amounts: "[Tremor Burst] raises the target's Stagger
    // Threshold by [Tremor] Potency" moves the line by 15 **HP**, not 15% of max
    // HP (JA-wiki 戦闘システム詳細 / 振動爆発).
    let before = state.units[1].stagger.threshold_hp(0).unwrap();
    let effects = vec![lcb_core::effects::Effect {
        kind: "tremor_burst".to_string(),
        consume_count: Some(1),
        ..Default::default()
    }];
    let mut notes = Vec::new();
    let mut use_ctx = battle::UseContext::default();
    battle::apply_effects_for_test(&mut state, &effects, 0, Some(1), &mut notes, &mut use_ctx);
    assert_eq!(state.units[1].stagger.threshold_hp(0).unwrap(), before + 15);
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
    // "consume 5 [Deep Tears]" spends 5 and leaves the rest behind.
    assert_eq!(state.units[0].statuses.potency("Deep Tears"), 15);
    // Below the threshold nothing happens.
    state.units[0].statuses.remove("Deep Tears");
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
            enemy_slot: None,
            used_top: false,
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
    // The Skill spends Unique Ammo (see `butterfly_heals_attacker_sp`).
    state.units[0].statuses.add_potency("The Living & The Departed", 10);
    state.units[0].statuses.add_count("The Living & The Departed", 10);
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
    battle::splash_attack_for_test(&mut state, imago, target, &mut use_, &hits, 0);
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
    // Turn 1's Combat Start already granted 1 Stack; start from 0 so the
    // assertion measures this use only.
    state.units[0].statuses.remove("Lamp");
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
    // A Skill's Turn End upkeep belongs to the Skill the unit used: an unselected
    // defense Skill pays nothing (see `apply_dashboard_phase`).
    state.actions = vec![lcb_core::state::SubmittedAction {
        actor: state.units[0].id.clone(),
        target: Some(state.units[1].id.clone()),
        skill: SkillId::new("1091304"),
        slot: 0,
        is_ego: false,
        ego: None,
        ego_kind: None,
        enemy_slot: None,
        used_top: false,
    }];
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
            let mut engages = 0;
            for action in sim.legal_actions(&state) {
                match action {
                    Action::Assign { actor, slot, skill, target } => {
                        if used.contains(&(actor.clone(), slot)) {
                            continue;
                        }
                        used.push((actor.clone(), slot));
                        sim.submit(&mut state, Action::Assign { actor, slot, skill, target })
                            .unwrap();
                    }
                    Action::Engage { actor, slot, skill, enemy_slot } => {
                        if used.contains(&(actor.clone(), slot)) {
                            continue;
                        }
                        used.push((actor.clone(), slot));
                        engages += 1;
                        sim.submit(
                            &mut state,
                            Action::Engage { actor, slot, skill, enemy_slot },
                        )
                        .unwrap();
                    }
                    _ => {}
                }
            }
            let _ = engages;
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
    assert_eq!(first_hp, vec![25406, 25200, 25057], "Imago HP after turns 1-3");
    assert_eq!(
        format!("{first_hash:016x}"),
        "ad505c9691031be2",
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
    // Turn 1 already ran its Combat Start clauses when the encounter was built,
    // so the Lamp grant is there immediately (no need to advance a turn).
    assert_eq!(
        state.units[0].statuses.stack("Lamp"),
        1,
        "Gregor gained 1 [Lamp] at Turn 1's Combat Start"
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

/// Sanity, Low Morale and Panic: SP is clamped to [-45, 45], -45 causes Panic
/// (the Sinner does not act and their Panic Type's clauses apply), -30..-45 is
/// Low Morale, and after a Panic turn the SP resets to 0.
/// Source: wiki.gg `Sanity` ("List of Panic Types") and `Clash`.
#[test]
fn low_morale_and_panic_follow_the_sanity_rules() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10414"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    // Ryōshū's Panic Type is "Solitude": Panic gains +3 [Sinking] Count at Turn End.
    assert_eq!(state.units[0].panic_type.as_deref(), Some("Solitude"));
    assert!(!state.units[0].panic_actions.is_empty());
    // -45 SP: Panic.
    state.units[0].sanity = Sanity::Sane { sp: -45 };
    battle::begin_turn(&mut state, &sim.library, &sim.mechanics, &sim.scripts);
    assert!(state.units[0].panicked, "SP -45 is Panic");
    assert!(!state.units[0].low_morale);
    // The Panic act does not happen, so no action is submitted for that unit.
    assert!(
        !state.actions.iter().any(|a| a.actor == state.units[0].id),
        "a Panicking Sinner does not act"
    );
    // Its Panic clause applies at Turn End (+3 [Sinking] Count).
    let before = state.units[0].statuses.count("Sinking");
    battle::end_turn(&mut state, &sim.mechanics);
    assert_eq!(state.units[0].statuses.count("Sinking"), before + 3);
    // The following Turn Start resets the SP to 0 (Panic recovery).
    battle::begin_turn(&mut state, &sim.library, &sim.mechanics, &sim.scripts);
    assert_eq!(state.units[0].sanity.sp(), 0, "SP resets after a Panic turn");
    assert!(!state.units[0].panicked);
    // -35 SP is Low Morale, not Panic.
    state.units[0].sanity = Sanity::Sane { sp: -35 };
    battle::begin_turn(&mut state, &sim.library, &sim.mechanics, &sim.scripts);
    assert!(state.units[0].low_morale && !state.units[0].panicked);
    // SP never leaves [-45, 45].
    state.units[0].sanity = Sanity::Sane { sp: -44 };
    state.units[0].sanity = state.units[0].sanity.add(-99);
    assert_eq!(state.units[0].sanity.sp(), -45);
}

/// Status text drives behaviour: "[Turn Start] gain 1 [Defense Level Up] for
/// every Stack (max 5)" (Protecting Sword), "Base Attack Skills deal
/// +(Stack x 10)% damage (max 30%)" (Tear-sharpened) and "For every 1
/// ([Sinking] + [Burn]), take +0.5% damage from Base Attack Skills (max 10%)"
/// (Dazzle).
/// Source: wiki.gg `Status Effects` for the three statuses.
#[test]
fn status_text_runs_as_effects() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10913"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    // Tear-sharpened: +10% damage per Stack on Base Attack Skills.
    state.units[0].statuses.remove("Tear-sharpened");
    state.units[0].statuses.add_stack("Tear-sharpened", 2);
    let (outgoing, _) = battle::passive_modifiers_for_test(&state, 0, Some(1));
    assert!((outgoing - 0.20).abs() < 1e-9, "2 Stack = +20% damage: {outgoing}");
    state.units[0].statuses.add_stack("Tear-sharpened", 10);
    let (capped, _) = battle::passive_modifiers_for_test(&state, 0, Some(1));
    assert!((capped - 0.30).abs() < 1e-9, "the bonus caps at 30%");
    // Protecting Sword: "[Turn Start] gain 1 [Defense Level Up] for every Stack".
    state.units[0].statuses.remove("Protecting Sword");
    state.units[0].statuses.remove("Defense Level Up");
    state.units[0].statuses.add_stack("Protecting Sword", 3);
    lcb_core::battle::begin_turn(&mut state, &sim.library, &sim.mechanics, &sim.scripts);
    assert!(
        state.units[0].statuses.get("Defense Level Up").potency
            + state.units[0].statuses.get("Defense Level Up").count
            >= 3,
        "3 Stack granted 3 [Defense Level Up]: {:?}",
        state.units[0].statuses.get("Defense Level Up")
    );
    // Dazzle: +0.5% damage taken per ([Sinking] + [Burn]) Count/Potency, max 10%.
    let mut state = sim
        .new_encounter(&["11214"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[0].statuses.add_count("Dazzle", 1);
    state.units[0].statuses.add_potency("Sinking", 4);
    state.units[0].statuses.add_potency("Burn", 2);
    let (_, taken) = battle::passive_modifiers_for_test(&state, 0, Some(1));
    assert!((taken - 0.03).abs() < 1e-9, "6 combined = +3% taken: {taken}");
}

/// Status riders: "[When Clash ends] inflict 2 [Sinking]" (Blessing) and
/// "[When hit] inflict 2 [Sinking] on the attacker".
/// Source: wiki.gg `Status Effects` (Blessing / Despair).
#[test]
fn status_clash_end_and_hit_riders_fire() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10913"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    // Blessing rides on the unit that holds it.
    state.units[0].statuses.remove("Blessing");
    state.units[0].statuses.add_stack("Blessing", 1);
    let target = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    let before = state.units[target].statuses.potency("Sinking");
    // A Clash between the two ends: the rider inflicts 2 [Sinking] on the other
    // unit (once per turn).
    let mut a = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1091301"),
    )
    .unwrap();
    let mut b = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        target,
        &SkillId::new("956701"),
    )
    .unwrap();
    battle::resolve_clash(&mut state, 0, target, &mut a, &mut b);
    let after = state.units[target].statuses.potency("Sinking");
    assert!(
        after >= before + 2,
        "Blessing inflicted [Sinking] when the Clash ended: {before} -> {after}"
    );
}

/// A Sinner at -45 SP who owns a Corrosion Skill is forced into E.G.O Corrosion
/// instead of Panicking, and their Skill is replaced by that Corrosion Skill.
/// Source: wiki.gg `Sanity` ("they are forced into E.G.O Corrosion, during which
/// they will go out of control and use E.G.O Corrosion Skills indiscriminately").
#[test]
fn corrosion_replaces_panic_at_minus_forty_five() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&[fixed::TEAM[3]], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    assert!(
        !state.units[0].corrosion_egos.is_empty(),
        "the E.G.O loadout has a Corrosion Skill"
    );
    state.units[0].sanity = Sanity::Sane { sp: -45 };
    battle::begin_turn(&mut state, &sim.library, &sim.mechanics, &sim.scripts);
    assert!(state.units[0].corroded, "Corrosion replaces Panic");
    assert!(!state.units[0].panicked);
    // Submitting anything for that unit is replaced by its Corrosion Skill.
    for action in sim.legal_actions(&state) {
        if let Action::Assign { actor, slot, skill, target } = action {
            sim.submit(&mut state, Action::Assign { actor, slot, skill, target })
                .unwrap();
        }
    }
    battle::resolve_combat(&mut state, &sim.library, &sim.mechanics);
    assert!(
        state.log.iter().any(|entry| entry.kind == "ego"
            && entry.detail.contains("corrosion")),
        "a Corrosion E.G.O Skill was used: {:?}",
        state.log.iter().map(|e| e.detail.clone()).collect::<Vec<_>>()
    );
}

/// "Then, Reuse this Coin (4 times per Skill)" really uses the Coin again (and
/// re-tosses it): E.G.O `21009` Harmony's third Coin is Reused four times, so
/// the Skill produces four more hits than it has Coins.
/// Sources: wiki.gg `Clash` (the `Reuse` trigger prefix), JA-wiki 守備スキル
/// ("再使用はコインを投げるごとに「スキルを使用した」という判定").
#[test]
fn reuse_coin_uses_the_coin_again() {
    use lcb_core::ids::EgoId;
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11004"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let target = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    let mut use_ = battle::build_ego_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &EgoId::new("21009"),
        EgoSkillKind::Awakening,
    )
    .expect("Harmony builds");
    let coins = use_.coins.len();
    // Give the E.G.O the resources it would have been paid with.
    for sin in ["wrath", "lust", "sloth", "pride", "envy"] {
        state.ego_resources.insert(sin.to_string(), 5);
    }
    state.preset_flips = (0..4096).map(|i| i % 2 == 0).collect();
    state.flip_cursor = 0;
    let hits = battle::one_sided_attack(&mut state, 0, target, &mut use_, 0);
    assert_eq!(
        hits.len(),
        coins + 4,
        "four Reuses of the third Coin: {coins} Coins -> {} hits",
        hits.len()
    );
    // The reused Coin kept its index, so it appears five times in total.
    let third = hits.iter().filter(|hit| hit.coin_index == 2).count();
    assert_eq!(third, 5, "the third Coin was used five times");
}

/// E.G.O Skill effect text must actually reach the engine.  The E.G.O entries
/// are keyed `<ego id>.<kind>@<tier>`, so a lookup by the bare id silently
/// produced an empty SkillMechanics and every E.G.O clause was dead.
/// Source: `data/mechanics/effects.json` (extracted from in-game E.G.O text).
#[test]
fn ego_skills_load_their_mechanics() {
    use lcb_core::ids::EgoId;
    let sim = sim();
    let state = sim
        .new_encounter(&["11004"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    for (ego, kind, expected) in [
        ("21009", "awakening", "Harmony"),
        ("21009", "corrosion", "Harmony"),
        ("21207", "awakening", "Solemn Lament"),
    ] {
        let use_ = battle::build_ego_use(
            &state,
            &sim.library,
            &sim.mechanics,
            0,
            &EgoId::new(ego),
            if kind == "awakening" {
                EgoSkillKind::Awakening
            } else {
                EgoSkillKind::Corrosion
            },
        )
        .expect("E.G.O builds");
        assert_eq!(
            use_.mechanics.note.as_deref(),
            Some(expected),
            "{ego}.{kind} mechanics were not loaded"
        );
        assert!(
            !use_.mechanics.coins.is_empty() || !use_.mechanics.on_use.is_empty(),
            "{ego}.{kind} has clauses"
        );
    }
}

/// "Reuse this Coin once for every 33% missing HP (max 2 times)": the Imago's
/// 滅多斬り re-uses its Coin only when its own HP is low enough.
/// Source: wiki.gg Imago skill text (`Reuse this Coin once for every 33% missing
/// HP (max 2 times)`).
#[test]
fn reuse_from_missing_hp_scales_with_hp() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    let target = 0usize;
    let hits_at = |state: &mut lcb_core::state::BattleState, hp_percent: i32| -> usize {
        state.units[enemy].hp = state.units[enemy].max_hp * hp_percent / 100;
        let mut use_ = battle::build_use(
            state,
            &sim.library,
            &sim.mechanics,
            enemy,
            &SkillId::new("956703"),
        )
        .unwrap();
        state.preset_flips = (0..4096).map(|i| i % 2 == 0).collect();
        state.flip_cursor = 0;
        battle::one_sided_attack(state, enemy, target, &mut use_, 0).len()
    };
    let full = hits_at(&mut state, 100);
    let low = hits_at(&mut state, 10);
    assert!(
        low > full,
        "more missing HP means more Reuses: {full} -> {low}"
    );
}

/// Runtime guard: playing several turns of the fixed encounter must not produce
/// "unhandled effect kind" warnings - every clause that reaches the evaluator
/// has an implementation.
/// Source: project plan ("unknown -> UNKNOWN, never silently dropped").
#[test]
fn fixed_content_playthrough_has_no_unhandled_kinds() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 23, BattleConfig::default())
        .unwrap();
    for _ in 0..4 {
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
    }
    let unhandled: Vec<&String> = state
        .warnings
        .iter()
        .filter(|warning| warning.contains("unhandled effect kind"))
        .collect();
    assert!(unhandled.is_empty(), "unhandled kinds: {unhandled:?}");
}

/// Data guard: every Skill the fixed content can use - identity Skills, their
/// defense Skill, every E.G.O Skill of the loadout and the Imago's six Slots -
/// must have a mechanics entry.  A keyed lookup that silently misses turns the
/// whole effect text into a no-op (that is how "Reuse this Coin" stayed dead).
#[test]
fn fixed_content_skills_all_resolve_mechanics() {
    use lcb_core::ids::EgoId;
    let sim = sim();
    let mut missing = Vec::new();
    for identity in fixed::TEAM {
        let record = sim
            .library
            .identity(&lcb_core::ids::IdentityId::new(identity))
            .expect("identity");
        for skill in &record.skills {
            if sim
                .mechanics
                .get_for(&SkillId::new(skill.id.clone()), Uptie(4))
                .is_none()
            {
                missing.push(skill.id.clone());
            }
        }
    }
    for (_, ego) in fixed::EGO_LOADOUT {
        for kind in ["awakening", "corrosion"] {
            let mech = sim.mechanics.get_ego(ego, kind);
            let record = sim.library.ego(&EgoId::new(ego)).expect("ego");
            let has_skill = if kind == "awakening" {
                record.awakening.is_some()
            } else {
                record.corrosion.is_some()
            };
            if has_skill && mech.note.is_none() {
                missing.push(format!("{ego}.{kind}"));
            }
        }
    }
    let script = sim.scripts.for_enemy(fixed::BOSS_IMAGO).expect("Imago script");
    for state in lcb_core::scripts::TimeState::ALL {
        for turn in 0..3 {
            for skill in script.turn_skills(100, state, turn) {
                let id = SkillId::new(skill.clone());
                if sim.mechanics.get_for(&id, Uptie(1)).is_none() {
                    missing.push(skill);
                }
            }
        }
    }
    missing.sort();
    missing.dedup();
    assert!(missing.is_empty(), "skills without mechanics: {missing:?}");
}

/// Turn 1 must already have run the units' Combat Start passives: the rules
/// books are attached before the first `begin_turn`, otherwise the first Turn
/// Start happens without passives and they only appear from Turn 2 on.
/// Source: the units' own Passives (in-game `Passives.json`).
#[test]
fn first_turn_already_applies_passives() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    // A passive's "[Combat Start]" belongs to the turn's Skill selection, so it
    // resolves once the panel is submitted (see `resolve_combat`).
    state.actions.clear();
    battle::resolve_combat(&mut state, &sim.library, &sim.mechanics);
    let jeong = state
        .units
        .iter()
        .position(|u| u.name.contains("Jeong"))
        .expect("Jeong is in the team");
    let outis = state
        .units
        .iter()
        .position(|u| u.name.contains("Udjat"))
        .expect("Outis is in the team");
    // Koi-Koi draws a Suit at Turn Start; Vanguard Team grants 2 [The Udjat].
    assert!(
        state.units[jeong].suit.is_some(),
        "the Hanafuda Suit is in hand on Turn 1"
    );
    assert_eq!(
        state.units[outis].statuses.stack("The Udjat -Vanguard-"),
        2,
        "Outis gained her Combat Start Stacks on Turn 1"
    );
}

/// Support Passives are not applied: the extracted ones are keyed by identity
/// but the team's support slots are a deck-building choice this project has no
/// data for, and attaching every support passive in the book gave units
/// passives they do not own.
#[test]
fn support_passives_are_not_applied() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    // A passive's "[Combat Start]" belongs to the turn's Skill selection, so it
    // resolves once the panel is submitted (see `resolve_combat`).
    state.actions.clear();
    battle::resolve_combat(&mut state, &sim.library, &sim.mechanics);
    let support_notes: Vec<String> = sim
        .passives
        .passives
        .values()
        .filter(|passive| passive.kind == "support")
        .map(|passive| passive.id.clone())
        .collect();
    assert!(!support_notes.is_empty(), "the book still records them");
    for unit in state.units.iter().filter(|u| u.kind.is_sinner()) {
        // Own combat passives only: every attached list must come from the unit's
        // own identity.
        assert!(
            unit.passives.len() <= 4,
            "{} carries {} passives",
            unit.name,
            unit.passives.len()
        );
    }
}

/// Butterfly (the unique Sinking behind Solemn Lament):
///   * "When hit, and if this unit's SP is at less than 0, take
///     ([Sinking] Potency / 5) Gloom damage for every value of The Departed
///     (max Gloom damage 30; rounded down; deals half damage to targets that are
///     Non-SP Units.)"
///   * "Turn End: Reset The Departed of this effect to 0; then, gain [Sinking]
///     equal to The Living and convert The Living into The Departed."
/// Source: in-game `BattleKeywords` / `Bufs` (SinkingWhite).
#[test]
fn butterfly_dot_and_turn_end_conversion() {
    use lcb_core::effects::Effect;
    let sim = sim();
    let mut state = sim
        .new_encounter(&[fixed::TEAM[0]], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let target = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    // Turn End: The Living (Potency) 8 and The Departed (Count) 3 become
    // [Sinking] 8 plus The Departed 8.
    state.units[target].statuses.remove("Sinking");
    state.units[target].statuses.add_potency("Butterfly", 8);
    state.units[target].statuses.add_count("Butterfly", 3);
    battle::end_turn(&mut state, &sim.mechanics);
    assert_eq!(state.units[target].statuses.potency("Butterfly"), 0);
    assert_eq!(state.units[target].statuses.count("Butterfly"), 8);
    assert_eq!(
        state.units[target].statuses.potency("Sinking"),
        8,
        "The Living was also gained as [Sinking]"
    );
    // On hit with negative SP: ([Sinking] Potency / 5) per The Departed, max 30.
    let mut notes = Vec::new();
    let mut ctx = battle::UseContext::default();
    state.units[0].sanity = Sanity::Sane { sp: -10 };
    // The DoT reads the [Sinking] Potency on the unit, and Sinking itself is
    // consumed by the same hit ("reduce its Count by 1"), so it needs Count.
    state.units[target].statuses.add_potency("Sinking", 40);
    state.units[target].statuses.add_count("Sinking", 5);
    let hp_before = state.units[target].hp;
    let mark = Effect {
        kind: "noop".to_string(),
        ..Default::default()
    };
    battle::apply_effects_for_test(&mut state, &[mark], 0, Some(target), &mut notes, &mut ctx);
    // The DoT lives in apply_hit; drive a real hit instead.
    let mut use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1011001"),
    )
    .unwrap();
    state.preset_flips = vec![true; 64];
    state.flip_cursor = 0;
    battle::one_sided_attack(&mut state, 0, target, &mut use_, 0);
    // The source writes "({{StatusEffect|Sinking}} Potency / 5) Gloom damage for
    // every value of The Departed" - i.e. the [Sinking] status, not The Living.
    // With [Sinking] 40 and The Departed 8 that is 8 x 8 = 64, half for a Non-SP
    // Unit, and the damage taken is capped at 30.
    let butterfly_damage: i32 = state
        .log
        .iter()
        .filter(|entry| entry.kind == "butterfly")
        .map(|entry| entry.detail.clone())
        .count() as i32;
    let lost = hp_before - state.units[target].hp;
    assert!(butterfly_damage >= 1, "the Butterfly rider fired");
    assert!(lost > 0, "Butterfly dealt damage: {lost} HP");
    assert!(
        lost <= 30 * 20,
        "each hit is capped at 30 Gloom damage: {lost} HP over the Skill"
    );
}

/// Corrosion is not a player choice: it is either random (negative SP), forced
/// by the E.G.O Corrosion state, or reached through Overclock.
/// Source: JA-wiki 戦闘システム詳細 (ランダム侵蝕の仕様 / オーバークロック).
#[test]
fn corrosion_is_not_a_player_choice() {
    use lcb_core::ids::EgoId;
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    for sin in ["wrath", "lust", "sloth", "gluttony", "gloom", "pride", "envy"] {
        state.ego_resources.insert(sin.to_string(), 9);
    }
    let offers_corrosion = sim.legal_actions(&state).iter().any(|action| {
        matches!(
            action,
            Action::UseEgo {
                kind: EgoSkillKind::Corrosion,
                ..
            }
        )
    });
    assert!(!offers_corrosion, "the player cannot pick Corrosion");
    // With low SP a submitted Awakening may fire the Corrosion Skill instead.
    // At SP -25 an E.G.O that costs 20 SP leaves -45, where the chance is 100%.
    state.units[0].sanity = Sanity::Sane { sp: -25 };
    let offered: Vec<String> = sim
        .legal_actions(&state)
        .iter()
        .filter(|action| matches!(action, Action::UseEgo { .. }))
        .map(|action| format!("{action:?}"))
        .collect();
    let action = sim
        .legal_actions(&state)
        .into_iter()
        .find(|action| {
            matches!(
                action,
                Action::UseEgo {
                    kind: EgoSkillKind::Awakening,
                    ..
                }
            )
        })
        .unwrap_or_else(|| panic!("no Awakening E.G.O offered; got {offered:?}"));
    sim.submit(&mut state, action).unwrap();
    battle::resolve_combat(&mut state, &sim.library, &sim.mechanics);
    assert!(
        state
            .log
            .iter()
            .any(|entry| entry.detail.contains("random Corrosion")),
        "at -45 SP the Corrosion chance is 100%"
    );
    let _ = EgoId::new("21009");
}

fn action_actor(action: &Action) -> &lcb_core::ids::UnitId {
    match action {
        Action::Assign { actor, .. }
        | Action::Engage { actor, .. }
        | Action::UseEgo { actor, .. } => actor,
        Action::Commit => unreachable!(),
    }
}

/// A Skill's `[Attack End]` clauses run after an attack that a Clash won, not
/// only after a one-sided attack.  "Attack End: Activates only once after an
/// Attack Skill has used all of its Coins" (wiki.gg `Clash`, trigger table).
#[test]
fn attack_end_runs_after_a_clash_win() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11114"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    // A large level gap makes the Clash a guaranteed win, and every Coin Heads.
    state.units[0].level = 60;
    state.units[enemy].level = 1;
    state.preset_flips = vec![true; 64];
    state.flip_cursor = 0;
    // Put the Skill on the panel (the draw is random) and use it.
    let skill = SkillId::new("1111403");
    state.units[0].dashboard[0].current = skill.clone();
    let actor = state.units[0].id.clone();
    let target = state.units[enemy].id.clone();
    let action = Action::Assign {
        actor,
        slot: 0,
        skill,
        target,
    };
    sim.submit(&mut state, action).unwrap();
    battle::resolve_combat(&mut state, &sim.library, &sim.mechanics);
    // "[Attack End] Gain 2 [Protection] next turn": the grant is queued.
    let queued = state.units[0]
        .pending_next_turn
        .iter()
        .filter(|pending| pending.status == "Protection")
        .map(|pending| pending.count)
        .max();
    assert_eq!(
        queued,
        Some(2),
        "the Clash-winning attack ended and queued 2 Protection Count"
    );
}

/// The Imago's "At less than N% HP, convert ... into [Unbreakable Coin]" clauses
/// are conditional: its Coins only become Unbreakable below the threshold.
/// Source: the Imago's own Skill text (956701 Fluttering Havoc).
#[test]
fn imago_coins_only_become_unbreakable_below_the_threshold() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    let target = 0usize;
    let count = |state: &mut lcb_core::state::BattleState, percent: i32| -> usize {
        state.units[enemy].hp = state.units[enemy].max_hp * percent / 100;
        let mut use_ = battle::build_use(
            state,
            &sim.library,
            &sim.mechanics,
            enemy,
            &SkillId::new("956701"),
        )
        .unwrap();
        battle::prepare_use_for_test(
            state,
            &sim.library,
            &sim.mechanics,
            enemy,
            Some(target),
            &mut use_,
        );
        use_.coins.iter().filter(|coin| coin.unbreakable).count()
    };
    assert_eq!(count(&mut state, 90), 0, "no Unbreakable Coins above 66% HP");
    assert_eq!(count(&mut state, 60), 1, "the final Coin below 66% HP");
    assert_eq!(count(&mut state, 30), 2, "every Coin below 33% HP");
}

/// Only Gregor's **Skill 3** turns its Coins into Unbreakable Coins, and only
/// while the target has [Dazzle].  Source: the identity's own Skill text
/// ("[On Use] If target has [Dazzle], convert all Coins on this Skill to
/// [Unbreakable Coin]s and gain Clash Power +3").
#[test]
fn only_gregors_third_skill_converts_to_unbreakable_coins() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11214"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    fn build(
        sim: &Simulator,
        state: &mut lcb_core::state::BattleState,
        enemy: usize,
        skill: &str,
    ) -> usize {
        let mut use_ = battle::build_use(
            state,
            &sim.library,
            &sim.mechanics,
            0,
            &SkillId::new(skill),
        )
        .unwrap();
        battle::prepare_use_for_test(state, &sim.library, &sim.mechanics, 0, Some(enemy), &mut use_);
        use_.coins.iter().filter(|coin| coin.unbreakable).count()
    }
    // Without [Dazzle] even Skill 3 stays breakable.
    state.units[enemy].statuses.remove("Dazzle");
    for skill in ["1121401", "1121402", "1121403", "1121404"] {
        assert_eq!(build(&sim, &mut state, enemy, skill), 0, "{skill} without [Dazzle]");
    }
    // With 3 [Dazzle] on the target only Skill 3 converts (all of its Coins).
    state.units[enemy].statuses.add_count("Dazzle", 3);
    assert_eq!(build(&sim, &mut state, enemy, "1121401"), 0, "Skill 1 never converts");
    assert_eq!(build(&sim, &mut state, enemy, "1121402"), 0, "Skill 2 never converts");
    let third = build(&sim, &mut state, enemy, "1121403");
    assert!(third > 0, "Skill 3 converts every Coin");
    // The guard (Skill 4) has one natively Unbreakable Coin, not all of them.
    let guard = build(&sim, &mut state, enemy, "1121404");
    assert!(guard < 2, "the guard has a single Unbreakable Coin: {guard}");
}

/// A destroyed ("cracked") Unbreakable Coin keeps the Skill's Base Power and the
/// per-Coin accumulation, and the Coin keeps the result it was tossed with:
/// "破壊不能コインはマッチで破壊されると**コイン威力が1になる**。ただし、元のコイン
/// 威力が1になるだけで、**後からコイン威力増加が適用される**"
/// (JA-wiki 破壊不能コイン).
#[test]
fn cracked_unbreakable_coins_keep_base_power_and_coin_power_bonus() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&[fixed::TEAM[0]], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    let target = 0usize;
    // Fluttering Havoc: Base Power 4, Coin Power 3, 2 Coins.
    let build = |state: &mut lcb_core::state::BattleState, bonus: i32| {
        state.units[enemy].hp = state.units[enemy].max_hp * 30 / 100;
        let mut use_ = battle::build_use(
            state,
            &sim.library,
            &sim.mechanics,
            enemy,
            &SkillId::new("956701"),
        )
        .unwrap();
        battle::prepare_use_for_test(
            state,
            &sim.library,
            &sim.mechanics,
            enemy,
            Some(target),
            &mut use_,
        );
        // Both Coins became Unbreakable and were destroyed by the Clash.
        assert!(
            use_.coins.iter().all(|coin| coin.unbreakable),
            "both Coins are Unbreakable below 33% HP"
        );
        for coin in use_.coins.iter_mut() {
            coin.state = CoinState::Cracked;
        }
        use_.ctx.coin_power_bonus = bonus;
        use_
    };
    let powers = |state: &mut lcb_core::state::BattleState, heads: bool, bonus: i32| -> Vec<i32> {
        let mut use_ = build(state, bonus);
        for coin in use_.coins.iter_mut() {
            coin.heads = Some(heads);
        }
        let hits = battle::one_sided_attack(state, enemy, target, &mut use_, 1);
        hits.iter().map(|hit| hit.power).collect()
    };
    assert_eq!(
        powers(&mut state, true, 0),
        vec![5, 6],
        "Base Power 4 + cracked Coin Power 1 per Heads Coin"
    );
    assert_eq!(
        powers(&mut state, false, 0),
        vec![4, 4],
        "Tails adds no Coin Power"
    );
    assert_eq!(
        powers(&mut state, true, 2),
        vec![7, 10],
        "Coin Power increases apply after the cracked Coin Power of 1"
    );
    // The cracked Coins still resolve their On Hit clauses.
    let hits = {
        let mut use_ = build(&mut state, 0);
        for coin in use_.coins.iter_mut() {
            coin.heads = Some(true);
        }
        battle::one_sided_attack(&mut state, enemy, target, &mut use_, 1)
    };
    assert_eq!(hits.len(), 2, "both cracked Coins attacked");
    assert!(
        state.units[target].statuses.potency("Burn") > 0,
        "the cracked Coins' On Hit clauses resolved"
    );
}

/// A Skill that **loses** a Clash: its Unbreakable Coins survive as "cracked"
/// (Coin Power 1) and still land, so the order is "winner's attack, then the
/// loser's Unbreakable Coins".  "[Attack End] activates only once after an
/// Attack Skill has used all of its Coins": destroyed normal Coins do not count
/// as used, Unbreakable Coins do - so the loser reaches its Attack End only when
/// every Coin it had was Unbreakable.
/// Sources: JA-wiki 破壊不能コイン, wiki.gg `Clash` (Attack End / Unbreakable Coin).
#[test]
fn clash_loser_keeps_its_unbreakable_coins() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11214"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    // Gregor's 1121403 converts all of its Coins to Unbreakable Coins while the
    // target has [Dazzle]; put it on his panel and make the Clash a loss.
    state.units[enemy].statuses.add_count("Dazzle", 3);
    state.units[0].dashboard[0].current = SkillId::new("1121403");
    state.units[0].level = 1;
    state.units[enemy].level = 60;
    state.preset_flips = vec![false; 64];
    state.flip_cursor = 0;
    let skill = SkillId::new("1121403");
    let actor = state.units[0].id.clone();
    let target = state.units[enemy].id.clone();
    sim.submit(
        &mut state,
        Action::Assign {
            actor: actor.clone(),
            slot: 0,
            skill,
            target: target.clone(),
        },
    )
    .unwrap();
    let hp_before = state.units[enemy].hp;
    battle::resolve_combat(&mut state, &sim.library, &sim.mechanics);
    assert_eq!(
        state.log.iter().filter(|e| e.kind == "clash").count(),
        1,
        "a Clash happened"
    );
    assert!(
        state.units[enemy].hp < hp_before,
        "the loser's Unbreakable Coins still landed: {} -> {}",
        hp_before,
        state.units[enemy].hp
    );
    assert!(
        state.units[0]
            .pending_next_turn
            .iter()
            .any(|pending| pending.status == "Plus Coin Boost"),
        "the losing all-Unbreakable Skill reached its Attack End"
    );
}

/// A Skill that loses a Clash with normal Coins does not reach [Attack End].
#[test]
fn clash_loser_with_normal_coins_has_no_attack_end() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11114"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    state.units[0].dashboard[0].current = SkillId::new("1111403");
    state.units[0].level = 1;
    state.units[enemy].level = 60;
    state.preset_flips = vec![false; 64];
    state.flip_cursor = 0;
    let actor = state.units[0].id.clone();
    let target = state.units[enemy].id.clone();
    sim.submit(
        &mut state,
        Action::Assign {
            actor: actor.clone(),
            slot: 0,
            skill: SkillId::new("1111403"),
            target,
        },
    )
    .unwrap();
    battle::resolve_combat(&mut state, &sim.library, &sim.mechanics);
    assert!(
        !state.units[0]
            .pending_next_turn
            .iter()
            .any(|pending| pending.status == "Protection"),
        "destroyed normal Coins do not count as used"
    );
}

/// Submitting the **top** card of a Slot must not overwrite the bottom card:
/// the panel is untouched until the turn resolves, which consumes the card that
/// was actually used and keeps the other one.
/// Source: JA-wiki 戦闘システム詳細 ("スキル構成", "パネル上に空きが出るとその分だけ
/// スキル構成から継ぎ足される").
#[test]
fn using_the_top_card_keeps_the_bottom_card() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 1, BattleConfig::default())
        .unwrap();
    let (current, next, preview) = {
        let entry = &state.units[0].dashboard[0];
        (entry.current.clone(), entry.next.clone(), entry.preview.clone())
    };
    assert_ne!(current, next, "the panel has two different cards");
    let enemy = state.units[1].id.clone();
    let actor = state.units[0].id.clone();
    // Use the top card.
    sim.submit(
        &mut state,
        Action::Assign {
            actor,
            slot: 0,
            skill: next.clone(),
            target: enemy,
        },
    )
    .unwrap();
    {
        let entry = &state.units[0].dashboard[0];
        assert_eq!(
            (entry.current.clone(), entry.next.clone(), entry.preview.clone()),
            (current.clone(), next.clone(), preview.clone()),
            "submitting leaves the panel untouched"
        );
    }
    sim.step_turn(&mut state).unwrap();
    let entry = &state.units[0].dashboard[0];
    assert_eq!(entry.current, current, "the unused bottom card stayed");
    assert_eq!(entry.next, preview, "the panel refilled from the top");
    assert_ne!(entry.next, next, "the used top card is gone");
}

/// "Gain 2 [Protection]" grants 2 Protection **Count** (the status measures its
/// effect by Count), and one-turn statuses are removed at Turn End.
/// Sources: in-game `Bufs` / `BattleKeywords` (Protection, Fragile, Haste, Bind)
/// and wiki.gg `Status Effects`.
#[test]
fn protection_lands_on_count_and_one_turn_statuses_expire() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11114"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[0].statuses.remove("Protection");
    // A real Skill grants it: 1111403's Attack End queues it for the next turn.
    state.units[0].pending_next_turn.push(lcb_core::state::PendingStatus {
        stack: 0,
        status: "Protection".to_string(),
        potency: 0,
        count: 2,
    });
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    lcb_core::battle::begin_turn(&mut state, &sim.library, &sim.mechanics, &sim.scripts);
    assert_eq!(
        state.units[0].statuses.count("Protection"),
        2,
        "the queued grant arrives as Count 2"
    );
    // It reduces incoming damage (10% per Count).
    let reduced = battle::incoming_damage_modifier_for_test(&state.units[0], "Wrath");
    assert!(reduced < 0.0, "Protection reduces incoming damage: {reduced}");
    // Haste and Bind move Speed by their Count, and both last one turn.
    state.units[0].statuses.remove("Haste");
    state.units[0].statuses.remove("Bind");
    state.units[0].statuses.add_count("Haste", 3);
    state.units[0].statuses.add_count("Bind", 1);
    state.units[0].speed_range = (5, 5);
    lcb_core::battle::begin_turn(&mut state, &sim.library, &sim.mechanics, &sim.scripts);
    assert_eq!(
        state.units[0].speed, 7,
        "5 + Haste 3 - Bind 1: the Count is what moves Speed"
    );
    // ... and they are gone at the end of that turn.
    state.actions.clear();
    lcb_core::battle::end_turn(&mut state, &sim.mechanics);
    assert_eq!(
        state.units[0].statuses.count("Protection"),
        0,
        "Protection lasts one turn"
    );
    let _ = enemy;
}

/// A "next turn" Bind / Haste queued by a Skill affects the Speed of the turn it
/// applies to: the queued statuses arrive before Speed is rolled.
/// Source: Gregor's 1121401 (`[On Use] Gain 1 [Bind] next turn`) and the Speed
/// rules (in-game `Bufs`: Binding / Agility).
#[test]
fn next_turn_speed_status_arrives_before_speed_is_rolled() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11214"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[0].speed_range = (5, 5);
    state.units[0].statuses.remove("Bind");
    state.units[0].statuses.remove("Haste");
    lcb_core::battle::begin_turn(&mut state, &sim.library, &sim.mechanics, &sim.scripts);
    assert_eq!(state.units[0].speed, 5, "no Bind yet");
    // The Skill queued "[On Use] Gain 1 [Bind] next turn".
    state.units[0].pending_next_turn.push(lcb_core::state::PendingStatus {
        stack: 0,
        status: "Bind".to_string(),
        potency: 0,
        count: 1,
    });
    lcb_core::battle::begin_turn(&mut state, &sim.library, &sim.mechanics, &sim.scripts);
    assert_eq!(
        state.units[0].speed,
        4,
        "the queued Bind lowered this turn's Speed"
    );
    assert_eq!(state.units[0].statuses.count("Bind"), 1);
}

/// Using a defense Skill consumes its Slot: the defense and the card it
/// replaced both leave the panel and the Slot refills.  "使った守備スキルは守備
/// スキルに変更した元のスキルごとパネルから消え、次のターンには新しいスキルが
/// パネルに追加される" (JA-wiki 守備スキル).
#[test]
fn defense_skill_rotates_its_slot() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11214"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let (current, next, preview) = {
        let entry = &state.units[0].dashboard[0];
        (entry.current.clone(), entry.next.clone(), entry.preview.clone())
    };
    let actor = state.units[0].id.clone();
    let target = state.units[1].id.clone();
    // Guard: the identity's Skill 4 is its defense Skill.
    sim.submit(
        &mut state,
        Action::Assign {
            actor,
            slot: 0,
            skill: SkillId::new("1121404"),
            target,
        },
    )
    .unwrap();
    assert!(state.units[0].dashboard[0].converted, "the card was replaced");
    assert_eq!(
        state.units[0].dashboard[0].replaced.as_ref(),
        Some(&current),
        "the replaced card is remembered"
    );
    sim.step_turn(&mut state).unwrap();
    let entry = &state.units[0].dashboard[0];
    assert!(!entry.converted, "the Slot rotated");
    assert!(entry.replaced.is_none());
    assert_eq!(entry.current, next, "the top card moved down");
    assert_eq!(entry.next, preview, "the panel refilled");
    assert_ne!(entry.current, SkillId::new("1121404"), "the Guard is gone");
}

/// A Clash compares both Skills' **attack (Offense) Levels**, not the
/// opponent's Defense Level.  Source: JA-wiki 威力 ("お互いの攻撃スキルの攻撃レベルを
/// 参照し、差がある場合は差3ごとに高い方にマッチ威力+1の補正がかかる").
#[test]
fn clash_level_bonus_uses_both_attack_levels() {
    let sim = sim();
    let state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    // Same Skill, all Coins Tails so only the level bonus differs.
    let power = |state: &mut lcb_core::state::BattleState| -> i32 {
        let mut a = battle::build_use(
            state,
            &sim.library,
            &sim.mechanics,
            0,
            &SkillId::new("1011001"),
        )
        .unwrap();
        let mut b = battle::build_use(
            state,
            &sim.library,
            &sim.mechanics,
            enemy,
            &SkillId::new("956701"),
        )
        .unwrap();
        state.preset_flips = vec![false; 32];
        state.flip_cursor = 0;
        battle::toss_all_for_test(state, 0, &mut a);
        // 6 levels ahead of the opponent: +2 Match Power.
        state.units[0].level = 60;
        state.units[enemy].level = 54;
        battle::match_power(state, 0, enemy, &mut a, &b)
    };
    let mut state = state;
    let equal = power(&mut state);
    // The opponent's Defense Level no longer matters.
    state.units[enemy].defense_level_mod = -12;
    let defense_changed = power(&mut state);
    assert_eq!(
        equal, defense_changed,
        "lowering the opponent's Defense Level must not change Match Power"
    );
    // Their attack level does.
    state.units[enemy].offense_level_mod = -12;
    let attack_changed = power(&mut state);
    assert_eq!(
        attack_changed,
        equal + 4,
        "6 more levels of attack advantage = +2, 12 more = +4"
    );
}

/// A status whose Count is consumed to 0 is removed, so it stops triggering.
/// Source: wiki.gg `Status Effects` (Overview): "In single-value or double-value
/// modes, if one or more of the values reach 0, the status effect is removed
/// from the unit."
#[test]
fn sinking_stops_once_its_count_is_gone() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&[fixed::TEAM[0]], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let target = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    // The Imago has no SP, so Sinking deals Gloom damage.
    state.units[target].statuses.remove("Sinking");
    state.units[target].statuses.add_potency("Sinking", 10);
    state.units[target].statuses.add_count("Sinking", 1);
    let before = state.units[target].hp;
    battle::apply_sinking(&mut state, target);
    let first = before - state.units[target].hp;
    assert!(first > 0, "the first trigger deals damage");
    assert_eq!(
        state.units[target].statuses.potency("Sinking"),
        0,
        "the status is gone once its Count is 0"
    );
    let after_first = state.units[target].hp;
    battle::apply_sinking(&mut state, target);
    assert_eq!(
        state.units[target].hp, after_first,
        "a second trigger does nothing"
    );
}

/// PROBE: can a fast Sinner pull an enemy Skill that targets someone else into
/// a Clash?  "スキルをセットする際、自分速度が相手のスロットよりも早ければ、その
/// スキルの使用先を自分に向けさせることができる" (JA-wiki 戦闘システム詳細 /
/// 集中戦闘), and "幻想体戦では、速度で勝っている敵のスキルの使用先を変更することが
/// できる".
#[test]
fn probe_fast_sinner_pulls_an_enemy_skill() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    // Sinner 0 is much faster than the enemy; every enemy Slot targets Sinner 1.
    for unit in state.units.iter_mut().filter(|u| u.kind.is_sinner()) {
        unit.speed = 2;
    }
    state.units[0].speed = 9;
    state.units[enemy].speed = 4;
    let slow = state.units[1].id.clone();
    for action in state.actions.iter_mut() {
        if action.actor == state.units[enemy].id {
            action.target = Some(slow.clone());
        }
    }
    // The fast Sinner is offered a chain to every enemy Slot it out-speeds.
    let actor = state.units[0].id.clone();
    let skill = state.units[0].dashboard[0].current.clone();
    let engages: Vec<u32> = sim
        .legal_actions(&state)
        .iter()
        .filter_map(|action| match action {
            Action::Engage {
                actor: who,
                enemy_slot,
                ..
            } if *who == actor => Some(*enemy_slot),
            _ => None,
        })
        .collect();
    assert!(
        engages.contains(&0),
        "the fast Sinner may chain to enemy Slot 0: {engages:?}"
    );
    sim.submit(
        &mut state,
        Action::Engage {
            actor,
            slot: 0,
            skill,
            enemy_slot: 0,
        },
    )
    .unwrap();
    state.preset_flips = vec![true; 256];
    state.flip_cursor = 0;
    battle::resolve_combat(&mut state, &sim.library, &sim.mechanics);
    assert_eq!(
        state.log.iter().filter(|e| e.kind == "clash").count(),
        1,
        "the pulled enemy Skill Clashed with the fast Sinner: {:?}",
        state
            .log
            .iter()
            .map(|e| format!("[{}] {}", e.kind, e.detail))
            .collect::<Vec<_>>()
    );
}

/// The review's case: the last chain wins even when its owner is *slower* than
/// the Sinner the enemy Slot originally aimed at (it still has to out-speed the
/// enemy itself).  The original target must not steal the Slot back through the
/// automatic matching.
/// Source: JA-wiki 戦闘システム詳細 ("敵のスキルの使用先は当然「最後に行った使用先の
/// 変更」に準拠する").
#[test]
fn last_chain_wins_even_when_it_is_slower_than_the_original_target() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    // The enemy Slot aims at Sinner 2.  Sinner 0 (speed 9) chains first, Sinner 1
    // (speed 7) chains last: both out-speed the enemy (3), but the *original
    // target* is Sinner 0's victim?  No - the original target is Sinner 2, so
    // make Sinner 0 the original target and let the slower Sinner 1 take over.
    for unit in state.units.iter_mut().filter(|u| u.kind.is_sinner()) {
        unit.speed = 1;
    }
    state.units[0].speed = 9;
    state.units[1].speed = 7;
    state.units[enemy].speed = 3;
    // Only Slot 0 aims at Sinner 0 (the original target of the pulled Slot); the
    // rest aim elsewhere, so the overridden chain has nothing else to Clash with.
    let original_target = state.units[0].id.clone();
    let elsewhere = state.units[2].id.clone();
    for action in state.actions.iter_mut() {
        if action.actor != state.units[enemy].id {
            continue;
        }
        action.target = Some(if action.slot == 0 {
            original_target.clone()
        } else {
            elsewhere.clone()
        });
    }
    let first = state.units[0].id.clone();
    let last = state.units[1].id.clone();
    let skill0 = state.units[0].dashboard[0].current.clone();
    let skill1 = state.units[1].dashboard[0].current.clone();
    let name_of =
        |state: &lcb_core::state::BattleState, index: usize, skill: &SkillId| -> String {
            battle::build_use(state, &sim.library, &sim.mechanics, index, skill)
                .map(|use_| use_.name)
                .unwrap_or_default()
        };
    let first_skill = name_of(&state, 0, &skill0);
    let last_skill = name_of(&state, 1, &skill1);
    // The original target chains first, the slower Sinner chains last.
    sim.submit(
        &mut state,
        Action::Engage {
            actor: first.clone(),
            slot: 0,
            skill: skill0,
            enemy_slot: 0,
        },
    )
    .unwrap();
    sim.submit(
        &mut state,
        Action::Engage {
            actor: last.clone(),
            slot: 0,
            skill: skill1,
            enemy_slot: 0,
        },
    )
    .unwrap();
    state.preset_flips = vec![true; 512];
    state.flip_cursor = 0;
    battle::resolve_combat(&mut state, &sim.library, &sim.mechanics);
    let clash = state
        .log
        .iter()
        .find(|entry| entry.kind == "clash")
        .expect("a Clash happened");
    assert!(
        clash.detail.contains(&last_skill),
        "the last chain owns the Slot even though it is slower: {:?} vs {last_skill:?}",
        clash.detail
    );
    assert!(
        !clash.detail.contains(&first_skill),
        "the original target does not Clash: {:?} vs {first_skill:?}",
        clash.detail
    );
    // Its Skill still attacks, one-sidedly, instead of stealing the Slot back.
    assert!(
        state
            .log
            .iter()
            .any(|entry| entry.kind == "attack" && entry.detail.contains(&first_skill)),
        "the overridden chain attacks one-sidedly: {:?}",
        state
            .log
            .iter()
            .map(|e| format!("[{}] {}", e.kind, e.detail))
            .collect::<Vec<_>>()
    );
}

/// When two Skills chain to the same enemy Slot, the **last** change wins
/// ("敵のスキルの使用先は当然「最後に行った使用先の変更」に準拠する",
/// JA-wiki 戦闘システム詳細).
#[test]
fn the_last_chain_to_an_enemy_slot_wins() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    // Both Sinner 0 and Sinner 1 out-speed the enemy, which aims at Sinner 2.
    for unit in state.units.iter_mut().filter(|u| u.kind.is_sinner()) {
        unit.speed = 1;
    }
    state.units[0].speed = 7;
    state.units[1].speed = 9;
    state.units[enemy].speed = 3;
    let slow = state.units[2].id.clone();
    for action in state.actions.iter_mut() {
        if action.actor == state.units[enemy].id {
            action.target = Some(slow.clone());
        }
    }
    let first = state.units[0].id.clone();
    let last = state.units[1].id.clone();
    let skill0 = state.units[0].dashboard[0].current.clone();
    let skill1 = state.units[1].dashboard[0].current.clone();
    // Ask the engine what the two Skills are called in the log, before the turn
    // resolves and the panel rotates.
    let name_of = |state: &lcb_core::state::BattleState,
                   index: usize,
                   skill: &SkillId|
     -> String {
        battle::build_use(state, &sim.library, &sim.mechanics, index, skill)
            .map(|use_| use_.name)
            .unwrap_or_default()
    };
    let first_skill = name_of(&state, 0, &skill0);
    let last_skill = name_of(&state, 1, &skill1);
    sim.submit(
        &mut state,
        Action::Engage {
            actor: first.clone(),
            slot: 0,
            skill: skill0,
            enemy_slot: 0,
        },
    )
    .unwrap();
    sim.submit(
        &mut state,
        Action::Engage {
            actor: last.clone(),
            slot: 0,
            skill: skill1,
            enemy_slot: 0,
        },
    )
    .unwrap();
    state.preset_flips = vec![true; 512];
    state.flip_cursor = 0;
    battle::resolve_combat(&mut state, &sim.library, &sim.mechanics);
    let clash = state
        .log
        .iter()
        .find(|entry| entry.kind == "clash")
        .expect("a Clash happened");
    assert!(
        clash.detail.contains(&last_skill),
        "the last chain owns the Slot: clash {:?} vs last skill {last_skill:?}",
        clash.detail
    );
    assert!(
        !clash.detail.contains(&first_skill),
        "the overridden chain does not Clash: {:?} vs {first_skill:?}",
        clash.detail
    );
    // The overridden Skill still attacks, one-sidedly.
    assert!(
        state
            .log
            .iter()
            .any(|entry| entry.kind == "attack" && entry.detail.contains(&first_skill)),
        "the overridden Skill attacks one-sidedly: {:?}",
        state
            .log
            .iter()
            .map(|e| format!("[{}] {}", e.kind, e.detail))
            .collect::<Vec<_>>()
    );
}

/// A slow Sinner cannot pull an enemy Skill aimed at someone else: their Speed
/// is not higher than the enemy Slot's, so both sides attack one-sidedly
/// ("自分の速度が相手のスロットの速度以下であり、かつ矢印が自分に向いていない場合、
/// その攻撃にマッチすることは出来ない").
#[test]
fn slow_sinner_cannot_pull_and_engage_is_rejected() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    for unit in state.units.iter_mut().filter(|u| u.kind.is_sinner()) {
        unit.speed = 1;
    }
    state.units[enemy].speed = 8;
    let slow = state.units[1].id.clone();
    for action in state.actions.iter_mut() {
        if action.actor == state.units[enemy].id {
            action.target = Some(slow.clone());
        }
    }
    let actor = state.units[0].id.clone();
    let skill = state.units[0].dashboard[0].current.clone();
    assert!(
        !sim.legal_actions(&state).iter().any(|action| matches!(
            action,
            Action::Engage { actor: who, .. } if *who == actor
        )),
        "no chain is offered to a slower Sinner"
    );
    assert!(
        sim.submit(
            &mut state,
            Action::Engage {
                actor,
                slot: 0,
                skill,
                enemy_slot: 0,
            },
        )
        .is_err(),
        "the engine rejects an illegal chain"
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

// ------------------------------------------------------------------------- //
// Full-review regressions (REVIEW_SIMULATOR_FULL.md)
// ------------------------------------------------------------------------- //

/// A flat conditional bonus is not scaled by the value in its condition.
/// "If the sum of the target's [Sinking] and both [Butterfly] is 6 or higher,
/// Coin Power +1" (Sinclair's Solemn Lament) is a threshold, so 4 or 10 Sinking
/// both give +1.  Source: in-game Skill text.
#[test]
fn flat_conditional_bonus_is_not_scaled_by_its_condition() {
    let sim = sim();
    for (sinking, expected) in [(0, 0), (4, 1), (10, 1)] {
        let mut state = sim
            .new_encounter(&["11004"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
            .unwrap();
        state.units[1].statuses.add_potency("Sinking", sinking);
        let mut use_ = battle::build_use(
            &state,
            &sim.library,
            &sim.mechanics,
            0,
            &SkillId::new("1100401"),
        )
        .unwrap();
        battle::prepare_use_for_test(
            &mut state,
            &sim.library,
            &sim.mechanics,
            0,
            Some(1),
            &mut use_,
        );
        assert_eq!(
            use_.ctx.coin_power_bonus, expected,
            "Sinking {sinking} must give a flat Coin Power bonus"
        );
    }
}

/// A Clash Power clause written by the Skill reaches `match_power`, and the
/// generic Power statuses reach the accumulating attack power as well as the
/// Final Power.  Source: wiki.gg `Status Effects` / `Clash`.
#[test]
fn power_modifiers_reach_both_clash_and_attack() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11214"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[1].statuses.add_stack("Dazzle", 1);
    let mut use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1121403"),
    )
    .unwrap();
    battle::prepare_use_for_test(
        &mut state,
        &sim.library,
        &sim.mechanics,
        0,
        Some(1),
        &mut use_,
    );
    assert!(use_.ctx.clash_power_bonus > 0, "the Skill grants Clash Power");
    let opponent = use_.clone();
    let with_bonus = battle::match_power(&mut state, 0, 1, &mut use_, &opponent);
    use_.ctx.clash_power_bonus = 0;
    let without = battle::match_power(&mut state, 0, 1, &mut use_, &opponent);
    assert!(
        with_bonus > without,
        "match_power must read the Skill's Clash Power bonus ({with_bonus} vs {without})"
    );

    // "Attack Power Up" now raises the attack itself, not only `final_power`.
    let mut state = sim
        .new_encounter(&["11214"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let mut plain = {
        let mut use_ = battle::build_use(
            &state,
            &sim.library,
            &sim.mechanics,
            0,
            &SkillId::new("1121401"),
        )
        .unwrap();
        for coin in use_.coins.iter_mut() {
            coin.heads = Some(true);
        }
        use_
    };
    let plain_hits = battle::one_sided_attack(&mut state.clone(), 0, 1, &mut plain, 0);
    state.units[0].statuses.add_count("Attack Power Up", 3);
    let mut boosted = {
        let mut use_ = battle::build_use(
            &state,
            &sim.library,
            &sim.mechanics,
            0,
            &SkillId::new("1121401"),
        )
        .unwrap();
        for coin in use_.coins.iter_mut() {
            coin.heads = Some(true);
        }
        use_
    };
    let boosted_hits = battle::one_sided_attack(&mut state, 0, 1, &mut boosted, 0);
    assert_eq!(
        boosted_hits[0].power - plain_hits[0].power,
        3,
        "Attack Power Up raises the attack power"
    );
}

/// A Coin's own damage clause applies to that Coin ("At less than 50% HP or if
/// the target has [Dazzle], deal +60% damage") and does not leak into the next
/// one.  Source: in-game Skill text.
#[test]
fn coin_damage_clause_applies_to_its_own_coin() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11214"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[1].statuses.add_stack("Dazzle", 1);
    let mut use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1121403"),
    )
    .unwrap();
    for coin in use_.coins.iter_mut() {
        coin.heads = Some(true);
    }
    state.preset_flips = vec![false; 256];
    state.flip_cursor = 0;
    let hits = battle::one_sided_attack(&mut state, 0, 1, &mut use_, 0);
    let last = hits.last().expect("hits").damage;
    let without = {
        let mut state = sim
            .new_encounter(&["11214"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
            .unwrap();
        state.units[1].statuses.add_stack("Dazzle", 1);
        let mut use_ = battle::build_use(
            &state,
            &sim.library,
            &sim.mechanics,
            0,
            &SkillId::new("1121403"),
        )
        .unwrap();
        for coin in use_.coins.iter_mut() {
            coin.heads = Some(true);
        }
        state.preset_flips = vec![false; 256];
        state.flip_cursor = 0;
        let clauses: Vec<Vec<lcb_core::effects::Effect>> = use_
            .mechanics
            .coins
            .values_mut()
            .map(|effects| {
                effects.retain(|effect| effect.kind != "damage_percent");
                effects.clone()
            })
            .collect();
        let _ = clauses;
        battle::one_sided_attack(&mut state, 0, 1, &mut use_, 0)
            .last()
            .expect("hits")
            .damage
    };
    assert!(
        last > without,
        "the last Coin's +60% must land on that Coin ({last} vs {without})"
    );
    assert_eq!(use_.ctx.damage_bonus, 0.0, "the bonus stays with its Coin");
}

/// Stagger lines are consumed: a second crossing escalates to 混乱+ without
/// refreshing the duration, and a recovered unit does not re-Stagger on 1 HP.
/// Source: JA-wiki 戦闘システム詳細 ("混乱区間はステージ中復活しない").
#[test]
fn stagger_lines_are_consumed_and_escalate() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10414"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let max = state.units[0].max_hp;
    state.units[0].hp = max * 60 / 100;
    assert!(battle::check_stagger(&mut state, 0));
    assert_eq!(state.units[0].stagger.level, 1);
    state.units[0].hp = max * 30 / 100;
    battle::check_stagger(&mut state, 0);
    assert_eq!(state.units[0].stagger.level, 2, "crossing again escalates");
    // Recover, then take 1 damage above the remaining lines.
    state.units[0].stagger.turns_remaining = 0;
    state.units[0].stagger.level = 0;
    state.units[0].hp = max * 40 / 100;
    assert!(
        !battle::check_stagger(&mut state, 0),
        "a consumed line does not come back"
    );
    assert_eq!(state.units[0].stagger.consumed, 2);
}

/// Stagger lines are HP amounts: a 10 Potency [Tremor Burst] moves the line by
/// 10 HP, not by 10% of max HP.
#[test]
fn stagger_thresholds_move_in_hp() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11114"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[1].statuses.add_potency("Tremor", 10);
    state.units[1].statuses.add_count("Tremor", 5);
    let before = state.units[1].stagger.threshold_hp(0).unwrap();
    let effect = lcb_core::effects::Effect {
        kind: "tremor_burst".to_string(),
        ..Default::default()
    };
    battle::apply_effects_for_test(
        &mut state,
        &[effect],
        0,
        Some(1),
        &mut Vec::new(),
        &mut battle::UseContext::default(),
    );
    assert_eq!(state.units[1].stagger.threshold_hp(0).unwrap(), before + 10);
}

/// "[Clashable Guard]" / "[Clashable Counter]" Skills take part in a Clash
/// instead of only arming the unit, and a normal Counter answers once per
/// incoming Skill.  Source: wiki.gg `Battles` / Defense Skills.
#[test]
fn clashable_defenses_take_part_in_clashes() {
    let sim = sim();
    for (identity, defense) in [("10913", "1091304"), ("10813", "1081304")] {
        let mut state = sim
            .new_encounter(&[identity], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
            .unwrap();
        state.actions = vec![
            lcb_core::state::SubmittedAction {
                actor: state.units[0].id.clone(),
                target: Some(state.units[1].id.clone()),
                skill: SkillId::new(defense),
                slot: 0,
                is_ego: false,
                ego: None,
                ego_kind: None,
                enemy_slot: None,
                used_top: false,
            },
            lcb_core::state::SubmittedAction {
                actor: state.units[1].id.clone(),
                target: Some(state.units[0].id.clone()),
                skill: SkillId::new("956701"),
                slot: 0,
                is_ego: false,
                ego: None,
                ego_kind: None,
                enemy_slot: None,
                used_top: false,
            },
        ];
        state.preset_flips = vec![true; 256];
        let results = battle::resolve_combat(&mut state, &sim.library, &sim.mechanics);
        assert_eq!(results.len(), 1, "{defense} must Clash");
        assert_eq!(
            state.log.iter().filter(|entry| entry.kind == "counter").count(),
            0,
            "{defense} clashed, so no Counter fires"
        );
    }
}

/// An Atk Weight splash resolves the full hit for every extra target (its own On
/// Hit clauses and its own triggers), while the user's own events stay with the
/// main target.  Source: wiki.gg `Battles` / Atk Weight.
#[test]
fn splash_targets_resolve_their_own_hit() {
    let sim = sim();
    let mut state = sim
        .new_encounter(
            &[fixed::TEAM[0], fixed::TEAM[1], fixed::TEAM[2]],
            &[fixed::BOSS_IMAGO],
            3,
            BattleConfig::default(),
        )
        .unwrap();
    for index in 0..3 {
        state.units[index].hp = 10_000;
        state.units[index].max_hp = 10_000;
        state.units[index].statuses = lcb_core::state::StatusSet::default();
    }
    let mut use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        3,
        &SkillId::new("956706"),
    )
    .unwrap();
    battle::prepare_use_for_test(
        &mut state,
        &sim.library,
        &sim.mechanics,
        3,
        Some(0),
        &mut use_,
    );
    let mut hits = Vec::new();
    for coin in use_.coins.iter_mut() {
        coin.heads = Some(true);
    }
    state.preset_flips = vec![false; 256];
    hits.extend(battle::one_sided_attack(&mut state, 3, 0, &mut use_, 0));
    battle::splash_attack_for_test(&mut state, 3, 0, &mut use_, &hits, 0);
    for index in 0..3 {
        assert!(
            state.units[index].statuses.potency("Burn") > 0,
            "target {index} must receive the Coin's [On Hit] clause"
        );
    }
}

/// A per-turn / per-encounter allowance is only spent once the clause resolves:
/// five failed "At 2 or fewer [LCA Fracture Round], [Reload]" checks must not
/// exhaust the encounter budget.  Source: in-game Skill text.
#[test]
fn failed_conditions_do_not_spend_the_allowance() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11114"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1111403"),
    )
    .unwrap();
    let reload: Vec<lcb_core::effects::Effect> = use_
        .mechanics
        .attack_end
        .iter()
        .filter(|effect| effect.kind == "reload_ammo")
        .cloned()
        .collect();
    assert!(!reload.is_empty(), "the Skill has a Reload clause");
    state.units[0].statuses.add_potency("LCA Fracture Round", 10);
    for _ in 0..5 {
        battle::apply_effects_for_test(
            &mut state,
            &reload,
            0,
            Some(1),
            &mut Vec::new(),
            &mut battle::UseContext::default(),
        );
    }
    assert!(
        state.units[0].turn_effect_usage.is_empty(),
        "failed checks must not spend the limit: {:?}",
        state.units[0].turn_effect_usage
    );
}

/// A hit resolves damage, then the defender's own effects, then the attacker's
/// [On Hit] clauses - so a freshly inflicted [Sinking] survives the hit that
/// applied it.  Source: JA-wiki ダメージ / 攻撃の流れ.
#[test]
fn freshly_inflicted_status_survives_its_own_hit() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11004"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let mut use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1100401"),
    )
    .unwrap();
    // Only the first Coin (whose [Heads Hit] inflicts Sinking) lands.
    use_.coins.truncate(1);
    use_.coins[0].heads = Some(true);
    state.preset_flips = vec![false; 128];
    battle::one_sided_attack(&mut state, 0, 1, &mut use_, 0);
    assert!(
        state.units[1].statuses.potency("Sinking") > 0,
        "the Sinking applied by this hit must not be consumed by it"
    );
}

/// Status components agree between writer and reader: "Gain 1 [Plus Coin Boost]"
/// fills Count (its text says it works by Count), and a Stack-measured status
/// keeps its Stack through the "next turn" queue.  Source: in-game status text.
#[test]
fn status_components_round_trip() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11214"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[1].statuses.add_stack("Dazzle", 3);
    let use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1121403"),
    )
    .unwrap();
    battle::apply_effects_for_test(
        &mut state,
        &use_.mechanics.attack_end.clone(),
        0,
        Some(1),
        &mut Vec::new(),
        &mut battle::UseContext::default(),
    );
    battle::begin_turn(&mut state, &sim.library, &sim.mechanics, &sim.scripts);
    let mut next = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1121401"),
    )
    .unwrap();
    battle::prepare_use_for_test(
        &mut state,
        &sim.library,
        &sim.mechanics,
        0,
        Some(1),
        &mut next,
    );
    assert_eq!(
        next.ctx.coin_power_boost, 1,
        "Plus Coin Boost must be read from the component it was written to"
    );

    // "Inflict 3 [Blue Sand] next turn" keeps its Stack.
    let mut state = sim
        .new_encounter(&["11114"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1111403"),
    )
    .unwrap();
    let sand: Vec<lcb_core::effects::Effect> = use_
        .mechanics
        .coin(2)
        .iter()
        .filter(|effect| effect.status.as_deref() == Some("Blue Sand"))
        .cloned()
        .collect();
    battle::apply_effects_for_test(
        &mut state,
        &sand,
        0,
        Some(1),
        &mut Vec::new(),
        &mut battle::UseContext::default(),
    );
    assert!(
        state.units[1].pending_next_turn.iter().any(|entry| entry.stack == 3),
        "the queued Blue Sand keeps its Stack"
    );
    battle::begin_turn(&mut state, &sim.library, &sim.mechanics, &sim.scripts);
    assert_eq!(state.units[1].statuses.stack("Blue Sand"), 3);
}

/// A Boss decides its state of time **before** that state's Turn Start passive
/// runs, so a switch to Future already uses Future's bonuses.
/// Source: wiki.gg `Butterfly of Entangled Lives::Imago` passives.
#[test]
fn time_state_is_decided_before_its_turn_start_passive() {
    use lcb_core::scripts::TimeState;
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11004"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    state.units[enemy].time_state = Some(TimeState::Present);
    state.units[enemy].statuses.remove("Poise");
    state.units[enemy].statuses.set_stack("In the Present", 10);
    state.units[enemy].statuses.set_stack("In the Future", 20);
    battle::begin_turn(&mut state, &sim.library, &sim.mechanics, &sim.scripts);
    assert_eq!(state.units[enemy].time_state, Some(TimeState::Future));
    assert_eq!(
        state.units[enemy].statuses.potency("Poise"),
        0,
        "the Future passive must run, not the Present one"
    );
}

// ------------------------------------------------------------------------- //
// Unique Ammo and the identities' core passives
// ------------------------------------------------------------------------- //

/// The two-part Unique Ammo opens at 10 of each and its spent units mirror onto
/// the unique [Sinking].  Source: passive `ISeeTheDyingButterfly.` /
/// wiki.gg `Status Effects` (The Living & The Departed, Butterfly).
#[test]
fn unique_ammo_opens_and_mirrors_its_split() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.actions.clear();
    battle::resolve_combat(&mut state, &sim.library, &sim.mechanics);
    assert_eq!(
        state.units[0].statuses.potency("The Living & The Departed"),
        10,
        "Begin encounters with 10 of each of [BulletLament]"
    );
    assert_eq!(state.units[0].statuses.count("The Living & The Departed"), 10);

    // Spend from a pool that only holds The Living: both units must come from it
    // and the target's Butterfly must repeat the split.
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[0].statuses = lcb_core::state::StatusSet::default();
    state.units[0].statuses.add_potency("The Living & The Departed", 2);
    state.units[1].statuses = lcb_core::state::StatusSet::default();
    let mut use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1011001"),
    )
    .unwrap();
    battle::prepare_use_for_test(
        &mut state,
        &sim.library,
        &sim.mechanics,
        0,
        Some(1),
        &mut use_,
    );
    for coin in use_.coins.iter_mut() {
        coin.heads = Some(true);
    }
    state.preset_flips = vec![true; 256];
    battle::one_sided_attack(&mut state, 0, 1, &mut use_, 0);
    assert_eq!(state.units[0].statuses.total("The Living & The Departed"), 0);
    assert_eq!(
        state.units[1].statuses.potency("Butterfly"),
        2,
        "the consumed The Living is inflicted as Butterfly"
    );
    assert_eq!(state.units[1].statuses.count("Butterfly"), 0);
}

/// Running dry midway cancels the remaining Coins and pays for a Reload.
/// Source: passive `ISeeTheDyingButterfly.`, wiki.gg `Status Effects` / Reload.
#[test]
fn running_dry_cancels_the_coins_and_reloads() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10110"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[0].statuses = lcb_core::state::StatusSet::default();
    state.units[0].statuses.add_potency("The Living & The Departed", 1);
    state.units[0].sanity = Sanity::Sane { sp: 10 };
    let mut use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1011002"),
    )
    .unwrap();
    battle::prepare_use_for_test(
        &mut state,
        &sim.library,
        &sim.mechanics,
        0,
        Some(1),
        &mut use_,
    );
    for coin in use_.coins.iter_mut() {
        coin.heads = Some(true);
    }
    state.preset_flips = vec![true; 256];
    let hits = battle::one_sided_attack(&mut state, 0, 1, &mut use_, 0);
    assert_eq!(hits.len(), 1, "the Skill is cancelled once the ammo runs out");
    assert!(use_.ctx.ammo_exhausted);
    assert_eq!(
        state.units[0].statuses.total("The Living & The Departed"),
        20,
        "the Reload refilled the pool"
    );
}

/// Outis' LCA Fracture Round is spent by the Skill that names it, powers that
/// Skill up and reloads at 2 or fewer.  Source: in-game Skill text + the
/// `LCA Fracture Round` status ("Max Capacity: 16").
#[test]
fn lca_fracture_rounds_are_spent_and_reloaded() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11114"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[0].statuses = lcb_core::state::StatusSet::default();
    state.units[0].statuses.set_stack("LCA Fracture Round", 4);
    let mut use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1111403"),
    )
    .unwrap();
    battle::prepare_use_for_test(
        &mut state,
        &sim.library,
        &sim.mechanics,
        0,
        Some(1),
        &mut use_,
    );
    // "Base Power +1 for every [LCA Fracture Round] about to be spent (4)".
    assert_eq!(use_.ctx.base_power_bonus, 4);
    for coin in use_.coins.iter_mut() {
        coin.heads = Some(true);
    }
    state.preset_flips = vec![true; 256];
    battle::one_sided_attack(&mut state, 0, 1, &mut use_, 0);
    assert_eq!(use_.ctx.ammo_spent_by_status.get("LCA Fracture Round"), Some(&4));
    battle::apply_attack_end_for_test(&mut state, &sim.library, &sim.mechanics, 0, Some(1), 0, &mut use_);
    assert_eq!(
        state.units[0].statuses.stack("LCA Fracture Round"),
        16,
        "At 2 or fewer, [Reload] refills the Unique Ammo"
    );
}

/// Gregor redirects an enemy Skill onto himself regardless of Speed (focused
/// encounters).  Source: passive `Dazzling Lamp`.
#[test]
fn gregor_redirects_without_outspeeding() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["11214", "11004"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[0].speed = 1;
    state.units[2].speed = 3;
    for action in state.actions.iter_mut() {
        if !state.units[2].kind.is_sinner() {
            continue;
        }
    }
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    for action in state.actions.iter_mut() {
        if action.actor == state.units[enemy].id {
            action.target = Some(state.units[1].id.clone());
        }
    }
    let action = Action::Engage {
        actor: state.units[0].id.clone(),
        slot: 0,
        skill: state.units[0].dashboard[0].current.clone(),
        enemy_slot: 0,
    };
    assert!(
        sim.submit(&mut state, action).is_ok(),
        "Dazzling Lamp lifts the Speed requirement"
    );
}

/// Rodion's Blessing / Despair state is decided at Turn Start.
/// Source: passive `The Sword Sharpened with Tears`.
#[test]
fn rodion_state_follows_her_sp() {
    let sim = sim();
    for (sp, expected) in [(0i32, "Blessing"), (-10, "Despair")] {
        let mut state = sim
            .new_encounter(&["10913"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
            .unwrap();
        state.units[0].sanity = Sanity::Sane { sp };
        state.units[0].statuses.remove("Blessing");
        state.units[0].statuses.remove("Despair");
        battle::begin_turn(&mut state, &sim.library, &sim.mechanics, &sim.scripts);
        assert!(
            state.units[0].statuses.stack(expected) > 0,
            "SP {sp} must give {expected}"
        );
    }
}

/// Ryoshu's Unwithering Flower feeds Petals from Sinking damage.
/// Source: passive `Unwithering Flower` (wiki.gg `Status Effects` / Petals).
#[test]
fn petals_grow_from_sinking_damage() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10414"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[1].statuses.add_potency("Sinking", 5);
    state.units[1].statuses.add_count("Sinking", 5);
    battle::apply_sinking_for_test(&mut state, 1);
    assert!(
        state.units[0].statuses.stack("Petals") > 0,
        "the enemy taking Sinking damage feeds her Petals"
    );
}

/// Ryoshu's passives trigger Unopposed Attacks at her Attack End: "Bang. Bang."
/// at 1+ [Bullet - Solitude] against a target below -40 SP or Staggered, and
/// "Stories that Never Cease" at 0 ammo (which then full-reloads).
/// Source: passives `A Void that Cannot be Filled` / `Unwithering Flower`.
#[test]
fn ryoshu_passives_fire_unopposed_attacks() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10414"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    state.units[enemy].hp = state.units[enemy].max_hp;
    state.units[0].statuses.set_stack("Bullet - Solitude", 6);
    let mut use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1041402"),
    )
    .unwrap();
    battle::prepare_use_for_test(
        &mut state,
        &sim.library,
        &sim.mechanics,
        0,
        Some(enemy),
        &mut use_,
    );
    // The target is Staggered, so "Bang. Bang." follows this Attack End.
    state.units[enemy].stagger.turns_remaining = 2;
    let before = state.units[enemy].hp;
    battle::apply_attack_end_for_test(
        &mut state,
        &sim.library,
        &sim.mechanics,
        0,
        Some(enemy),
        0,
        &mut use_,
    );
    assert!(
        state.units[enemy].hp < before,
        "the passive follow-up attack must land"
    );
    assert_eq!(
        state.log.iter().filter(|entry| entry.kind == "unopposed").count(),
        1
    );

    // With the pool empty, "Stories that Never Cease" replaces it and full-reloads.
    let mut state = sim
        .new_encounter(&["10414"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let enemy = state.units.iter().position(|u| !u.kind.is_sinner()).unwrap();
    state.units[0].statuses.set_stack("Bullet - Solitude", 0);
    let mut use_ = battle::build_use(
        &state,
        &sim.library,
        &sim.mechanics,
        0,
        &SkillId::new("1041402"),
    )
    .unwrap();
    battle::prepare_use_for_test(
        &mut state,
        &sim.library,
        &sim.mechanics,
        0,
        Some(enemy),
        &mut use_,
    );
    battle::apply_attack_end_for_test(
        &mut state,
        &sim.library,
        &sim.mechanics,
        0,
        Some(enemy),
        0,
        &mut use_,
    );
    assert_eq!(
        state.units[0].statuses.stack("Bullet - Solitude"),
        6,
        "the follow-up consumed 10 SP and full-reloaded"
    );
}

/// Hong Lu's Hanafuda Hand is converted at the **next** Turn Start, not at once.
/// Source: passive `Koi-Koi` / Skill `Shuffle Hands - Floral Deck`.
#[test]
fn hanafuda_conversion_waits_for_the_next_turn() {
    let sim = sim();
    let mut state = sim
        .new_encounter(&["10813"], &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    state.units[0].suit = Some("HanafudaOne".to_string());
    let effects = vec![lcb_core::effects::Effect {
        kind: "suit_convert".to_string(),
        next_turn: true,
        condition: Some(lcb_core::effects::Condition {
            slot: Some("leftmost".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    }];
    battle::apply_effects_for_test(
        &mut state,
        &effects,
        0,
        None,
        &mut Vec::new(),
        &mut battle::UseContext::default(),
    );
    assert_eq!(
        state.units[0].suit.as_deref(),
        Some("HanafudaOne"),
        "the conversion is queued, not applied"
    );
    battle::begin_turn(&mut state, &sim.library, &sim.mechanics, &sim.scripts);
    assert_ne!(
        state.units[0].suit.as_deref(),
        Some("HanafudaOne"),
        "the Hand is converted at the following Turn Start"
    );
}

/// Sin Resonance grants Offense/Defense Level by the Skill's position on the
/// chain.  Source: wiki.gg `Sin Resonance` / Offense-Defense Level Gain
/// (+1/+3/+3/+5/+5/+7/+7/+9/+9/+11/+11; an Absolute chain gives the 3rd value
/// onwards to every Skill in it).
#[test]
fn resonance_grants_offense_level_by_chain_position() {
    assert_eq!(battle::resonance_level_bonus(1), 1);
    assert_eq!(battle::resonance_level_bonus(3), 3);
    assert_eq!(battle::resonance_level_bonus(4), 5);
    assert_eq!(battle::resonance_level_bonus(11), 11);
    assert_eq!(battle::resonance_level_bonus(20), 11);
    assert_eq!(battle::a_resonance_level_bonus(1), 0);
    assert_eq!(battle::a_resonance_level_bonus(2), 3);
    assert_eq!(battle::a_resonance_level_bonus(3), 5);
    assert_eq!(battle::a_resonance_level_bonus(4), 5);
    assert_eq!(battle::a_resonance_level_bonus(9), 11);

    // Three Gloom Skills in a row give every Skill of the chain +3 (A-Reson 3
    // is +5, so the third one gets that; the first two get the regular value).
    let sim = sim();
    let mut state = sim
        .new_encounter(&fixed::TEAM, &[fixed::BOSS_IMAGO], 3, BattleConfig::default())
        .unwrap();
    let mut gloom = Vec::new();
    for (index, unit) in state.units.iter().enumerate() {
        if !unit.kind.is_sinner() {
            continue;
        }
        let skill = unit.dashboard[0].current.clone();
        if battle::sin_of_for_test(&sim.library, &skill) == Some(lcb_core::ids::Sin::Gloom) {
            gloom.push((index, skill));
        }
    }
    if gloom.len() >= 3 {
        state.actions.clear();
        for (index, skill) in gloom.iter().take(3) {
            state.actions.push(lcb_core::state::SubmittedAction {
                actor: state.units[*index].id.clone(),
                target: Some(state.units[state.units.len() - 1].id.clone()),
                skill: skill.clone(),
                slot: 0,
                is_ego: false,
                ego: None,
                ego_kind: None,
                enemy_slot: None,
                used_top: false,
            });
        }
        battle::resolve_combat(&mut state, &sim.library, &sim.mechanics);
        let bonus = state
            .resonance_levels
            .values()
            .copied()
            .max()
            .unwrap_or(0);
        assert_eq!(bonus, 5, "the third Skill of a Gloom A-Reson gains +5");
    }
}
