//! Combat execution: coin flips, clashing, one-sided attacks, status ticks and
//! the turn loop.
//!
//! Every rule implemented here cites the wiki.gg page it comes from in
//! `docs/MECHANICS.md`; rules that could not be sourced are represented by an
//! explicit config knob plus a `UnknownRule` warning instead of a guessed value.

use crate::damage::{compute_damage, level_clash_bonus, DamageInputs};
use crate::effects::{Component, Condition, Effect, MechanicsBook, SkillMechanics};
use crate::ids::{DamageType, EgoId, Sin, SkillId, UnitId, Uptie};
use crate::library::Library;
use crate::state::{
    BattleState, Phase, Sanity, SubmittedAction, Unit, UnitKind, Winner, SP_LIMIT,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Skill ids that mean "no skill": enemy slots that are empty.
pub const EMPTY_SKILL: &str = "";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoinState {
    Fresh,
    Destroyed,
    /// Unbreakable coin that survived a lost clash (wiki.gg `Clash`).
    Cracked,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CoinRuntime {
    pub heads: Option<bool>,
    pub state: CoinState,
    pub unbreakable: bool,
    /// Set when Paralyze fixed this coin's power to 0 for the current toss.
    #[serde(default)]
    pub paralyzed: bool,
}

impl CoinRuntime {
    pub fn fresh(unbreakable: bool) -> Self {
        Self {
            heads: None,
            state: CoinState::Fresh,
            unbreakable,
            paralyzed: false,
        }
    }
}

/// Values computed once per skill use from the skill's `[On Use]` effects.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UseContext {
    /// Affinity of the skill that produced this context.
    #[serde(default)]
    pub sin: Sin,
    pub clash_power_bonus: i32,
    pub coin_power_bonus: i32,
    pub base_power_bonus: i32,
    /// Dynamic damage modifier contributions for this skill use.
    pub damage_bonus: f64,
    pub shield_gain: i32,
    pub ammo_spent: i32,
    /// "The Living & The Departed" is a **two-part** pool: each unit spent is
    /// randomly The Living (Potency) or The Departed (Count), and a Skill that
    /// inflicts the unique [Sinking] mirrors the split it consumed.
    #[serde(default)]
    pub ammo_living: i32,
    #[serde(default)]
    pub ammo_departed: i32,
    /// Ammo spent per pool, for "for every [X] spent by this Skill" clauses.
    #[serde(default)]
    pub ammo_spent_by_status: BTreeMap<String, i32>,
    /// "If this unit runs out of ammo midway through Skill use, cancel all
    /// subsequent Coins and [Reload]".
    #[serde(default)]
    pub ammo_exhausted: bool,
    pub unbreakable_coins: Vec<u32>,
    /// "This Attack Skill deals 0 damage" (The Quickening).
    #[serde(default)]
    pub zero_damage: bool,
    /// "convert all Coins on this Skill to Unbreakable Coins".
    #[serde(default)]
    pub unbreakable_all: bool,
    /// Ammo this skill use will spend ("about to be spent").
    #[serde(default)]
    pub ammo_planned: i32,
    /// Status amount spent by this use ("Stack consumed").
    #[serde(default)]
    pub consumed_status: i32,
    /// Extra critical chance (Poise potency + modifiers).
    #[serde(default)]
    pub crit_chance_bonus: i32,
    /// Extra critical damage multiplier.
    #[serde(default)]
    pub crit_damage_bonus: f64,
    /// "Lower user's Stagger Threshold by N% of damage dealt".
    #[serde(default)]
    pub lower_stagger_percent: i32,
    /// "While Clashing with this Skill, the main target's [X] Count does not
    /// drop below 1" - statuses protected during this use.
    #[serde(default)]
    pub status_count_floor: Vec<String>,
    /// The skill's owner lost its Clash (for "after Clash Lose" clauses).
    #[serde(default)]
    pub lost_clash: bool,
    /// "Target cannot be Staggered until this Skill's Attack End".
    #[serde(default)]
    pub no_stagger_target: bool,
    /// "[Clashable Guard]" / "[Clashable Counter]": the defense Skill may be
    /// matched against an enemy attack instead of only arming the unit.
    #[serde(default)]
    pub clashable_defense: bool,
    /// An E.G.O's SP cost, paid at the [Before Attack] timing.
    #[serde(default)]
    pub ego_sp_pending: Option<i32>,
    /// "Reuse this Skill on the target that has the highest HP" when it kills.
    #[serde(default)]
    pub reuse_on_kill: bool,
    /// "treat the target's resistance as at least X" pairs (kind, value).
    #[serde(default)]
    pub resist_floor: Vec<(String, f64)>,
    /// `Plus Coin Boost` / `Minus Coin Drop` for this use.
    #[serde(default)]
    pub coin_power_boost: i32,
    #[serde(default)]
    pub coin_power_drop: i32,
    pub notes: Vec<String>,
    /// Running skill power during an attack (Base Power + Heads Coin Power).
    #[serde(default)]
    pub accumulated: i32,
    /// "[Before Attack] At 3+ (Gloom Reson.), Atk Weight +1".
    #[serde(default)]
    pub attack_weight_bonus: i32,
    /// "Then, Reuse this Coin (N times per Skill)": coin index -> repeats left.
    #[serde(default)]
    pub reuse_coins: BTreeMap<u32, i32>,
    /// Number of times each Coin has hit ("# of Coin 3 hits").
    #[serde(default)]
    pub coin_hits: BTreeMap<u32, i32>,
    /// "Then, Reuse this Coin (N times per Skill)": Coin -> times it may be used
    /// again in this Skill use.
    #[serde(default)]
    pub reuse_caps: BTreeMap<u32, i32>,
    /// "Reuse this Coin once for every N% missing HP (max K times)".
    #[serde(default)]
    pub reuse_hp: Option<(i32, i32)>,
    /// How many of the missing-HP Reuses were already spent.
    #[serde(default)]
    pub reuse_hp_used: i32,
    /// HP this unit lost to its own Skill ("HP lost due to this effect").
    #[serde(default)]
    pub self_damage_taken: i32,
    /// "[Attack End] If ... targets are killed" bookkeeping.
    #[serde(default)]
    pub kills: i32,
    /// "Deal -N% damage against sub-targets".
    #[serde(default)]
    pub sub_target_damage_percent: i32,
    /// Indiscriminate (random Corrosion): sub-targets may include allies.
    #[serde(default)]
    pub indiscriminate: bool,
    /// "each Coin flips against a random enemy among its targets".
    #[serde(default)]
    pub random_coin_targets: bool,
    /// "(Chance to flip Heads)% chance to inflict The Departed".
    #[serde(default)]
    pub butterfly_split: bool,
    /// How many times a Coin of this use has been Reused so far.
    #[serde(default)]
    pub reuse_uses: i32,
    /// "Take -N% HP damage from attacks" (incoming damage modifier).
    #[serde(default)]
    pub damage_taken_bonus: f64,
    /// "[On Target Kill] ... (once per Skill)" bookkeeping.
    #[serde(default)]
    pub on_kill_done: bool,
    /// Amount of each status this use consumed ("Stack consumed").
    #[serde(default)]
    pub consumed_by_status: BTreeMap<String, i32>,
    /// "Deal Gloom damage equal to (N)% of this Coin's final damage".
    #[serde(default)]
    pub final_damage_percent: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillUse {
    pub actor: UnitId,
    pub target: Option<UnitId>,
    /// Dashboard slot the skill was used from.
    #[serde(default)]
    pub slot: u32,
    pub skill: SkillId,
    pub name: String,
    pub sin: Sin,
    pub damage_type: DamageType,
    pub base_power: i32,
    pub coin_power: i32,
    pub offense_level_mod: i32,
    pub defense_level_mod: Option<i32>,
    pub attack_weight: u32,
    pub coins: Vec<CoinRuntime>,
    pub mechanics: SkillMechanics,
    pub ctx: UseContext,
    pub is_defense: bool,
    pub is_ego: bool,
    pub ego: Option<EgoId>,
}

impl SkillUse {
    pub fn current_coin(&self) -> Option<usize> {
        self.coins.iter().position(|c| c.state == CoinState::Fresh)
    }

    pub fn has_fresh_coins(&self) -> bool {
        self.current_coin().is_some()
    }

    pub fn cracked_coins(&self) -> Vec<usize> {
        self.coins
            .iter()
            .enumerate()
            .filter(|(_, c)| c.state == CoinState::Cracked)
            .map(|(i, _)| i)
            .collect()
    }

    pub fn remaining_coins(&self) -> usize {
        self.coins.iter().filter(|c| c.state != CoinState::Destroyed).count()
    }
}

/// Defense skill families.  Source: wiki.gg `Battles` / Defense Skills.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DefenseKind {
    /// Adds Shield equal to its Final Power for the turn; does not attack.
    Guard,
    /// Flips its Coin against every incoming Coin; equal or higher negates it.
    Evade,
    /// Counterattacks whoever attacks the unit.
    Counter,
}

/// A defense skill that is active for the current turn.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActiveDefense {
    pub unit: UnitId,
    pub kind: DefenseKind,
    pub skill: SkillId,
    pub name: String,
    pub sin: Sin,
    pub damage_type: DamageType,
    pub base_power: i32,
    pub coin_power: i32,
    pub offense_level_mod: i32,
    pub coins: Vec<CoinRuntime>,
    pub mechanics: SkillMechanics,
    pub ctx: UseContext,
    /// Evade is lost after failing once.
    pub lost: bool,
    /// Guards gain their Shield when the unit is first attacked this turn
    /// (JA-wiki ガード: "使用対象としたスキルの攻撃前"; if the unit is never
    /// attacked the Guard does not activate at all).
    pub activated: bool,
    /// Defense level modifier the Guard imposes for the turn.
    pub defense_level_mod: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HitResult {
    pub coin_index: usize,
    pub heads: bool,
    pub power: i32,
    pub damage: i32,
    pub critical: bool,
    pub staggered: bool,
    pub killed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClashResult {
    pub winner: Option<UnitId>,
    pub rounds: i32,
    pub attacker_coins_left: usize,
    pub defender_coins_left: usize,
}

/// Everything a caller may submit for one dashboard slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Assign {
        actor: UnitId,
        slot: u32,
        skill: SkillId,
        target: UnitId,
    },
    /// Chain this Skill to a specific enemy Skill Slot.  Focused encounters
    /// only, and only when the enemy Skill already targets this unit or this
    /// unit's Speed is higher than that Slot's Speed (JA-wiki 戦闘システム詳細 /
    /// 集中戦闘).
    Engage {
        actor: UnitId,
        slot: u32,
        skill: SkillId,
        enemy_slot: u32,
    },
    /// Use an E.G.O skill (wiki.gg `Clash` / E.G.O Skills).
    UseEgo {
        actor: UnitId,
        slot: u32,
        ego: EgoId,
        kind: EgoSkillKind,
        target: UnitId,
    },
    /// The target row of the four-step action space in the plan:
    /// pick unit -> pick skill -> pick target -> commit.
    Commit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EgoSkillKind {
    Awakening,
    Corrosion,
    /// Corrosion without the indiscriminate downside: 1.5x SP and resources,
    /// rounded up (wiki.gg `Clash` / Overclocking).
    Overclock,
}

/// Legal actions for the current phase.  Enumerates unit -> slot -> skill ->
/// target, exactly the action space the plan asks the Python side to see.
pub fn legal_actions(state: &BattleState, library: &Library) -> Vec<Action> {
    if state.phase != Phase::AwaitingActions || state.winner.is_some() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let enemies = state.living_enemies();
    for unit in state.units.iter().filter(|u| u.alive && u.kind.is_sinner()) {
        if unit.is_staggered() {
            continue;
        }
        let identity = match &unit.kind {
            UnitKind::Sinner { identity } => identity,
            _ => continue,
        };
        let Some(record) = library.identity(identity) else { continue };
        // Both clearly visible skills of a slot are selectable ("2 clearly
        // visible skills and 1 faint preview", JA-wiki チェーンパネル).
        let mut skills: Vec<SkillId> = unit
            .dashboard
            .iter()
            .flat_map(|s| [s.current.clone(), s.next.clone()])
            .filter(|s| s.0 != EMPTY_SKILL)
            .collect();
        // Defense skills are always available from the portrait.
        for skill in &record.skills {
            if skill.slot() == Some(crate::ids::SkillSlot::Defense) {
                skills.push(SkillId::new(skill.id.clone()));
            }
        }
        skills.sort();
        skills.dedup();
        // Enemy Skill Slots this unit may chain to: the ones already aimed at it,
        // plus (in a focused encounter) any Slot it out-speeds
        // ("幻想体戦では、速度で勝っている敵のスキルの使用先を変更することができる").
        let pullable: Vec<u32> = if state.config.focused_encounter {
            state
                .actions
                .iter()
                .filter(|a| {
                    !state
                        .index_of(&a.actor)
                        .map(|index| state.units[index].kind.is_sinner())
                        .unwrap_or(false)
                })
                .filter(|a| {
                    let aimed_at_me = a.target.as_ref() == Some(&unit.id);
                    let owner_speed = state
                        .index_of(&a.actor)
                        .map(|index| state.units[index].speed)
                        .unwrap_or(0);
                    aimed_at_me || unit.speed > owner_speed
                })
                .map(|a| a.slot)
                .collect()
        } else {
            Vec::new()
        };
        for slot in unit.dashboard.iter().map(|s| s.slot).collect::<Vec<_>>() {
            for skill in &skills {
                for enemy_slot in &pullable {
                    out.push(Action::Engage {
                        actor: unit.id.clone(),
                        slot,
                        skill: skill.clone(),
                        enemy_slot: *enemy_slot,
                    });
                }
                for target in &enemies {
                    out.push(Action::Assign {
                        actor: unit.id.clone(),
                        slot,
                        skill: skill.clone(),
                        target: target.clone(),
                    });
                }
            }
            // E.G.O skills the identity owns and the team can currently pay for.
            for ego_id in &unit.ego_slots {
                let Some(ego) = library.ego(ego_id) else { continue };
                // The player never picks Corrosion directly (JA-wiki 戦闘システム
                // 詳細: it fires randomly when SP is negative, or through
                // Overclock, or is forced in the E.G.O Corrosion state).
                for kind in [EgoSkillKind::Awakening, EgoSkillKind::Overclock] {
                    if !ego_kind_available(state, ego_id, kind) {
                        continue;
                    }
                    if !ego_affordable(state, unit, ego, kind) {
                        continue;
                    }
                    for target in &enemies {
                        out.push(Action::UseEgo {
                            actor: unit.id.clone(),
                            slot,
                            ego: ego_id.clone(),
                            kind,
                            target: target.clone(),
                        });
                    }
                }
            }
        }
    }
    out.push(Action::Commit);
    out
}

pub fn submit(state: &mut BattleState, action: Action) -> Result<(), String> {
    match action {
        Action::Assign {
            actor,
            slot,
            skill,
            target,
        } => {
            let Some(unit) = state.unit_mut(&actor) else {
                return Err(format!("unknown unit {actor}"));
            };
            if !unit.alive {
                return Err(format!("{actor} is not alive"));
            }
            let Some(entry) = unit.dashboard.iter_mut().find(|s| s.slot == slot) else {
                return Err(format!("{actor} has no slot {slot}"));
            };
            // Both cards drawn on the Slot are legal actions.  Submitting does
            // **not** rewrite the panel: the chosen card is consumed when the
            // turn resolves, and the other one stays where it is (a defense
            // skill or an E.G.O replaces the bottom card instead).
            let used_top = entry.next == skill && entry.current != skill;
            entry.target = Some(target.clone());
            if entry.current != skill && !used_top {
                // A defense Skill or an E.G.O replaces the bottom card; keep the
                // card it replaced so the rotation consumes the right one.
                entry.converted = true;
                if entry.replaced.is_none() {
                    entry.replaced = Some(entry.current.clone());
                }
                entry.current = skill.clone();
            }
            state.actions.retain(|a| !(a.actor == actor && a.slot == slot));
            state.actions.push(SubmittedAction {
                actor,
                slot,
                skill,
                target: Some(target),
                is_ego: false,
                ego: None,
                ego_kind: None,
                enemy_slot: None,
                used_top,
            });
            Ok(())
        }
        Action::Engage {
            actor,
            slot,
            skill,
            enemy_slot,
        } => {
            let Some(unit) = state.unit(&actor) else {
                return Err(format!("unknown unit {actor}"));
            };
            if !unit.alive {
                return Err(format!("{actor} is not alive"));
            }
            // The pull is only legal when this Skill already has that enemy
            // Slot aimed at it, or beats its Speed.
            let Some(enemy) = state.units.iter().find(|u| !u.kind.is_sinner()) else {
                return Err("no enemy".to_string());
            };
            let enemy_index = state.index_of(&enemy.id).unwrap_or(0);
            let aimed_at_me = state.actions.iter().any(|a| {
                a.actor == enemy.id && a.slot == enemy_slot && a.target.as_ref() == Some(&actor)
            });
            // "When chaining Skills, this unit can redirect the Attack Skills'
            // targeting to itself regardless of Speed (Focused Encounters only)"
            // (Gregor's `Dazzling Lamp`) and an enemy Skill marked "Can Clash
            // with this Skill regardless of Speed" can always be chained to.
            let ignores_speed = unit.passive_ids.iter().any(|id| id == "1121402")
                || state.units[enemy_index]
                    .clash_any_speed_slots
                    .contains(&enemy_slot);
            let faster = unit.speed > state.units[enemy_index].speed || ignores_speed;
            if !(aimed_at_me || (state.config.focused_encounter && faster)) {
                return Err(format!(
                    "{actor} cannot chain to enemy Slot {enemy_slot} (Speed {} vs {})",
                    unit.speed, state.units[enemy_index].speed
                ));
            }
            let target = enemy.id.clone();
            let Some(entry) = state
                .unit_mut(&actor)
                .and_then(|unit| unit.dashboard.iter_mut().find(|s| s.slot == slot))
            else {
                return Err(format!("{actor} has no slot {slot}"));
            };
            let used_top = entry.next == skill && entry.current != skill;
            entry.target = Some(target.clone());
            if entry.current != skill && !used_top {
                entry.converted = true;
                if entry.replaced.is_none() {
                    entry.replaced = Some(entry.current.clone());
                }
                entry.current = skill.clone();
            }
            state.actions.retain(|a| !(a.actor == actor && a.slot == slot));
            state.actions.push(SubmittedAction {
                actor,
                slot,
                skill,
                target: Some(target),
                is_ego: false,
                ego: None,
                ego_kind: None,
                enemy_slot: Some(enemy_slot),
                used_top,
            });
            Ok(())
        }
        Action::UseEgo {
            actor,
            slot,
            ego,
            kind,
            target,
        } => {
            let Some(unit) = state.unit(&actor) else {
                return Err(format!("unknown unit {actor}"));
            };
            if !unit.alive {
                return Err(format!("{actor} is not alive"));
            }
            if !unit.ego_slots.contains(&ego) {
                return Err(format!("{actor} does not have E.G.O {ego} equipped"));
            }
            state.actions.retain(|a| !(a.actor == actor && a.slot == slot));
            state.actions.push(SubmittedAction {
                actor,
                slot,
                skill: SkillId::new(ego.as_str().to_string()),
                target: Some(target),
                is_ego: true,
                ego: Some(ego),
                ego_kind: Some(kind),
                enemy_slot: None,
                used_top: false,
            });
            Ok(())
        }
        Action::Commit => {
            state.phase = Phase::Combat;
            Ok(())
        }
    }
}

/// Is this E.G.O skill type present on the record?
fn ego_kind_available(_state: &BattleState, ego: &EgoId, kind: EgoSkillKind) -> bool {
    let _ = (ego, kind);
    true
}

/// Resource + SP check (wiki.gg `Clash` / E.G.O Skills, Overclocking).
fn ego_affordable(state: &BattleState, unit: &Unit, ego: &crate::library::EgoRecord, kind: EgoSkillKind) -> bool {
    let (sp_cost, multiplier) = match kind {
        EgoSkillKind::Awakening => (ego.awakening_sp.unwrap_or(0), 1.0),
        EgoSkillKind::Corrosion => (ego.corrosion_sp.unwrap_or(0), 1.0),
        EgoSkillKind::Overclock => (ego.corrosion_sp.unwrap_or(0), 1.5),
    };
    if sp_cost > 0 || multiplier > 1.0 {
        let cost = if multiplier == 1.0 {
            sp_cost
        } else {
            (sp_cost as f64 * multiplier).ceil() as i32
        };
        match unit.sanity {
            Sanity::None => {}
            Sanity::Sane { sp } => {
                if sp - cost < -SP_LIMIT {
                    return false;
                }
            }
        }
    }
    for (sin, amount) in crate::library::Library::ego_cost(ego) {
        let needed = if multiplier == 1.0 {
            amount
        } else {
            (amount as f64 * multiplier).ceil() as i32
        };
        let have = state.ego_resources.get(&sin).copied().unwrap_or(0);
        if have < needed {
            return false;
        }
    }
    true
}

/// Chance (percent) that an E.G.O use turns into a random Corrosion, based on
/// the SP left after paying for it.  Source: JA-wiki 戦闘システム詳細.
/// "The Living & The Departed" (in-game `BulletLament`).
pub const AMMO_SOLEMN_LAMENT: &str = "The Living & The Departed";
/// "Bullet - Solitude" (in-game `BulletGodok`).
pub const AMMO_SOLITUDE: &str = "Bullet - Solitude";
/// "LCA Fracture Round" (in-game `LCA_Bullet`).
pub const AMMO_FRACTURE: &str = "LCA Fracture Round";

/// Which half of a two-part ammo pool a single unit is drawn from.
///
/// "When [ReloadLament]ing, or when gaining [BulletLament], gain either The
/// Living or The Departed based on this unit's current SP - At 0 or higher SP:
/// 30% chance to gain The Living; 70% chance to gain The Departed.  At less than
/// 0 SP: 70% / 30%.  The probabilities are calculated separately for each ammo"
/// (passive `ISeeTheDyingButterfly.`).  Spending uses the same split ("Whether
/// The Living(Potency) or The Departed(Count) will be consumed for each shot is
/// randomly determined", wiki.gg `Status Effects`).
fn roll_living(state: &mut BattleState, unit_index: usize) -> bool {
    let living_chance = if state.units[unit_index].sanity.sp() >= 0 {
        30
    } else {
        70
    };
    state.flip(living_chance)
}

/// Spend `amount` from one ammo pool.  Returns how much was actually spent.
fn spend_ammo(
    state: &mut BattleState,
    unit_index: usize,
    status: &str,
    amount: i32,
) -> i32 {
    if amount <= 0 {
        return 0;
    }
    if status != AMMO_SOLEMN_LAMENT {
        // Single-value ammo (Bullet - Solitude, LCA Fracture Round): Stack.
        let mut spent = 0;
        for _ in 0..amount {
            if state.units[unit_index].statuses.stack(status) <= 0 {
                break;
            }
            state.units[unit_index].statuses.add_stack(status, -1);
            spent += 1;
        }
        return spent;
    }
    let mut spent = 0;
    let mut living = 0;
    for _ in 0..amount {
        let wants_living = roll_living(state, unit_index);
        let unit = &mut state.units[unit_index];
        let has_living = unit.statuses.potency(AMMO_SOLEMN_LAMENT) > 0;
        let has_departed = unit.statuses.count(AMMO_SOLEMN_LAMENT) > 0;
        let take_living = if wants_living { has_living } else { !has_departed && has_living };
        if take_living {
            unit.statuses.add_potency(AMMO_SOLEMN_LAMENT, -1);
            living += 1;
        } else if has_departed {
            unit.statuses.add_count(AMMO_SOLEMN_LAMENT, -1);
        } else {
            break;
        }
        spent += 1;
    }
    // The split travels back to the caller through the (already applied) pool.
    LAST_AMMO_SPLIT.with(|split| *split.borrow_mut() = Some(living));
    spent
}

thread_local! {
    /// The Living half of the most recent `spend_ammo` call.
    static LAST_AMMO_SPLIT: std::cell::RefCell<Option<i32>> = const { std::cell::RefCell::new(None) };
}

fn take_ammo_split() -> Option<i32> {
    LAST_AMMO_SPLIT.with(|split| split.borrow_mut().take())
}

/// Pay for and perform a `[Reload]` of The Living & The Departed.
///
/// "Consume (30 - (The Living + The Departed))/2 SP.  Reset both values on self
/// to 0, and gain the max value of their sum based on the probabilities listed
/// in the Passive `ISeeTheDyingButterfly.` - On Reload, gain at least 1 of The
/// Living and The Departed." (wiki.gg `Status Effects` / Reload)
pub fn reload_solitude(state: &mut BattleState, unit_index: usize) -> String {
    let sum = state.units[unit_index].statuses.total(AMMO_SOLEMN_LAMENT);
    let sp_cost = ((30 - sum) / 2).max(0);
    let sanity = state.units[unit_index].sanity;
    state.units[unit_index].sanity = sanity.add(-sp_cost);
    state.units[unit_index].statuses.remove(AMMO_SOLEMN_LAMENT);
    let mut living = 0;
    // The pool's maximum sum is 20.
    for _ in 0..20 {
        if roll_living(state, unit_index) {
            living += 1;
        }
    }
    let living = living.clamp(1, 19);
    let unit = &mut state.units[unit_index];
    unit.statuses.add_potency(AMMO_SOLEMN_LAMENT, living);
    unit.statuses
        .add_count(AMMO_SOLEMN_LAMENT, 20 - living);
    unit.statuses.add_potency("Reload (Solemn Lament)", 1);
    format!("Reloaded [The Living & The Departed] (SP -{sp_cost})")
}

/// `Petals` "can be gained up to 15 Stacks per turn" (wiki.gg `Status Effects`).
fn petals_gain_cap(state: &BattleState, unit_index: usize) -> i32 {
    (15 - state.units[unit_index].petals_gained).max(0)
}

pub fn corrosion_chance(state: &BattleState, unit_index: usize) -> Option<i32> {
    corrosion_chance_for_sp(state.units[unit_index].sanity.sp())
}

/// The same table for an SP value the unit does not have yet (the E.G.O cost is
/// paid at [Before Attack], but the random Corrosion roll happens when the Skill
/// is built).
fn corrosion_chance_for_sp(sp: i32) -> Option<i32> {
    if sp >= -24 {
        return None;
    }
    Some(if sp >= -34 {
        25
    } else if sp >= -44 {
        75
    } else {
        100
    })
}

/// Give back the 0.5x surcharge an Overclock paid, so a random Corrosion costs
/// the normal Corrosion price.
fn refund_overclock_surcharge(
    state: &mut BattleState,
    ego: &crate::library::EgoRecord,
) {
    for (sin, amount) in crate::library::Library::ego_cost(ego) {
        let surcharge = (amount as f64 * 1.5).ceil() as i32 - amount;
        if surcharge > 0 {
            *state.ego_resources.entry(sin).or_insert(0) += surcharge;
        }
    }
}

/// Pay the E.G.O costs.  Returns a description for the log.
/// The SP an E.G.O costs, before it is paid.
pub fn ego_sp_cost(ego: &crate::library::EgoRecord, kind: EgoSkillKind) -> i32 {
    let (sp_cost, multiplier) = match kind {
        EgoSkillKind::Awakening => (ego.awakening_sp.unwrap_or(0), 1.0),
        EgoSkillKind::Corrosion => (ego.corrosion_sp.unwrap_or(0), 1.0),
        EgoSkillKind::Overclock => (ego.corrosion_sp.unwrap_or(0), 1.5),
    };
    if multiplier == 1.0 {
        sp_cost
    } else {
        (sp_cost as f64 * multiplier).ceil() as i32
    }
}

/// Spend the E.G.O's SP.  The wiki puts this at the **[Before Attack]** timing,
/// after the Clash is decided and before the attack rolls, so the SP loss cannot
/// change the Clash's Heads rate
/// (JA-wiki ダメージ / 攻撃の流れ: "「攻撃前」のタイミングが発生。攻撃前効果が発動し、
/// E.G.Oを使用していた場合は精神力が低下する"; wiki.gg `Clash`).
pub fn pay_ego_sp(state: &mut BattleState, unit_index: usize, sp: i32) {
    let sanity = state.units[unit_index].sanity;
    state.units[unit_index].sanity = sanity.add(-sp);
}

fn pay_ego(state: &mut BattleState, _unit_index: usize, ego: &crate::library::EgoRecord, kind: EgoSkillKind) -> String {
    let multiplier = match kind {
        EgoSkillKind::Overclock => 1.5,
        _ => 1.0,
    };
    let mut spent = Vec::new();
    for (sin, amount) in crate::library::Library::ego_cost(ego) {
        let needed = if multiplier == 1.0 {
            amount
        } else {
            (amount as f64 * multiplier).ceil() as i32
        };
        let entry = state.ego_resources.entry(sin.clone()).or_insert(0);
        *entry -= needed;
        spent.push(format!("{needed} {sin}"));
    }
    format!("resources: {}", spent.join(", "))
}

// --------------------------------------------------------------------------- //
// conditions
// --------------------------------------------------------------------------- //

fn unit_status_value(unit: &Unit, key: &str, component: Option<Component>) -> i32 {
    match component {
        Some(Component::Potency) => unit.statuses.potency(key),
        Some(Component::Count) => unit.statuses.count(key),
        Some(Component::Stack) => unit.statuses.stack(key),
        // No component: "has [X]" covers Potency + Count and Stack, so that
        // stack-only statuses (Dazzle, Charge, ...) are detected too.
        None => {
            let status = unit.statuses.get(key);
            status.potency + status.count + status.stack
        }
    }
}

fn sum_statuses(unit: &Unit, statuses: &[String], component: Option<Component>) -> i32 {
    // A status listed twice means "both values" (the wiki's "both [Butterfly]":
    // Potency **and** Count, counted once).
    let mut total = 0;
    let mut done: Vec<&String> = Vec::new();
    for key in statuses {
        if done.contains(&key) {
            continue;
        }
        done.push(key);
        let occurrences = statuses.iter().filter(|entry| *entry == key).count();
        if occurrences >= 2 {
            total += unit.statuses.potency(key) + unit.statuses.count(key);
        } else {
            total += unit_status_value(unit, key, component);
        }
    }
    total
}

fn condition_holds(
    condition: &Condition,
    actor: &Unit,
    target: Option<&Unit>,
    clash_count: i32,
    slot: u32,
) -> bool {
    // "If any of the following conditions are met, ..."
    if !condition.any_of.is_empty() {
        return condition
            .any_of
            .iter()
            .any(|alternative| condition_holds(alternative, actor, target, clash_count, slot));
    }
    if let Some(limit) = condition.self_speed_at_most {
        if actor.speed > limit {
            return false;
        }
    }
    if let Some(advantage) = condition.target_speed_advantage {
        match target {
            Some(target) if target.speed - actor.speed >= advantage => {}
            _ => return false,
        }
    }
    if let Some(limit) = condition.target_sp_below {
        match target {
            Some(target) if target.sanity.sp() < limit => {}
            _ => return false,
        }
    }
    if let Some(required) = condition.self_sp_at_least {
        if actor.sanity.sp() < required {
            return false;
        }
    }
    if let Some(limit) = condition.self_sp_below {
        if actor.sanity.sp() >= limit {
            return false;
        }
    }
    // Sin Resonance conditions are read from the values the combat phase
    // computed for this turn (see `compute_resonance`).
    if let Some(required) = condition.a_reson_gte {
        if actor.a_reson_max < required {
            return false;
        }
    }
    if let Some(required) = condition.resonance_gte {
        let have = match condition.resonance_of.as_deref() {
            // The Resonance table is keyed by the lowercase sin (`gloom`), while
            // the Skill text writes `Gloom`.
            Some(sin) => actor
                .resonance_of
                .get(&sin.to_lowercase())
                .copied()
                .unwrap_or(0),
            None => actor.resonance_max,
        };
        if have < required {
            return false;
        }
    }
    if condition.requires_a_reson && actor.a_reson_max < 3 {
        return false;
    }
    let source_is_target = condition.source.as_deref() == Some("target");
    let unit = if source_is_target {
        match target {
            Some(t) => t,
            None => return false,
        }
    } else {
        actor
    };
    if let Some(hp) = condition.hp_above_percent {
        if unit.hp_percent() <= hp {
            return false;
        }
    }
    if let Some(hp) = condition.hp_below_percent {
        let percent = unit.hp_percent();
        let ok = if condition.hp_or_equal.unwrap_or(false) {
            percent <= hp
        } else {
            percent < hp
        };
        if !ok {
            return false;
        }
    }
    if let Some(count) = condition.clash_count_gte {
        if clash_count < count {
            return false;
        }
    }
    if condition.target_defeated {
        match target {
            Some(target) if !target.alive => {}
            _ => return false,
        }
    }
    if condition.target_survived {
        match target {
            Some(target) if target.alive => {}
            _ => return false,
        }
    }
    if !condition.any_status.is_empty() {
        let unit = match target {
            Some(target) => target,
            None => return false,
        };
        if !condition
            .any_status
            .iter()
            .any(|status| has_named(unit, status))
        {
            return false;
        }
    }
    if condition.target_is_low_morale || condition.target_is_panicked {
        let in_state = match target {
            Some(target) => {
                (condition.target_is_panicked && target.panicked)
                    || (condition.target_is_low_morale && (target.low_morale || target.panicked))
            }
            None => false,
        };
        if !in_state {
            return false;
        }
    }
    if condition.target_is_sp_unit {
        match target {
            Some(target) if !matches!(target.sanity, Sanity::None) => {}
            _ => return false,
        }
    }
    if condition.target_is_non_sp_unit {
        match target {
            Some(target) if matches!(target.sanity, Sanity::None) => {}
            _ => return false,
        }
    }
    if let Some(where_) = condition.slot.as_deref() {
        // "If this Skill was equipped on this unit's leftmost Skill Slot".
        if where_ == "leftmost" && slot != 0 {
            return false;
        }
    }
    if condition.any_target_killed {
        if actor.skill_kills <= 0 {
            return false;
        }
    }
    // "When attacking just a single target, Coin Power +2 and deal +100% damage"
    // (the Skill use must hit one unit).
    if condition.single_target && actor.planned_targets > 1 {
        return false;
    }
    if condition.target_has_amplitude {
        match target {
            Some(target) if has_amplitude(target) => {}
            _ => return false,
        }
    }
    for required in &condition.has_status {
        if !has_named(unit, required) {
            return false;
        }
    }
    for absent in &condition.lacks_status {
        if has_named(unit, absent) {
            return false;
        }
    }
    let mut value = None;
    if !condition.statuses.is_empty() {
        value = Some(sum_statuses(unit, &condition.statuses, condition.component));
    } else if let Some(status) = &condition.status {
        value = Some(unit_status_value(unit, status, condition.component));
    }
    if let Some(value) = value {
        if let Some(gte) = condition.gte {
            if value < gte {
                return false;
            }
        }
        if let Some(lte) = condition.lte {
            if value > lte {
                return false;
            }
        }
    }
    true
}

/// Scaled value helper for power kinds: `value + (measured / per) * step`,
/// capped by `max`.  `value` is the base for `clash_power` / `coin_power` /
/// `base_power` / `damage_percent`.
///
/// Without a `per` the effect is a flat conditional bonus: the status written in
/// its condition is only a threshold ("with 4 [Sinking], Coin Power +1"), so the
/// measured value must not be added to it.  Only `per` turns the measurement
/// into a multiplier.
fn scaled(effect: &Effect, measured: i32) -> i32 {
    let value = effect.value.unwrap_or(0);
    let max = effect.max.unwrap_or(i32::MAX);
    let Some(per) = effect.per.filter(|per| *per > 0) else {
        return value.min(max);
    };
    let step = effect.step.unwrap_or(1);
    (value + (measured / per) * step).min(max)
}

/// Scaled amount for status applications: the effect's own `potency` / `count`
/// is the base, and "for every N ..." forms add `(measured / per) * step`.
fn amount(base: i32, effect: &Effect, measured: i32) -> i32 {
    let per = effect.per.unwrap_or(0);
    let step = effect.step.unwrap_or(1);
    let max = effect.max.unwrap_or(i32::MAX);
    let scaled_part = if per > 0 { (measured / per) * step } else { 0 };
    (base + scaled_part).min(max)
}

// --------------------------------------------------------------------------- //
// effect application
// --------------------------------------------------------------------------- //

/// Tremor amplitudes: "[Amplitude Conversion] into [Tremor - Decay]" is stored
/// as a marker status on the unit (wiki.gg `Status Effects` / Tremor).
pub const AMPLITUDE_CONVERSION: &str = "Amplitude Conversion";
pub const AMPLITUDE_ENTANGLEMENT: &str = "Amplitude Entanglement";

/// A status is "present" if it carries a value or is a marker (amplitudes).
fn has_named(unit: &Unit, name: &str) -> bool {
    unit_status_value(unit, name, None) > 0
        || unit.status_markers.iter().any(|marker| marker.starts_with(name))
}

#[doc(hidden)]
pub fn has_amplitude(unit: &Unit) -> bool {
    unit.status_markers
        .iter()
        .any(|marker| marker.starts_with(AMPLITUDE_CONVERSION) || marker.starts_with(AMPLITUDE_ENTANGLEMENT))
}

#[doc(hidden)]
pub fn amplitude_of(unit: &Unit) -> Option<&str> {
    unit.status_markers
        .iter()
        .find(|marker| marker.starts_with(AMPLITUDE_CONVERSION))
        .map(|marker| {
            marker
                .trim_start_matches(AMPLITUDE_CONVERSION)
                .trim_start_matches(':')
                .trim()
        })
}

/// Units a Heal / SP Heal / status gain reaches.  The wiki writes these as
/// "self and 2 other allies with the least SP", "3 allies with the lowest HP
/// percentages", "N random allies" (wiki.gg `Status Effects`).
fn ally_targets(state: &BattleState, ctx: &EffectContext<'_>, effect: &Effect) -> Vec<usize> {
    let actor_index = ctx.actor_index;
    let actor = &state.units[actor_index];
    let include_self = effect.include_self.unwrap_or(matches!(effect.ally.as_deref(), None | Some("self")));
    let same_side = |unit: &Unit| unit.alive && unit.kind.is_sinner() == actor.kind.is_sinner();
    let mut others: Vec<usize> = state
        .units
        .iter()
        .enumerate()
        .filter(|(index, unit)| *index != actor_index && same_side(unit))
        .map(|(index, _)| index)
        .collect();
    let kind = effect.ally.as_deref().unwrap_or("self");
    // "Apply 1 [Haste] next turn to ([Bind] on self / 3) other allies with the
    // slowest Speed": the number of allies comes from a status on the actor.
    let count = match effect.from_resonance {
        // "Heal 10 SP for (1 + highest Reson.) other allies with the least SP
        // (max 4 units)": the number of allies comes from the Resonance.
        true => {
            let base = effect.ally_count.unwrap_or(effect.value.unwrap_or(1));
            let reson = actor.resonance_max;
            ((base + reson).max(0) as usize).min(effect.max.unwrap_or(i32::MAX) as usize)
        }
        false => match effect.ally_from_status.as_deref() {
        Some(status) => {
            let divisor = effect.ally_from_divisor.unwrap_or(1).max(1);
            let have = actor.statuses.count(status)
                + actor.statuses.potency(status)
                + actor.statuses.stack(status);
            (have / divisor).max(0) as usize
        }
            None => effect
                .ally_count
                .or(effect.value)
                .unwrap_or(0)
                .max(0) as usize,
        },
    };
    if kind == "self" {
        return vec![actor_index];
    }
    match kind {
        "all_allies" | "all" => {
            if include_self {
                others.insert(0, actor_index);
            }
            return others;
        }
        "lowest_hp" => {
            others.sort_by_key(|index| state.units[*index].hp_percent());
            return finish(others, count, actor_index, include_self);
        }
        "lowest_sp" => {
            others.sort_by_key(|index| state.units[*index].sanity.sp());
            return finish(others, count, actor_index, include_self);
        }
        "slowest" => {
            others.sort_by_key(|index| state.units[*index].speed);
            return finish(others, count, actor_index, include_self);
        }
        "random" => {
            // Deterministic shuffle so replays stay identical.
            let mut seed = state.rng.clone();
            let mut picked = Vec::new();
            let mut pool = others;
            while !pool.is_empty() && picked.len() < count {
                let index = seed.below(pool.len() as u32) as usize;
                picked.push(pool.remove(index));
            }
            if include_self {
                picked.insert(0, actor_index);
            }
            return picked;
        }
        _ => return vec![actor_index],
    }
}

/// Pick up to `count` allies.  "(including this unit)" means the unit is one of
/// the `count`, not an extra one.
fn finish(allies: Vec<usize>, count: usize, actor_index: usize, include_self: bool) -> Vec<usize> {
    if include_self {
        let others = count.saturating_sub(1);
        let mut picked: Vec<usize> = allies.into_iter().take(others).collect();
        picked.insert(0, actor_index);
        return picked;
    }
    allies.into_iter().take(count).collect()
}

pub struct EffectContext<'a> {
    pub actor_index: usize,
    pub target_index: Option<usize>,
    pub clash_count: i32,
    /// True when the acting skill lost its Clash ("after Clash Lose" clauses).
    pub clash_lost: bool,
    /// Dashboard slot the skill was used from, when known.
    pub slot: u32,
    pub mechanics_note: &'a mut Vec<String>,
}

/// Apply a list of effects.  Unknown kinds are recorded, never silently dropped.
pub fn apply_effects(
    state: &mut BattleState,
    effects: &[Effect],
    ctx: &mut EffectContext<'_>,
    use_ctx: &mut UseContext,
) {
    for effect in effects {
        if effect.only_after_clash_lose && !ctx.clash_lost {
            continue;
        }
        // "50% chance to ..." and other probabilistic clauses take their flip
        // from the replayable RNG stream so replays stay identical.
        if let Some(chance) = effect.chance {
            if !state.flip(chance.clamp(0, 100)) {
                continue;
            }
        }
        // "If target was killed, activate the effect above once more".
        let _repeat = if effect.repeat_on_kill {
            ctx.target_index
                .map(|index| !state.units[index].alive)
                .unwrap_or(false)
        } else {
            false
        };
        // "(N times per Encounter)" limits live in the unit's usage map too,
        // keyed with an `encounter:` prefix so Turn Start does not clear them.
        // The attempt is only recorded once the clause actually applies, so a
        // failed condition does not spend the allowance.
        let mut usage_grants: Vec<String> = Vec::new();
        if let Some(limit) = effect.per_encounter {
            let key = format!(
                "encounter:{}",
                effect.raw.clone().unwrap_or_else(|| effect.kind.clone())
            );
            let used = state.units[ctx.actor_index]
                .turn_effect_usage
                .get(&key)
                .copied()
                .unwrap_or(0);
            if used >= limit {
                continue;
            }
            usage_grants.push(key);
        }
        // "(N times per turn)" limits.
        if let Some(limit) = effect.per_turn {
            let key = effect.raw.clone().unwrap_or_else(|| effect.kind.clone());
            let used = state.units[ctx.actor_index]
                .turn_effect_usage
                .get(&key)
                .copied()
                .unwrap_or(0);
            if used >= limit {
                continue;
            }
            usage_grants.push(key);
        }
        let holds = match &effect.condition {
            Some(cond) => {
                let actor = &state.units[ctx.actor_index];
                let target = ctx.target_index.map(|i| &state.units[i]);
                condition_holds(cond, actor, target, ctx.clash_count, ctx.slot)
            }
            None => true,
        };
        if !holds {
            continue;
        }
        match effect.kind.as_str() {
            "inflict" | "gain" => {
                let index = if effect.source.as_deref() == Some("self") || effect.kind == "gain" {
                    Some(ctx.actor_index)
                } else {
                    ctx.target_index
                };
                let Some(index) = index else { continue };
                let Some(status) = effect.status.clone() else { continue };
                // "inflict 3 [Sinking] on 2 random enemies" and other clauses
                // that reach beyond the main target.
                if effect.ally.is_some() {
                    let targets = ally_targets(state, ctx, effect);
                    let base = effect.potency.or(effect.count).unwrap_or(0);
                    for target in targets {
                        if effect.next_turn {
                            // "All allies gain 2 [Rhythm] next turn": the grant is
                            // queued, exactly like the self-targeted form.
                            let stack = effect.stack.unwrap_or(0);
                            state.units[target].pending_next_turn.push(
                                crate::state::PendingStatus {
                                    status: status.clone(),
                                    potency: if stack == 0 { base } else { 0 },
                                    count: if stack == 0 {
                                        effect.count.unwrap_or(base)
                                    } else {
                                        0
                                    },
                                    stack,
                                },
                            );
                            continue;
                        }
                        let unit = &mut state.units[target];
                        match effect.butterfly_part.as_deref() {
                            Some("departed") | Some("count") => {
                                unit.statuses.add_count(&status, base)
                            }
                            Some(_) => unit.statuses.add_potency(&status, base),
                            None => {
                                unit.statuses.add_potency(&status, base);
                                unit.statuses.add_count(&status, effect.count.unwrap_or(base));
                            }
                        }
                    }
                    continue;
                }
                let measured = effect
                    .condition
                    .as_ref()
                    .map(|c| {
                        let unit = &state.units[ctx.actor_index];
                        if !c.statuses.is_empty() {
                            sum_statuses(unit, &c.statuses, c.component)
                        } else {
                            c.status.as_deref().map(|s| unit_status_value(unit, s, c.component)).unwrap_or(0)
                        }
                    })
                    .unwrap_or(0);
                let potency = effect
                    .potency
                    .map(|base| amount(base, effect, measured))
                    .unwrap_or(0);
                let count = effect
                    .count
                    .map(|base| amount(base, effect, measured))
                    .unwrap_or(0);
                let stack_amount = effect
                    .stack
                    .map(|base| amount(base, effect, measured))
                    .unwrap_or(0);
                // "A random ally gains 1 ~ 2 [Rhythm]".
                let (potency, count) = if let (Some(lo), Some(hi)) =
                    (effect.range_min, effect.range_max)
                {
                    let rolled = lo + state.roll_inclusive(hi - lo);
                    (rolled, 0)
                } else {
                    (potency, count)
                };
                let potency = if let Some(per_sp) = effect.per_sp {
                    // "Turn Start: gain 1 [Protecting Sword] for every 8 SP".
                    let sp = state.units[ctx.actor_index].sanity.sp();
                    let steps = (sp.abs() / per_sp).max(0);
                    (steps * effect.potency.unwrap_or(1)).min(effect.max.unwrap_or(i32::MAX))
                } else if effect.from_resonance && effect.per.is_some() {
                    // "Gain 1 [Lamp] for every 2 Gloom Reson. (max 3)": one step
                    // per whole `per` Resonance.
                    let per = effect.per.unwrap_or(1).max(1);
                    let step = effect.value.or(effect.potency).unwrap_or(1);
                    let reson = match effect.resonance_of.as_deref() {
                        Some(sin) => state.units[ctx.actor_index]
                            .resonance_of
                            .get(&sin.to_lowercase())
                            .copied()
                            .unwrap_or(0),
                        None => state.units[ctx.actor_index].resonance_max,
                    };
                    ((reson / per) * step).min(effect.max.unwrap_or(i32::MAX))
                } else if effect.multiplier.is_some() || effect.from_resonance {
                    // "(6 + Gloom Reson.) [Sinking]", "(Gloom Reson. + 1) ..."
                    let base = effect.value.unwrap_or(0);
                    let multiplier = effect.multiplier.unwrap_or(1);
                    let reson = match effect.resonance_of.as_deref() {
                        Some(sin) => state.units[ctx.actor_index]
                            .resonance_of
                            .get(&sin.to_lowercase())
                            .copied()
                            .unwrap_or(0),
                        None => state.units[ctx.actor_index].resonance_max,
                    };
                    (base + multiplier * reson).min(effect.max.unwrap_or(i32::MAX))
                } else {
                    potency
                };
                if effect.next_turn {
                    // "Gain 2 Protection next turn" / "Inflict 3 [Blue Sand] next
                    // turn" -> applied at the next Turn Start, component included.
                    state.units[index].pending_next_turn.push(crate::state::PendingStatus {
                        status: status.clone(),
                        potency,
                        count,
                        stack: stack_amount,
                    });
                    continue;
                }
                // Stack-based statuses (In the Past/Present/Future, Dazzle, …)
                // are written to Stack instead of Potency/Count.
                if effect.component == Some(Component::Stack) {
                    let store = &mut state.units[index].statuses;
                    let mut amount = if potency != 0 {
                        potency
                    } else if stack_amount != 0 {
                        stack_amount
                    } else {
                        count
                    };
                    // "Gain [X] up to N Stack".
                    if let Some(cap) = effect.up_to {
                        amount = amount.min((cap - store.stack(&status)).max(0));
                    }
                    store.add_stack(&status, amount);
                    continue;
                }
                // "[Butterfly](The Departed)" is the Count half of the unique
                // Sinking, "[Butterfly](The Living)" its Potency (wiki.gg
                // `Status Effects` / Butterfly).
                // "(Chance to flip Heads)% chance to inflict The Departed ...
                // (calculates every [Butterfly] Stack independently)".
                if status == "Butterfly" && use_ctx.butterfly_split && effect.butterfly_part.is_none() {
                    let heads_percent = heads_percent(state, &state.units[ctx.actor_index]);
                    let stacks = potency.max(count);
                    let mut departed = 0;
                    for _ in 0..stacks {
                        if state.flip(heads_percent) {
                            departed += 1;
                        }
                    }
                    let unit = &mut state.units[index];
                    unit.statuses.add_potency(&status, stacks - departed);
                    unit.statuses.add_count(&status, departed);
                    continue;
                }
                if let Some(part) = effect.butterfly_part.as_deref() {
                    let unit = &mut state.units[index];
                    match part {
                        "departed" | "count" => {
                            unit.statuses.add_count(&status, count.max(potency));
                        }
                        _ => {
                            unit.statuses.add_potency(&status, potency.max(count));
                        }
                    }
                    continue;
                }
                let count_status = effect.status2.clone().unwrap_or_else(|| status.clone());
                // A state of time makes the unit inflict or gain more of its
                // status (Burn / Poise / Bleed).  Butterfly is exempt: "No
                // external effects can raise the amount of Butterfly inflicted"
                // (in-game `BattleKeywords` / SinkingWhite).
                let (mut potency, mut count) = (potency, count);
                if status == "Butterfly" {
                    let unit = &mut state.units[index];
                    unit.statuses.add_potency(&status, potency);
                    unit.statuses
                        .add_count(&effect.status2.clone().unwrap_or_else(|| status.clone()), count);
                    continue;
                }
                if let Some((boosted, bonus)) = time_state_bonus(state, ctx.actor_index) {
                    if status == boosted {
                        potency += bonus.potency;
                    }
                    if status == boosted || count_status == boosted {
                        count += bonus.count;
                    }
                }
                let unit = &mut state.units[index];
                unit.statuses.add_potency(&status, potency);
                unit.statuses.add_count(&count_status, count);
            }
            "spend_ammo" => {
                let status = effect
                    .status
                    .clone()
                    .unwrap_or_else(|| AMMO_SOLEMN_LAMENT.to_string());
                let amount = effect.value.unwrap_or(1).max(0);
                let spent = spend_ammo(state, ctx.actor_index, &status, amount);
                if let Some(living) = take_ammo_split() {
                    use_ctx.ammo_living += living;
                    use_ctx.ammo_departed += spent - living;
                }
                if spent < amount {
                    // "Some attacks cancel if this unit runs out of ammo" - the
                    // remaining Coins are forfeited (and Reload kicks in).
                    use_ctx.ammo_exhausted = true;
                }
                use_ctx.ammo_spent += spent;
                *use_ctx.ammo_spent_by_status.entry(status).or_insert(0) += spent;
            }
            "spend_ammo_all" => {
                // "Spend all of [The Living & The Departed] on self".
                let status = effect
                    .status
                    .clone()
                    .unwrap_or_else(|| AMMO_SOLEMN_LAMENT.to_string());
                let have = state.units[ctx.actor_index].statuses.total(&status);
                let spent = spend_ammo(state, ctx.actor_index, &status, have);
                use_ctx.ammo_spent += spent;
                *use_ctx.ammo_spent_by_status.entry(status).or_insert(0) += spent;
            }
            "inflict_equal_ammo_spent" => {
                let Some(status) = effect.status.clone() else { continue };
                let Some(index) = ctx.target_index else { continue };
                // "Inflict [Butterfly] equal to [The Living & The Departed]
                // spent": the unique Sinking keeps the split the Skill consumed
                // ("inflict the same amount of The Living and The Departed as
                // they were consumed").
                let ammo = effect
                    .ammo
                    .clone()
                    .unwrap_or_else(|| AMMO_SOLEMN_LAMENT.to_string());
                if ammo == AMMO_SOLEMN_LAMENT {
                    let living = use_ctx.ammo_living;
                    let departed = use_ctx.ammo_departed;
                    let unit = &mut state.units[index];
                    if living != 0 {
                        unit.statuses.add_potency(&status, living);
                    }
                    if departed != 0 {
                        unit.statuses.add_count(&status, departed);
                    }
                } else {
                    let spent = use_ctx
                        .ammo_spent_by_status
                        .get(&ammo)
                        .copied()
                        .unwrap_or(use_ctx.ammo_spent);
                    state.units[index].statuses.add_potency(&status, spent);
                }
            }
            "clash_power" => {
                let measured = measured_from_condition(effect, &state.units[ctx.actor_index], ctx.target_index.map(|i| &state.units[i]));
                use_ctx.clash_power_bonus += scaled(effect, measured);
            }
            "coin_power" => {
                let measured = measured_from_condition(effect, &state.units[ctx.actor_index], ctx.target_index.map(|i| &state.units[i]));
                use_ctx.coin_power_bonus += scaled(effect, measured);
            }
            "base_power" => {
                let measured = measured_from_condition(effect, &state.units[ctx.actor_index], ctx.target_index.map(|i| &state.units[i]));
                use_ctx.base_power_bonus += scaled(effect, measured);
            }
            "damage_percent" => {
                let measured = measured_from_condition(effect, &state.units[ctx.actor_index], ctx.target_index.map(|i| &state.units[i]));
                use_ctx.damage_bonus += scaled(effect, measured) as f64 / 100.0;
            }
            "shield_percent_hp" => {
                let percent = effect.percent.unwrap_or(0);
                let max_percent = effect.max.unwrap_or(percent);
                let measured = measured_from_condition(effect, &state.units[ctx.actor_index], ctx.target_index.map(|i| &state.units[i]));
                let gain = (percent * measured).min(max_percent);
                let unit = &mut state.units[ctx.actor_index];
                let shield = unit.max_hp * gain / 100;
                use_ctx.shield_gain += shield;
            }
            "sp_damage" => {
                if let Some(index) = ctx.target_index {
                    let value = effect.value.unwrap_or(0);
                    let sanity = state.units[index].sanity;
                    state.units[index].sanity = sanity.add(-value);
                }
            }
            "unbreakable_coin" => {
                use_ctx.unbreakable_coins.extend(effect.coins.iter().copied());
            }
            "tremor_burst" => {
                // "Raise target's Stagger Threshold by [Tremor] Potency on target"
                let Some(index) = ctx.target_index else { continue };
                let potency = state.units[index].statuses.potency("Tremor");
                if potency > 0 {
                    // "Gain 2 [AlriuneEGOWe] every time Tremor Burst is triggered
                    // regardless of the unit" (Unwithering Flower).
                    for other in 0..state.units.len() {
                        if state.units[other]
                            .passive_ids
                            .iter()
                            .any(|id| id == "1041411")
                            && petals_gain_cap(state, other) >= 2
                        {
                            state.units[other].statuses.add_stack("Petals", 2);
                            state.units[other].petals_gained += 2;
                        }
                    }
                    state.units[index].stagger.shift_first_threshold(potency);
                    // "Trigger [Tremor Burst]; then, reduce target's [Tremor]
                    // Count by 1" - the reduction is its own clause, so a burst
                    // without it consumes nothing.
                    if let Some(consume) = effect.consume_count.filter(|value| *value > 0) {
                        tick_status(state, index, "Tremor", 0, -consume);
                    }
                }
            }
            "activate_status" => {
                let Some(status) = effect.status.clone() else { continue };
                let times = effect.times.unwrap_or(1);
                let consume = effect.consume_count.unwrap_or(1);
                let Some(index) = ctx.target_index else { continue };
                let unit = &mut state.units[index];
                let potency = unit.statuses.potency(&status);
                if potency > 0 {
                    // "Activate [Burn] on target 2 times" deals the Potency twice.
                    let total = potency * times;
                    unit.take_damage(total);
                }
                // "Target loses 2 [Burn] Count" is the total, not per activation.
                if consume > 0 {
                    tick_status(state, index, &status, 0, -consume);
                }
            }
            "lose_status_count" => {
                let Some(status) = effect.status.clone() else { continue };
                let Some(index) = ctx.target_index else { continue };
                let amount = effect.count.or(effect.value).unwrap_or(1);
                tick_status(state, index, &status, 0, -amount);
            }
            "reload_ammo" => {
                let unit = &state.units[ctx.actor_index];
                if effect.requires_a_reson && unit.a_reson_max < 3 {
                    continue;
                }
                if let Some(required) = effect.a_reson_gte {
                    if unit.a_reson_max < required {
                        continue;
                    }
                }
                match effect.ammo.as_deref() {
                    Some(AMMO_FRACTURE) => {
                        // "At 2 or fewer [LCA Fracture Round], [Reload]": the
                        // Unique Ammo refills to its capacity.
                        state.units[ctx.actor_index]
                            .statuses
                            .set_stack(AMMO_FRACTURE, 16);
                        state.push_log("reload", "Reloaded [LCA Fracture Round]".to_string());
                    }
                    Some(AMMO_SOLITUDE) => {
                        state.units[ctx.actor_index]
                            .statuses
                            .set_stack(AMMO_SOLITUDE, 6);
                        state.push_log("reload", "Reloaded [Bullet - Solitude]".to_string());
                    }
                    _ => {
                        // Reload (Solemn Lament): spend the SP, reset both values
                        // and refill the pool.
                        let detail = reload_solitude(state, ctx.actor_index);
                        state.push_log("reload", detail);
                    }
                }
            }
            "reuse_percent_missing_hp" => {
                // "Reuse this Coin once for every N% missing HP (max K times)".
                use_ctx.reuse_hp = Some((effect.per.unwrap_or(33).max(1), effect.max.unwrap_or(1)));
            }
            "inflict_per_ammo" => {
                // "If [LCA_Bullet] was spent, inflict additional [SheutFracture]
                // per [LCA_Bullet] spent" (Outis's Scepter of Horus - Replica).
                let Some(status) = effect.status.clone() else { continue };
                let Some(index) = ctx.target_index else { continue };
                let ammo = effect
                    .ammo
                    .clone()
                    .unwrap_or_else(|| AMMO_FRACTURE.to_string());
                let spent = use_ctx.ammo_spent_by_status.get(&ammo).copied().unwrap_or(0);
                let amount = effect.potency.unwrap_or(1) + spent;
                state.units[index].statuses.add_potency(&status, amount);
            }
            "inflict_on_attacker" => {
                // Stored on the defender and applied when it is hit with Shield.
                let Some(status) = effect.status.clone() else { continue };
                let potency = effect.potency.unwrap_or(0);
                state.units[ctx.actor_index]
                    .retaliate_on_hit
                    .push(crate::state::RetaliateOnHit { status, potency });
            }
            "gain_from_resonance" => {
                // "Gain (highest Reson.) of [X] (max N)"
                let unit = &state.units[ctx.actor_index];
                if effect.requires_a_reson && unit.a_reson_max < 3 {
                    continue;
                }
                let multiplier = effect.multiplier.unwrap_or(1);
                let max = effect.max.unwrap_or(i32::MAX);
                let amount = (unit.resonance_max * multiplier).min(max);
                if amount > 0 {
                    if let Some(status) = effect.status.clone() {
                        let unit = &mut state.units[ctx.actor_index];
                        unit.statuses.add_potency(&status, amount);
                    }
                }
            }
            "discard_other_in_slot" => {
                // "If the other Skill in the same Skill Slot is a different
                // Skill, [Discard] that Skill" (the second visible skill).
                discard_from_slot(state, ctx.actor_index, ctx.slot, true);
            }
            "discard_lowest_rank" => {
                let count = effect.value.unwrap_or(1);
                discard_lowest_rank(state, ctx.actor_index, count);
            }
            "consume_surplus_status" => {
                // "If this unit has 20+ [Poise] Potency, consume up to 20 surplus
                // Potency past 20 to deal +(consumed x N)% damage".
                let Some(status) = effect.status.clone() else { continue };
                let threshold = effect.threshold.unwrap_or(0);
                let limit = effect.value.unwrap_or(0);
                let step = effect.step.unwrap_or(0);
                let max = effect.max.unwrap_or(i32::MAX);
                let unit = &state.units[ctx.actor_index];
                let potency = unit.statuses.potency(&status);
                if potency > threshold {
                    let surplus = (potency - threshold).min(limit);
                    // Consuming Potency can empty the status: remove it when the
                    // value it uses reaches 0.
                    tick_status(state, ctx.actor_index, &status, -surplus, 0);
                    use_ctx.damage_bonus += ((surplus * step).min(max)) as f64 / 100.0;
                }
            }
            "reuse_on_kill" => {
                use_ctx.reuse_on_kill = true;
            }
            "damage_taken_percent" => {
                // "Take -(SP / 2)% HP damage from attacks (max 20%)": the value
                // is negative for a reduction and is capped by `max`.
                let percent = effect.value.unwrap_or(0);
                let max = effect.max.unwrap_or(i32::MAX);
                use_ctx.damage_taken_bonus += (percent.clamp(-max, max)) as f64 / 100.0;
            }
            "damage_percent_from_negative_sp" => {
                // "Deal +(-SP/2)% damage with Base Skills (max 20%)".
                let per = effect.per.unwrap_or(1).max(1);
                let step = effect.step.unwrap_or(1);
                let max = effect.max.unwrap_or(i32::MAX);
                let sp = state.units[ctx.actor_index].sanity.sp();
                let deficit = (-sp).max(0);
                use_ctx.damage_bonus += ((deficit / per) * step).min(max) as f64 / 100.0;
            }
            "damage_percent_missing_hp" => {
                // "Deal more damage based on missing HP on self (max 15%)".
                let percent = effect.damage_percent_missing_hp.unwrap_or(0);
                let missing = 100 - state.units[ctx.actor_index].hp_percent();
                use_ctx.damage_bonus += (percent * missing / 100) as f64 / 100.0;
            }
            "extra_damage_percent_of_damage" => {
                use_ctx.final_damage_percent += effect.final_damage_percent.unwrap_or(0);
            }
            "resist_floor" => {
                if let Some(kind) = effect.status.clone() {
                    let value = effect.value.unwrap_or(0) as f64 / 10.0;
                    use_ctx.resist_floor.push((kind, value));
                }
            }
            "halve_status" => {
                let Some(status) = effect.status.clone() else { continue };
                let unit = &mut state.units[ctx.actor_index];
                let stack = unit.statuses.stack(&status);
                unit.statuses.set_stack(&status, stack / 2);
                if !has_named(&state.units[ctx.actor_index], &status) {
                    state.units[ctx.actor_index].statuses.remove(&status);
                }
            }
            "damage_from_status_divisor" => {
                // "Deal ([Poise] on self / 2) Pride damage on target"
                let Some(status) = effect.status.clone() else { continue };
                let divisor = effect.value.unwrap_or(1).max(1);
                let amount = state.units[ctx.actor_index].statuses.potency(&status) / divisor;
                if amount > 0 {
                    if let Some(index) = ctx.target_index {
                        let sin = effect
                            .sin
                            .as_deref()
                            .and_then(Sin::parse)
                            .unwrap_or(use_ctx.sin);
                        let resist = state.units[index].resist_sin(sin);
                        let damage = (amount as f64
                            * (1.0 + crate::damage::resistance_modifier(resist)))
                        .floor()
                        .max(1.0) as i32;
                        state.units[index].take_damage(damage);
                    }
                }
                if let Some(count) = effect.count {
                    let unit = &mut state.units[ctx.actor_index];
                    unit.statuses.add_count(&status, -count);
                }
            }
            "base_power_per_ammo_planned" => {
                // "Base Power +1 for every [LCA Fracture Round] about to be spent
                // by this Skill": capped by the ammo the unit actually holds.
                let status = effect
                    .status
                    .clone()
                    .unwrap_or_else(|| AMMO_SOLEMN_LAMENT.to_string());
                let available = state.units[ctx.actor_index].statuses.total(&status);
                let planned = use_ctx.ammo_planned.min(available);
                let step = effect.step.unwrap_or(1);
                use_ctx.base_power_bonus += step * planned;
            }
            "crit_chance_from_target_sp" => {
                // "boost crit chance proportional to target's SP" (a negative SP
                // target raises the crit chance).
                if let Some(index) = ctx.target_index {
                    let sp = state.units[index].sanity.sp();
                    if sp < 0 {
                        use_ctx.crit_chance_bonus += -sp;
                    }
                }
            }
            "crit_damage_bonus" => {
                use_ctx.crit_damage_bonus += effect.percent.unwrap_or(0) as f64 / 100.0;
            }
            "sp_damage_self_per_stack" => {
                let Some(status) = effect.status.clone() else { continue };
                let step = effect.step.unwrap_or(0);
                let stack = state.units[ctx.actor_index].statuses.stack(&status);
                let sanity = state.units[ctx.actor_index].sanity;
                state.units[ctx.actor_index].sanity = sanity.add(-(step * stack));
            }
            "turn_end_sp_and_gain" => {
                // "At less than 3 [Tear-sharpened], lose 15 SP and gain 1
                // [Tear-sharpened] / - If this unit has [Tear-sharpened], lose
                // ([Tear-sharpened] Stack x 15) more SP": the sub-clause reads the
                // Stack the unit had when the clause started, which is why the
                // extra loss is folded in here (see the extractor).
                let Some(status) = effect.status.clone() else { continue };
                let threshold = effect.threshold.unwrap_or(0);
                let unit = &state.units[ctx.actor_index];
                let current = unit.statuses.stack(&status);
                if current < threshold {
                    let extra = effect.step.unwrap_or(0) * current;
                    let sanity = unit.sanity;
                    state.units[ctx.actor_index].sanity =
                        sanity.add(-(effect.value.unwrap_or(0) + extra));
                    let count = effect.count.unwrap_or(1);
                    state.units[ctx.actor_index].statuses.add_stack(&status, count);
                }
            }
            "consume_status_to_inflict" => {
                // "consume 1 [PenetratingSword] to inflict 1 [Sinking]",
                // "consume all [SwordCutwithTear] to inflict 3 [Sinking] and
                // +3 [Sinking] Count, and deal +50% damage with that Coin".
                let Some(from) = effect.status.clone() else { continue };
                let Some(to) = effect.status2.clone() else { continue };
                let limit = effect.value.unwrap_or(1).max(1);
                let unit = &state.units[ctx.actor_index];
                let have = unit.statuses.stack(&from) + unit.statuses.potency(&from)
                    + unit.statuses.count(&from);
                if have <= 0 {
                    continue;
                }
                let consume = have.min(limit);
                let unit = &mut state.units[ctx.actor_index];
                unit.statuses.add_stack(&from, -consume);
                unit.statuses.remove(&from);
                use_ctx.consumed_status += consume;
                *use_ctx.consumed_by_status.entry(from).or_insert(0) += consume;
                let Some(target) = ctx.target_index else { continue };
                let potency = effect.potency.unwrap_or(consume).max(consume);
                let count = effect.count.unwrap_or(0);
                state.units[target].statuses.add_potency(&to, potency);
                if count != 0 {
                    state.units[target].statuses.add_count(&to, count);
                }
                if let Some(percent) = effect.percent {
                    use_ctx.damage_bonus += percent as f64 / 100.0;
                }
            }
            "consume_status_up_to" => {
                let Some(status) = effect.status.clone() else { continue };
                let threshold = effect.threshold.unwrap_or(0);
                let limit = effect.value.unwrap_or(0);
                let unit = &state.units[ctx.actor_index];
                let have = unit.statuses.potency(&status) + unit.statuses.count(&status);
                if have >= threshold {
                    // "consume up to 20 [Deep Tears]" spends what is there, at
                    // most 20 - the rest stays.
                    let consumed = consume_status_amount(
                        state,
                        ctx.actor_index,
                        &status,
                        have.min(limit),
                    );
                    use_ctx.consumed_status += consumed;
                    *use_ctx.consumed_by_status.entry(status).or_insert(0) += consumed;
                }
            }
            "damage_percent_per_consumed_status" => {
                let step = effect.step.unwrap_or(0);
                use_ctx.damage_bonus += (step * use_ctx.consumed_status) as f64 / 100.0;
            }
            "lower_own_stagger_threshold" => {
                // Applied after the hit, using the damage dealt (see apply_hit).
                use_ctx.lower_stagger_percent = effect.percent.unwrap_or(0);
            }
            "damage_percent_per_ammo_spent" => {
                // "Deal +2% damage for every value of [X] spent by this Skill":
                // the pool the clause names, not every pool the Skill spent.
                let step = effect.step.unwrap_or(0);
                let spent = match effect.status.as_deref().filter(|name| !name.contains(" spent")) {
                    Some(status) => use_ctx
                        .ammo_spent_by_status
                        .get(status)
                        .copied()
                        .unwrap_or(0),
                    None => use_ctx.ammo_spent,
                };
                use_ctx.damage_bonus += (step * spent) as f64 / 100.0;
            }
            "gloom_damage_equal_target_status" => {
                // "[On Hit] Inflict Gloom Damage equal to "All" [Butterfly] on
                // target", where "All" is the sum of both values on the target.
                let Some(status) = effect.status.clone() else { continue };
                let Some(index) = ctx.target_index else { continue };
                let amount = match effect.component {
                    Some(Component::Potency) => state.units[index].statuses.potency(&status),
                    Some(Component::Count) => state.units[index].statuses.count(&status),
                    Some(Component::Stack) => state.units[index].statuses.stack(&status),
                    None => {
                        state.units[index].statuses.potency(&status)
                            + state.units[index].statuses.count(&status)
                    }
                };
                if amount > 0 {
                    let resist = state.units[index].resist_sin(Sin::Gloom);
                    let damage = (amount as f64
                        * (1.0 + crate::damage::resistance_modifier(resist)))
                    .floor()
                    .max(1.0) as i32;
                    state.units[index].take_damage(damage);
                }
            }
            "consume_status_for_damage" => {
                let Some(status) = effect.status.clone() else { continue };
                let threshold = effect.threshold.unwrap_or(0);
                let consume = effect.value.unwrap_or(0);
                let unit = &state.units[ctx.actor_index];
                let have = unit.statuses.potency(&status)
                    + unit.statuses.count(&status)
                    + unit.statuses.stack(&status);
                if have >= threshold {
                    // "consume 5 [Deep Tears]" removes 5, not the whole status.
                    let taken = consume_status_amount(state, ctx.actor_index, &status, consume);
                    use_ctx.damage_bonus += effect.percent.unwrap_or(0) as f64 / 100.0;
                    use_ctx.consumed_status += taken;
                    *use_ctx.consumed_by_status.entry(status).or_insert(0) += taken;
                }
            }
            "shield_percent_from_sp" => {
                let divisor = effect.value.unwrap_or(1).max(1);
                let sp = state.units[ctx.actor_index].sanity.sp().max(0);
                let percent = sp / divisor;
                let shield = state.units[ctx.actor_index].max_hp * percent / 100;
                use_ctx.shield_gain += shield;
            }
            "shield_percent_per_status" => {
                let Some(status) = effect.status.clone() else { continue };
                let percent = effect.percent.unwrap_or(0);
                let max = effect.max.unwrap_or(percent);
                let count = state.units[ctx.actor_index].statuses.count(&status)
                    + state.units[ctx.actor_index].statuses.potency(&status);
                let total = (percent * count.max(1)).min(max);
                let shield = state.units[ctx.actor_index].max_hp * total / 100;
                use_ctx.shield_gain += shield;
            }
            "convert_unbreakable_and_clash" => {
                let value = effect.value.unwrap_or(0);
                use_ctx.clash_power_bonus += value;
                use_ctx.unbreakable_all = true;
            }
            "zero_damage" => {
                use_ctx.zero_damage = true;
            }
            "no_damage_taken" => {
                let unit = &mut state.units[ctx.actor_index];
                unit.statuses.set_stack("No Damage Taken", 1);
            }
            "attack_weight" => {
                if effect.from_resonance || effect.per.is_some() {
                    let per = effect.per.unwrap_or(1).max(1);
                    let step = effect.step.unwrap_or(1);
                    let max = effect.max.unwrap_or(i32::MAX);
                    let reson = match effect.resonance_of.as_deref() {
                        Some(sin) => state.units[ctx.actor_index]
                            .resonance_of
                            .get(&sin.to_lowercase())
                            .copied()
                            .unwrap_or(0),
                        None => state.units[ctx.actor_index].resonance_max,
                    };
                    use_ctx.attack_weight_bonus += ((reson / per) * step).min(max);
                } else {
                    use_ctx.attack_weight_bonus += effect.value.unwrap_or(1);
                }
            }
            "self_damage" => {
                let unit = &state.units[ctx.actor_index];
                let max_hp = unit.max_hp;
                let percent = effect.self_damage_percent.unwrap_or(0);
                let mut damage = max_hp * percent / 100;
                if let (Some(lo), Some(hi)) = (effect.self_damage_min, effect.self_damage_max) {
                    damage = lo + state.roll_inclusive(hi - lo);
                }
                if damage <= 0 {
                    continue;
                }
                // "does not reduce this unit's HP below 1" / "does not get
                // Staggered due to this effect".
                let unit = &mut state.units[ctx.actor_index];
                let floor = if effect.hp_floor_one { 1 } else { 0 };
                let applied = damage.min((unit.hp - floor).max(0));
                let hp = (unit.hp - applied).max(floor);
                unit.hp = hp;
                use_ctx.self_damage_taken += applied;
                let name = unit.name.clone();
                state.push_log(
                    "self-damage",
                    format!("{name} took {applied} HP damage from its own Skill"),
                );
                if !effect.no_stagger {
                    check_stagger(state, ctx.actor_index);
                }
            }
            "self_sp_damage" => {
                let mut amount = effect.self_sp_damage.unwrap_or(0);
                if let (Some(lo), Some(hi)) = (effect.self_sp_damage_min, effect.self_sp_damage_max) {
                    amount = lo + state.roll_inclusive(hi - lo);
                }
                if amount == 0 {
                    continue;
                }
                let sanity = state.units[ctx.actor_index].sanity;
                state.units[ctx.actor_index].sanity = sanity.add(-amount);
            }
            "sp_heal" | "heal_hp" | "heal_percent_hp" => {
                let targets = ally_targets(state, ctx, effect);
                let percent = effect.percent.unwrap_or(0);
                let measured = measured_from_condition(
                    effect,
                    &state.units[ctx.actor_index],
                    ctx.target_index.map(|i| &state.units[i]),
                );
                for index in targets {
                    match effect.kind.as_str() {
                        "sp_heal" => {
                            let value = effect.value.unwrap_or(0);
                            let sanity = state.units[index].sanity;
                            state.units[index].sanity = sanity.add(value);
                        }
                        "heal_hp" => {
                            let value = effect.value.unwrap_or(0);
                            state.units[index].heal(value);
                        }
                        _ => {
                            // "Heal N allies with the lowest HP percentages by
                            // (10 + (...)/3)% HP (max 15%)".
                            let _ = percent;
                            let gain = scaled(effect, measured);
                            let heal = state.units[index].max_hp * gain / 100;
                            state.units[index].heal(heal);
                        }
                    }
                }
            }
            "heal_from_status" => {
                let Some(status) = effect.heal_from_status.clone() else { continue };
                let divisor = effect.heal_from_divisor.unwrap_or(1).max(1);
                let cap = effect.max.unwrap_or(i32::MAX);
                let amount = ctx
                    .target_index
                    .map(|index| {
                        unit_status_value(&state.units[index], &status, effect.heal_from_component)
                            / divisor
                    })
                    .unwrap_or(0)
                    .min(cap);
                let targets = ally_targets(state, ctx, effect);
                for index in targets {
                    let sanity = state.units[index].sanity;
                    state.units[index].sanity = sanity.add(amount);
                }
            }
            "refund_consumed_status" => {
                let Some(status) = effect.status.clone() else { continue };
                let percent = effect.refund_percent.unwrap_or(50);
                let consumed = use_ctx
                    .consumed_by_status
                    .get(&status)
                    .copied()
                    .unwrap_or_else(|| {
                        if use_ctx.consumed_status > 0 {
                            use_ctx.consumed_status
                        } else {
                            0
                        }
                    });
                let refund = consumed * percent / 100;
                if refund > 0 {
                    state.units[ctx.actor_index].statuses.add_potency(&status, refund);
                }
            }
            "amplitude_conversion" => {
                let Some(index) = ctx.target_index else { continue };
                let into = effect
                    .amplitude_into
                    .clone()
                    .unwrap_or_else(|| "Tremor".to_string());
                let unit = &mut state.units[index];
                unit.status_markers
                    .retain(|marker| !marker.starts_with(AMPLITUDE_CONVERSION));
                unit.status_markers
                    .push(format!("{AMPLITUDE_CONVERSION}: {into}"));
                let name = unit.name.clone();
                state.push_log(
                    "amplitude",
                    format!("{name} entered [Amplitude Conversion] into [{into}]"),
                );
            }
            "amplitude_entanglement" => {
                let Some(index) = ctx.target_index else { continue };
                let into = effect
                    .amplitude_into
                    .clone()
                    .unwrap_or_else(|| "Tremor".to_string());
                let unit = &mut state.units[index];
                unit.status_markers
                    .retain(|marker| !marker.starts_with(AMPLITUDE_ENTANGLEMENT));
                unit.status_markers
                    .push(format!("{AMPLITUDE_ENTANGLEMENT}: {into}"));
            }
            "compound" => {
                // A wiki line that carries several effects ("... ; then ...").
                // The line's own "next turn" applies to every sub-effect.
                let mut subs = effect.sub_effects.clone();
                if effect.next_turn {
                    for sub in subs.iter_mut() {
                        sub.next_turn = true;
                    }
                }
                apply_effects(state, &subs, ctx, use_ctx);
            }
            "base_power_from_consumed" => {
                // "Base Power +1 for every 5 Stack consumed".
                let per = effect.per.unwrap_or(1).max(1);
                let step = effect.step.unwrap_or(1);
                use_ctx.base_power_bonus += (use_ctx.consumed_status / per) * step;
            }
            "damage_percent_from_missing_sp" => {
                // "deal more damage the further their SP value is from 0".
                let step = effect.step.unwrap_or(0);
                let max = effect.max.unwrap_or(i32::MAX);
                let missing = ctx
                    .target_index
                    .map(|index| 0 - state.units[index].sanity.sp())
                    .unwrap_or(0)
                    .max(0);
                use_ctx.damage_bonus += (step * missing).min(max) as f64 / 100.0;
            }
            "damage_percent_per_resonance" => {
                // "At 2+ Gloom Reson., deal +8% damage for every Gloom Reson."
                let step = effect.step.unwrap_or(0);
                let max = effect.max.unwrap_or(i32::MAX);
                let reson = match effect.resonance_of.as_deref() {
                    Some(sin) => state.units[ctx.actor_index]
                        .resonance_of
                        .get(sin)
                        .copied()
                        .unwrap_or(0),
                    None => state.units[ctx.actor_index].resonance_max,
                };
                let per = effect.per.unwrap_or(1).max(1);
                use_ctx.damage_bonus += ((reson / per) * step).min(max) as f64 / 100.0;
            }
            "damage_percent_from_resist" => {
                // "If the main target has higher than N Sin Resist., deal +X%
                // damage for every 0.1 excess Resist. (max Y%)".
                let Some(index) = ctx.target_index else { continue };
                let base = effect.value.unwrap_or(10) as f64 / 10.0;
                let resist = state.units[index].resist_sin(Sin::Gloom);
                let excess = (resist - base).max(0.0);
                let step = effect.step.unwrap_or(0);
                let max = effect.max.unwrap_or(i32::MAX);
                let percent = ((excess * 10.0).floor() as i32 * step).min(max);
                use_ctx.damage_bonus += percent as f64 / 100.0;
            }
            "gain_up_to_with_self_damage" => {
                // "Gain [Lamp] up to 8 Stack; for every Stack gained, take HP
                // damage equal to 1% of max HP (does not reduce HP below 1)".
                let Some(status) = effect.status.clone() else { continue };
                let cap = effect.up_to.unwrap_or(0);
                let percent = effect.self_damage_percent.unwrap_or(0);
                let (gained, max_hp) = {
                    let unit = &mut state.units[ctx.actor_index];
                    let have = unit.statuses.stack(&status);
                    let gained = (cap - have).max(0);
                    unit.statuses.add_stack(&status, gained);
                    (gained, unit.max_hp)
                };
                let damage = (max_hp * percent / 100) * gained;
                if damage > 0 {
                    let unit = &mut state.units[ctx.actor_index];
                    let floor = if effect.hp_floor_one { 1 } else { 0 };
                    unit.hp = (unit.hp - damage).max(floor);
                    let name = unit.name.clone();
                    state.push_log(
                        "self-damage",
                        format!("{name} took {damage} HP damage for {gained} [{status}] Stack"),
                    );
                }
            }
            "reuse_coin" => {
                // "Then, Reuse this Coin (N times per Skill)" - replayed by the
                // attack loop; "(if the sum of HP lost due to this effect is
                // less than N)" stops the repeats early.
                if let Some(times) = effect.reuse_coin {
                    // "(4 times per Skill)" is a per-Skill cap on that Coin, not
                    // a budget that the reuse itself may extend.
                    let allowed = match effect.threshold {
                        Some(limit) => use_ctx.self_damage_taken < limit,
                        None => true,
                    };
                    // "Reuse this Coin ([Bright -光-] Potency - 1) times (4 times
                    // max)": the status sets how many repeats are available.
                    let times = match effect.reuse_from_status.as_deref() {
                        Some(status) => {
                            let have = state.units[ctx.actor_index].statuses.potency(status);
                            (have - effect.reuse_minus.unwrap_or(0)).clamp(0, times)
                        }
                        None => times,
                    };
                    // `coin_index` is 1-based, matching `coin_hits`.
                    let coin = effect.coin_index.unwrap_or(0);
                    if allowed && times > 0 {
                        let entry = use_ctx.reuse_caps.entry(coin).or_insert(0);
                        *entry = (*entry).max(times);
                    }
                }
            }
            "heal_per_coin_hits" => {
                // "Heal (# of Coin 3 hits x 2) SP": the hits of the Coin the
                // clause belongs to, not every Coin of the Skill.
                let per = effect.value.unwrap_or(0);
                let hits = match effect.coin_index {
                    Some(coin) => use_ctx.coin_hits.get(&coin).copied().unwrap_or(0),
                    None => use_ctx.coin_hits.values().copied().sum::<i32>(),
                };
                let amount = per * hits;
                for index in ally_targets(state, ctx, effect) {
                    let sanity = state.units[index].sanity;
                    state.units[index].sanity = sanity.add(amount);
                }
            }
            "combat_end_sp_damage" => {
                // "[Attack End] For 2 turns, lose 8 SP at Combat End".
                let amount = effect.value.unwrap_or(0);
                let turns = effect.turns.unwrap_or(1);
                if amount != 0 && turns > 0 {
                    state.units[ctx.actor_index]
                        .combat_end_sp_loss
                        .push((amount, turns));
                }
            }
            "inflict_on_random_other" => {
                // "inflict N [X] against a random non-targeted enemy".
                let Some(status) = effect.status.clone() else { continue };
                let potency = effect.potency.unwrap_or(0);
                let actor_is_sinner = state.units[ctx.actor_index].kind.is_sinner();
                let mut pool: Vec<usize> = state
                    .units
                    .iter()
                    .enumerate()
                    .filter(|(index, unit)| {
                        unit.alive
                            && *index != ctx.actor_index
                            && unit.kind.is_sinner() != actor_is_sinner
                            && Some(*index) != ctx.target_index
                    })
                    .map(|(index, _)| index)
                    .collect();
                if pool.is_empty() {
                    pool = state
                        .units
                        .iter()
                        .enumerate()
                        .filter(|(index, unit)| {
                            unit.alive && *index != ctx.actor_index && Some(*index) != ctx.target_index
                        })
                        .map(|(index, _)| index)
                        .collect();
                }
                if pool.is_empty() {
                    continue;
                }
                let pick = state.rng.below(pool.len() as u32) as usize;
                let target = pool[pick];
                state.units[target].statuses.add_potency(&status, potency);
            }
            "suit_convert" => {
                // "[Combat Start] convert the Suit in this unit's Hand to a
                // random Suit that corresponds to one of this unit's Base Attack
                // Skills" (wiki.gg `Status Effects` / Hand - Suit).
                let suits = ["HanafudaOne", "HanafudaTwo", "HanafudaThree"];
                let pick = state.rng.below(suits.len() as u32) as usize;
                state.units[ctx.actor_index].suit = Some(suits[pick].to_string());
            }
            "sub_target_damage" => {
                use_ctx.sub_target_damage_percent = effect.percent.unwrap_or(0);
            }
            "tag" => {
                // Tags are lifted to Skill tags by the extractor; a stray one
                // carries no value of its own.
            }
            "flag" => {
                // A named switch a phase turns on for this Skill use, e.g.
                // "[Clash Lose] Target cannot be Staggered until Attack End".
                match effect.flag.as_deref() {
                    Some("no_stagger_target") => use_ctx.no_stagger_target = true,
                    other => ctx
                        .mechanics_note
                        .push(format!("unknown flag `{other:?}` ({:?})", effect.raw)),
                }
            }
            "bonus_damage_percent_of_coin" => {
                // Resolved in `apply_hit`, where the Coin's final damage is known.
            }
            "noop" => {}
            other => {
                ctx.mechanics_note
                    .push(format!("unhandled effect kind `{other}` ({:?})", effect.raw));
            }
        }
        // The clause resolved, so its per-turn / per-encounter allowance is spent
        // now (and not when its condition merely failed).
        for key in usage_grants {
            let used = state.units[ctx.actor_index]
                .turn_effect_usage
                .get(&key)
                .copied()
                .unwrap_or(0);
            state.units[ctx.actor_index]
                .turn_effect_usage
                .insert(key, used + 1);
        }
    }
}

/// Read the value a condition measured on the actor/target.
fn measured_from_condition(
    effect: &Effect,
    actor: &Unit,
    target: Option<&Unit>,
) -> i32 {
    let Some(cond) = &effect.condition else { return 0 };
    let unit = if cond.source.as_deref() == Some("target") {
        match target {
            Some(t) => t,
            None => return 0,
        }
    } else {
        actor
    };
    if !cond.statuses.is_empty() {
        return sum_statuses(unit, &cond.statuses, cond.component);
    }
    cond.status
        .as_deref()
        .map(|s| unit_status_value(unit, s, cond.component))
        .unwrap_or(0)
}

// --------------------------------------------------------------------------- //
// power / coins
// --------------------------------------------------------------------------- //

pub fn heads_percent(state: &BattleState, unit: &Unit) -> i32 {
    let _ = state;
    unit.sanity.heads_percent()
}



// --------------------------------------------------------------------------- //
// actions
// --------------------------------------------------------------------------- //

pub fn pending_uses(state: &BattleState, library: &Library, mechanics: &MechanicsBook) -> Vec<SkillUse> {
    let mut uses = Vec::new();
    for action in &state.actions {
        let Some(unit_index) = state.index_of(&action.actor) else { continue };
        if let Some(use_) = build_use(state, library, mechanics, unit_index, &action.skill) {
            uses.push(use_);
        }
    }
    let _ = EMPTY_SKILL;
    uses
}

/// Build a runtime skill use for an E.G.O skill.
pub fn build_ego_use(
    state: &BattleState,
    library: &Library,
    mechanics: &MechanicsBook,
    unit_index: usize,
    ego_id: &EgoId,
    kind: EgoSkillKind,
) -> Option<SkillUse> {
    let unit = &state.units[unit_index];
    let record = library.ego(ego_id)?;
    let (skill, key) = match kind {
        EgoSkillKind::Awakening => (record.awakening.as_ref(), "awakening"),
        EgoSkillKind::Corrosion | EgoSkillKind::Overclock => {
            (record.corrosion.as_ref().or(record.awakening.as_ref()), "corrosion")
        }
    };
    let skill = skill?;
    let mech = mechanics.get_ego(ego_id.as_str(), key);
    let coins = (0..skill.coins.unwrap_or(1))
        .map(|index| {
            CoinRuntime::fresh(
                mech.coin(index + 1)
                    .iter()
                    .any(|e| e.kind == "unbreakable_coin"),
            )
        })
        .collect();
    Some(SkillUse {
        actor: unit.id.clone(),
        target: None,
        slot: 0,
        skill: SkillId::new(ego_id.as_str().to_string()),
        name: format!("{} [{}]", skill.name.clone().unwrap_or_else(|| ego_id.to_string()), key),
        sin: skill.sin.as_deref().and_then(Sin::parse).unwrap_or(Sin::Wrath),
        damage_type: skill
            .kind
            .as_deref()
            .and_then(DamageType::parse)
            .unwrap_or(DamageType::Blunt),
        base_power: skill.base_power.unwrap_or(0),
        coin_power: skill.coin_power.unwrap_or(0),
        offense_level_mod: skill.offense_level_mod.unwrap_or(0),
        defense_level_mod: None,
        attack_weight: skill.attack_weight.unwrap_or(1),
        coins,
        mechanics: mech,
        ctx: UseContext::default(),
        is_defense: false,
        is_ego: true,
        ego: Some(ego_id.clone()),
    })
}

/// Build a runtime skill use for a unit.
pub fn build_use(
    state: &BattleState,
    library: &Library,
    mechanics: &MechanicsBook,
    unit_index: usize,
    skill: &SkillId,
) -> Option<SkillUse> {
    let unit = &state.units[unit_index];
    let UnitKind::Sinner { identity } = &unit.kind else {
        if let UnitKind::Abnormality { enemy, .. } = &unit.kind {
            return build_enemy_use(state, library, mechanics, unit_index, enemy.as_str(), skill);
        }
        return None;
    };
    let record = library.identity(identity)?;
    let skill_record = record.skills.iter().find(|s| s.id == skill.0)?;
    let tier = skill_record.tier(state.config.uptie)?;
    let mech = mechanics.get_or_default_for(skill, state.config.uptie);
    let coins = (0..tier.coins.unwrap_or(1))
        .map(|index| CoinRuntime::fresh(mech.coin(index + 1).iter().any(|e| e.kind == "unbreakable_coin")))
        .collect();
    Some(SkillUse {
        actor: unit.id.clone(),
        target: None,
        slot: 0,
        skill: skill.clone(),
        name: skill_record.display_name(),
        sin: skill_record.sin(state.config.uptie).unwrap_or(Sin::Wrath),
        damage_type: skill_record.damage_type(state.config.uptie).unwrap_or(DamageType::Blunt),
        base_power: tier.base_power.unwrap_or(0),
        coin_power: tier.coin_power.unwrap_or(0),
        offense_level_mod: tier.offense_level_mod.unwrap_or(0),
        defense_level_mod: tier.defense_level_mod,
        attack_weight: tier.attack_weight.unwrap_or(1),
        coins,
        mechanics: mech,
        ctx: UseContext::default(),
        is_defense: skill_record.slot() == Some(crate::ids::SkillSlot::Defense),
        is_ego: false,
        ego: None,
    })
}

fn build_enemy_use(
    state: &BattleState,
    library: &Library,
    mechanics: &MechanicsBook,
    unit_index: usize,
    enemy_id: &str,
    skill: &SkillId,
) -> Option<SkillUse> {
    let unit = &state.units[unit_index];
    let record = library.enemies.get(enemy_id)?;
    let skill_record = record
        .skills
        .iter()
        .find(|s| s.skill_id() == skill.0 || s.display_name() == skill.0)?;
    let mech = mechanics
        .get_for(skill, state.config.uptie)
        .cloned()
        .or_else(|| mechanics.get(skill).cloned())
        .unwrap_or_default();
    let coins = (0..skill_record.coins.unwrap_or(1))
        .map(|index| CoinRuntime::fresh(mech.coin(index + 1).iter().any(|e| e.kind == "unbreakable_coin")))
        .collect();
    Some(SkillUse {
        actor: unit.id.clone(),
        target: None,
        slot: 0,
        skill: skill.clone(),
        name: skill_record.display_name(),
        sin: skill_record.sin().unwrap_or(Sin::Wrath),
        damage_type: skill_record.damage_type().unwrap_or(DamageType::Blunt),
        base_power: skill_record.base_power.unwrap_or(0),
        coin_power: skill_record.coin_power.unwrap_or(0),
        offense_level_mod: skill_record.offense_level_mod.unwrap_or(0),
        defense_level_mod: None,
        attack_weight: skill_record.attack_weight.unwrap_or(1),
        coins,
        mechanics: mech,
        ctx: UseContext::default(),
        is_defense: false,
        is_ego: false,
        ego: None,
    })
}

// --------------------------------------------------------------------------- //
// clash + attack
// --------------------------------------------------------------------------- //

/// Toss every coin that is still in play.  Heads chance comes from the unit's
/// Sanity (`H = 50 + SP`).  Paralyze sets a Coin's power to 0 for one toss and is
/// consumed per tossed coin (Japanese wiki `戦闘システム詳細`, `麻痺`).
fn toss_all(state: &mut BattleState, unit_index: usize, use_: &mut SkillUse) {
    let percent = heads_percent(state, &state.units[unit_index]);
    for index in 0..use_.coins.len() {
        if use_.coins[index].state != CoinState::Fresh {
            continue;
        }
        let heads = state.flip(percent);
        let paralyze = state.units[unit_index].statuses.potency("Paralyze");
        use_.coins[index].heads = Some(heads);
        use_.coins[index].paralyzed = paralyze > 0;
        if paralyze > 0 {
            state.units[unit_index].statuses.add_potency("Paralyze", -1);
        }
    }
}

/// Effective coin power of one coin, including `Coin Power +N` modifiers and
/// Paralyze.  Paralyze is consumed once per tossed coin.
/// Do a Coin clause's own gates allow it?  `[Hit after Clash Lose]` needs a lost
/// Clash, `[On Hit without Cracking]` a Coin the Clash did not destroy, and any
/// written condition is evaluated against the current units.
fn coin_clause_applies(
    state: &BattleState,
    attacker_index: usize,
    defender_index: usize,
    use_: &SkillUse,
    effect: &Effect,
    coin_cracked: bool,
    clash_count: i32,
) -> bool {
    if effect.only_after_clash_lose && !use_.ctx.lost_clash {
        return false;
    }
    if effect.only_without_cracking && coin_cracked {
        return false;
    }
    match &effect.condition {
        Some(condition) => condition_holds(
            condition,
            &state.units[attacker_index],
            Some(&state.units[defender_index]),
            clash_count,
            use_.slot,
        ),
        None => true,
    }
}

/// Clauses of a Coin that act on the unit being hit (stati, activations,
/// target damage).  An Atk Weight splash runs these for every extra target, but
/// the Skill's own bookkeeping happens only with the main target.
fn is_target_facing_clause(effect: &Effect) -> bool {
    matches!(
        effect.kind.as_str(),
        "inflict"
            | "activate_status"
            | "tremor_burst"
            | "lose_status_count"
            | "gloom_damage_equal_target_status"
            | "damage_from_status_divisor"
            | "consume_status_to_inflict"
            | "convert_status"
    )
}

/// Clauses that only raise the damage of the Coin they are written on.
///
/// They have to be resolved **before** that Coin's damage is computed; the rest
/// of the Coin's clauses (status inflictions, activations) resolve after the hit
/// (wiki.gg `Battles`: a Coin's damage is final before its On Hit effects).
fn is_coin_damage_clause(effect: &Effect) -> bool {
    matches!(
        effect.kind.as_str(),
        "damage_percent"
            | "damage_percent_from_resist"
            | "damage_percent_per_resonance"
            | "damage_percent_per_ammo_spent"
            | "damage_percent_per_consumed_status"
            | "damage_percent_missing_hp"
            | "damage_percent_from_negative_sp"
            | "damage_percent_from_missing_sp"
            | "damage_percent_by_missing_hp"
            | "damage_percent_per_stack"
            | "damage_percent_per_coin_heads"
    )
}

fn effective_coin_power(
    _state: &mut BattleState,
    _unit_index: usize,
    use_: &SkillUse,
    coin_index: usize,
) -> i32 {
    let coin = &use_.coins[coin_index];
    if coin.paralyzed {
        // "麻痺": the Coin contributes no Coin Power.
        return 0;
    }
    let boost = use_.ctx.coin_power_bonus
        + if use_.coin_power > 0 {
            // Plus Coin Boost raises plus coins, Minus Coin Drop lowers minus coins.
            use_.ctx.coin_power_boost
        } else {
            -use_.ctx.coin_power_drop
        };
    if coin.state == CoinState::Cracked {
        // "破壊不能コインはマッチで破壊されると**コイン威力が1になる**。ただし、
        // 元のコイン威力が1になるだけで、後からコイン威力増加が適用される"
        // (JA-wiki 破壊不能コイン): the destroyed Unbreakable Coin's own Coin
        // Power becomes 1, then the Coin Power increases still apply.
        return 1 + boost;
    }
    use_.coin_power + boost
}

/// **Final Power** of a skill: Base Power once, plus the Coin Power of every
/// coin that landed Heads.  Source: wiki.gg `Battles` ("both units toss all of
/// their Skill's Coins; this determines the Skill's power in a Clash") and the
/// Japanese wiki `戦闘システム詳細` ("両者が自分のスキルにある全てのコインを投げて
/// 最終威力を決定。コインの表裏によって基本威力+コイン威力の値になる").
pub fn final_power(
    state: &mut BattleState,
    unit_index: usize,
    use_: &mut SkillUse,
) -> i32 {
    let mut total = power_base(state, unit_index, use_);
    for index in 0..use_.coins.len() {
        if use_.coins[index].state != CoinState::Fresh {
            continue;
        }
        if use_.coins[index].heads.unwrap_or(false) {
            total += effective_coin_power(state, unit_index, use_, index);
        }
    }
    total.max(0)
}

/// The part of a Skill's power that is not tied to individual Coins: Base Power,
/// its bonuses, the time-state / signature passives and the generic
/// `Power Up/Down` / `Attack Power Up/Down` statuses.
///
/// Both the Clash's Final Power and the accumulating attack power start here, so
/// a modifier cannot show up in one and be missing from the other.
fn power_base(state: &mut BattleState, unit_index: usize, use_: &SkillUse) -> i32 {
    let mut total = use_.base_power + use_.ctx.base_power_bonus;
    if let Some((_, bonus)) = time_state_bonus(state, unit_index) {
        total += bonus.final_power;
    }
    total += time_passives::signature_final_power(state, unit_index, &use_.skill);
    // Generic skill-power statuses (wiki.gg `Status Effects`):
    // `Power Up/Down` affect every skill, `Attack Power Up/Down` only attacks.
    {
        let statuses = &state.units[unit_index].statuses;
        let all = statuses.count("Power Up") - statuses.potency("Power Down");
        let attack = if use_.is_defense {
            // A defense Skill reads Defense Power Up/Down instead of the attack
            // variants ("Defense Power Up: +1 Defense Skill Power").
            statuses.count("Defense Power Up") - statuses.potency("Defense Power Down")
        } else {
            statuses.count("Attack Power Up") - statuses.potency("Attack Power Down")
        };
        total += all + attack;
    }
    total
}

/// The Past / Present / Future passives of the Imago.
///
/// Source: wiki.gg `Butterfly of Entangled Lives::Imago` passives
/// `Past [過去]`, `Present [現在]`, `Future [未來]`.
pub mod time_passives {
    use super::*;

    /// Is `skill` the state-exclusive "big" skill of the unit's active state?
    pub fn is_signature(state: &BattleState, unit_index: usize, skill: &SkillId) -> bool {
        let unit = &state.units[unit_index];
        let Some(active) = unit.time_state else { return false };
        unit.time_signature
            .iter()
            .any(|(time_state, id)| *time_state == active && id == skill.as_str())
    }

    /// Signature skills gain Final Power +2 in their own state.
    pub fn signature_final_power(state: &BattleState, unit_index: usize, skill: &SkillId) -> i32 {
        if is_signature(state, unit_index, skill) {
            2
        } else {
            0
        }
    }

    /// Past: "When hit by Sinners, inflict 1 Burn and +1 Burn Count on the
    /// attacking Sinner."  Disabled when the earlier stations cut that component.
    pub fn on_hit_by_sinner(state: &mut BattleState, enemy_index: usize, attacker_index: usize) {
        if state.units[enemy_index].time_state != Some(crate::scripts::TimeState::Past) {
            return;
        }
        if state.campaign.is_disabled("past:burn_on_hit") {
            return;
        }
        if !state.units[attacker_index].kind.is_sinner() {
            return;
        }
        let statuses = &mut state.units[attacker_index].statuses;
        statuses.add_potency("Burn", 1);
        statuses.add_count("Burn", 1);
    }

    /// Turn-start effects of the active state.
    pub fn turn_start(state: &mut BattleState, unit_index: usize) {
        let Some(active) = state.units[unit_index].time_state else { return };
        let stack = state.units[unit_index].statuses.stack(active.stack_key());
        let sinner_count = state
            .units
            .iter()
            .filter(|u| u.alive && u.kind.is_sinner())
            .count() as i32;
        match active {
            crate::scripts::TimeState::Past => {
                if state.campaign.is_disabled("past:turn_start") {
                    let _ = stack;
                    return;
                }
                // "Turn Start: Inflict 5 HP Healing Down and 3 Wrath Fragility
                // on all Sinners who have 10+ (Burn Potency + Burn Count)."
                for index in 0..state.units.len() {
                    if !state.units[index].kind.is_sinner() || !state.units[index].alive {
                        continue;
                    }
                    let burn = state.units[index].statuses.total_of("Burn");
                    if burn >= 10 {
                        state.units[index].statuses.add_potency("HP Healing Down", 5);
                        state.units[index].statuses.add_potency("Wrath Fragility", 3);
                    }
                }
                let _ = stack;
            }
            crate::scripts::TimeState::Present => {
                // "Turn Start: gain (2 + # of Sinners) Poise Potency and Count."
                let gain = 2 + sinner_count;
                state.units[unit_index].statuses.add_potency("Poise", gain);
                state.units[unit_index].statuses.add_count("Poise", gain);
            }
            crate::scripts::TimeState::Future => {
                // "Turn Start: All Sinners gain +(3 + # of current turn / 2)
                // Bleed Count (rounded up)."
                let gain = 3 + (state.turn as i32 + 1) / 2;
                for index in 0..state.units.len() {
                    if state.units[index].kind.is_sinner() && state.units[index].alive {
                        state.units[index].statuses.add_count("Bleed", gain);
                    }
                }
            }
        }
    }

    /// Last-coin effects of the signature skills.
    pub fn signature_last_coin(
        state: &mut BattleState,
        enemy_index: usize,
        skill: &SkillId,
        coin_index: usize,
        coin_count: usize,
        target_index: usize,
    ) {
        if coin_index + 1 != coin_count || !is_signature(state, enemy_index, skill) {
            return;
        }
        let Some(active) = state.units[enemy_index].time_state else { return };
        let stack = state.units[enemy_index].statuses.stack(active.stack_key());
        match active {
            crate::scripts::TimeState::Past => {
                if state.campaign.is_disabled("past:kalpagni") {
                    return;
                }
                // Kalpāgni's final Coin: SP damage equal to half the Stack.
                let sanity = state.units[target_index].sanity;
                state.units[target_index].sanity = sanity.add(-(stack / 2));
            }
            crate::scripts::TimeState::Present => {
                // Smite the Wicked's final Coin: raise the Stagger Threshold.
                let raise = stack / 2;
                state.units[target_index]
                    .stagger
                    .shift_first_threshold(raise);
            }
            crate::scripts::TimeState::Future => {
                // Bloodflower's final Coin: heal (Stack x 3) HP on self.
                let heal = stack * 3;
                state.units[enemy_index].heal(heal);
            }
        }
    }

    /// Present: critical hits deal extra damage based on the unit's Poise.
    pub fn crit_damage_bonus(state: &BattleState, unit_index: usize) -> f64 {
        if state.units[unit_index].time_state != Some(crate::scripts::TimeState::Present) {
            return 0.0;
        }
        let poise = &state.units[unit_index].statuses;
        let bonus = ((poise.potency("Poise") + poise.count("Poise")) * 2).min(120) as f64 / 100.0;
        bonus
    }

    /// Present: "Does not lose Poise Count On Crit."
    pub fn keeps_poise_on_crit(state: &BattleState, unit_index: usize) -> bool {
        state.units[unit_index].time_state == Some(crate::scripts::TimeState::Present)
    }

    /// Future: Bleed damage heals the unit that has the state active.
    pub fn bleed_lifesteal_target(state: &BattleState) -> Option<usize> {
        state.units.iter().position(|u| {
            u.alive && u.time_state == Some(crate::scripts::TimeState::Future)
        })
    }
}

/// Bonus the unit's active state of time grants (wiki.gg `Bufs_Refraction6`).
pub fn time_state_bonus(
    state: &BattleState,
    unit_index: usize,
) -> Option<(&'static str, crate::scripts::StackBonus)> {
    let unit = &state.units[unit_index];
    let time_state = unit.time_state?;
    let stack = unit.statuses.stack(time_state.stack_key());
    Some((time_state.boosted_status(), crate::scripts::TimeState::stack_bonus(stack)))
}

/// **Match Power** = Final Power + level bonus.  Only the side with the higher
/// skill level gains `+1 per 3 levels difference`; resistances, defense level,
/// attack type and sin affinity never participate in a clash.
pub fn match_power(
    state: &mut BattleState,
    unit_index: usize,
    opponent_index: usize,
    use_: &mut SkillUse,
    opponent_use: &SkillUse,
) -> i32 {
    let power = final_power(state, unit_index, use_);
    // "お互いの攻撃スキルの攻撃レベルを参照し、差がある場合は差3ごとに高い方に
    // マッチ威力+1の補正がかかる" (JA-wiki 威力): a Clash compares both Skills'
    // **attack (Offense) Levels**, including each Skill's own level modifier -
    // the opponent's Defense Level plays no part here.
    let my_level = state.units[unit_index].offense_level() + use_.offense_level_mod;
    let their_level =
        state.units[opponent_index].offense_level() + opponent_use.offense_level_mod;
    let mut total = power + level_clash_bonus(my_level, their_level);
    // "Clash Power +N" clauses of the Skill itself ("[On Use] At 10+ [X],
    // Clash Power +1").
    total += use_.ctx.clash_power_bonus;
    total += state.units[unit_index].statuses.count("Clash Power Up");
    if let Some((_, bonus)) = time_state_bonus(state, unit_index) {
        total += bonus.clash_power;
    }
    // "Causality that Threads the Past, the Present, and the Future":
    // from 10+ clashes against the same target, Clash Power swings randomly.
    if let Some(swing) = clash_count_swing(state, unit_index, opponent_index) {
        let key = clash_key(state, unit_index, opponent_index);
        let count = state.clash_counts.get(&key).copied().unwrap_or(0);
        if count >= swing.threshold {
            let amount = count / swing.divisor.max(1);
            let positive = state.flip(50);
            total += if positive { amount } else { -amount };
        }
    }
    total
}

/// A Clash is counted per **pair** of units, so the key is canonical: whether
/// the Sinner or the enemy moved first must not change it.
fn clash_key(state: &BattleState, a: usize, b: usize) -> String {
    let first = state.units[a].id.to_string();
    let second = state.units[b].id.to_string();
    if first <= second {
        format!("{first}|{second}")
    } else {
        format!("{second}|{first}")
    }
}

fn clash_count_swing(
    state: &BattleState,
    unit_index: usize,
    _opponent_index: usize,
) -> Option<crate::scripts::ClashCountSwing> {
    state.units[unit_index].clash_count_swing
}

/// Resolve a clash.
///
/// Per round both sides toss **all** of their remaining coins and compare Match
/// Power.  The lower side destroys **its first remaining coin**; on a tie
/// nothing is destroyed and the clash continues.  When one side has no fresh
/// coins left the other side is the winner and attacks with what remains.
pub fn resolve_clash(
    state: &mut BattleState,
    a_index: usize,
    b_index: usize,
    a: &mut SkillUse,
    b: &mut SkillUse,
) -> ClashResult {
    let outcome = resolve_clash_inner(state, a_index, b_index, a, b);
    // "[When Clash ends]" clauses of the statuses both units hold (Rodion's
    // Blessing / Despair inflict [Sinking] once per turn when a Clash ends).
    apply_status_event(state, a_index, "clash_end", Some(b_index));
    apply_status_event(state, b_index, "clash_end", Some(a_index));
    outcome
}

fn resolve_clash_inner(
    state: &mut BattleState,
    a_index: usize,
    b_index: usize,
    a: &mut SkillUse,
    b: &mut SkillUse,
) -> ClashResult {
    let mut rounds = 0;
    loop {
        if !a.has_fresh_coins() || !b.has_fresh_coins() {
            break;
        }
        toss_all(state, a_index, a);
        toss_all(state, b_index, b);
        // "[コイン判定時] 出血" - Bleed triggers when a Coin is tossed, and unlike
        // Poise it also triggers **during a Clash** (JA-wiki ダメージ/スキル等効果
        // のタイミング一覧: "出血と呼吸で扱いが違う(マッチ中に発動する/しない)").
        // A Skill's "While Clashing with this Skill, the main target's [Bleed]
        // Count does not drop below 1" protects its opponent.
        let floor_on_a = b.ctx.status_count_floor.iter().any(|status| status == "Bleed");
        let floor_on_b = a.ctx.status_count_floor.iter().any(|status| status == "Bleed");
        tick_bleed_with_floor(state, a_index, floor_on_a);
        tick_bleed_with_floor(state, b_index, floor_on_b);
        // A Clash whose participant died to Bleed ends there.
        if !state.units[a_index].alive || !state.units[b_index].alive {
            break;
        }
        let a_power = match_power(state, a_index, b_index, a, b);
        let b_power = match_power(state, b_index, a_index, b, a);
        rounds += 1;
        if a_power > b_power {
            destroy_first_coin(&mut b.coins, b_index);
        } else if b_power > a_power {
            destroy_first_coin(&mut a.coins, a_index);
        }
        // Tie: nothing is destroyed, the clash continues with fresh tosses.
        if rounds > 128 {
            break;
        }
    }
    let winner = match (a.has_fresh_coins(), b.has_fresh_coins()) {
        (true, false) => Some(a.actor.clone()),
        (false, true) => Some(b.actor.clone()),
        _ => None,
    };
    ClashResult {
        winner,
        rounds,
        attacker_coins_left: a.remaining_coins(),
        defender_coins_left: b.remaining_coins(),
    }
}

/// Destroy the first coin that is still in play (Japanese wiki: "低い方のコインを
/// 1コイン目から順に一つ破壊").
fn destroy_first_coin(coins: &mut [CoinRuntime], _unit_index: usize) {
    if let Some(coin) = coins.iter_mut().find(|c| c.state == CoinState::Fresh) {
        break_coin(coin, 0);
    }
}

fn break_coin(coin: &mut CoinRuntime, _unit_index: usize) {
    coin.state = if coin.unbreakable {
        CoinState::Cracked
    } else {
        CoinState::Destroyed
    };
}

/// Bleed ticks when a unit tosses an attack coin (wiki.gg `Status Effects`).
fn tick_bleed(state: &mut BattleState, unit_index: usize) {
    tick_bleed_with_floor(state, unit_index, false)
}

/// Bleed ticks when an attack coin is tossed.  `keep_count` honours a skill's
/// "the main target's Bleed Count does not drop below 1" clause.
fn tick_bleed_with_floor(state: &mut BattleState, unit_index: usize, keep_count: bool) {
    let potency = state.units[unit_index].statuses.potency("Bleed");
    if potency <= 0 {
        return;
    }
    let (_, hp_lost) = state.units[unit_index].take_damage(potency);
    let count = state.units[unit_index].statuses.count("Bleed");
    if !(keep_count && count <= 1) {
        // "Then, reduce its Count by 1" - reaching 0 removes the status.
        tick_status(state, unit_index, "Bleed", 0, -1);
    }
    // Future passive: "Whenever Bleed activates on self or on Sinners, heal HP
    // equal to the said Bleed damage."
    if let Some(healer) = time_passives::bleed_lifesteal_target(state) {
        if healer != unit_index {
            state.units[healer].heal(hp_lost);
        }
    }
}

/// One-sided attack with every remaining (fresh) coin.
///
/// The skill's power accumulates as the coins are used: start from Base Power,
/// add Coin Power for every Heads coin, and deal damage equal to the current
/// accumulated power.  Japanese wiki `戦闘システム詳細`: "コインを1枚目から順に振り、
/// その威力分のダメージを相手に与えていく", with the worked example of a 4+4,
/// three-coin skill dealing 8, 12, 16 (36 total) on all Heads.
pub fn one_sided_attack(
    state: &mut BattleState,
    attacker_index: usize,
    mut defender_index: usize,
    use_: &mut SkillUse,
    clash_count: i32,
) -> Vec<HitResult> {
    // An E.G.O's SP cost belongs to the "[Before Attack]" timing: it is paid once
    // the Clash is over and just before the first Coin is tossed.
    if let Some(sp) = use_.ctx.ego_sp_pending.take() {
        pay_ego_sp(state, attacker_index, sp);
    }
    let mut hits = Vec::new();
    let mut order: Vec<usize> = (0..use_.coins.len())
        .filter(|i| use_.coins[*i].state == CoinState::Fresh)
        .collect();
    // Two budgets: the one known before the attack ("Reuse this Coin once for
    // every N% missing HP" is evaluated On Use) and the one the Coin itself
    // grants while it resolves ("Then, Reuse this Coin (4 times per Skill)").

    loop {
        let Some(coin_index) = order.first().copied() else { break };
        order.remove(0);
        let keep_bleed = use_.ctx.status_count_floor.iter().any(|s| s == "Bleed");
        tick_bleed_with_floor(state, attacker_index, keep_bleed);
        // Bleed (or any other self damage) can kill the attacker mid-Skill: the
        // remaining Coins are then not used.
        if !state.units[attacker_index].alive {
            break;
        }
        if use_.coins[coin_index].heads.is_none() {
            toss_single(state, attacker_index, use_, coin_index);
        }
        let heads = use_.coins[coin_index].heads.unwrap_or(false);
        if use_.ctx.accumulated == 0 {
            // The accumulator starts at the Skill's Base Power (with its
            // modifiers) for the first coin used.
            use_.ctx.accumulated = power_base(state, attacker_index, use_);
        }
        if heads {
            use_.ctx.accumulated += effective_coin_power(state, attacker_index, use_, coin_index);
        }
        let power = use_.ctx.accumulated;
        // "each Coin flips against a random enemy among its targets" - the first
        // Coin keeps the main target (E.G.O Solemn Lament).
        let mut target = defender_index;
        if use_.ctx.random_coin_targets && coin_index > 0 {
            let pool: Vec<usize> = state
                .units
                .iter()
                .enumerate()
                .filter(|(_, unit)| {
                    unit.alive && unit.kind.is_sinner() != state.units[attacker_index].kind.is_sinner()
                })
                .map(|(index, _)| index)
                .collect();
            if pool.len() > 1 {
                target = pool[state.rng.below(pool.len() as u32) as usize];
            }
        }
        let hit = apply_hit(
            state,
            attacker_index,
            target,
            use_,
            coin_index,
            power,
            heads,
            clash_count,
        );
        hits.push(hit);
        // "If this unit runs out of [BulletLament] midway through Skill use,
        // cancel all subsequent Coins and [ReloadLament]" (passive
        // `ISeeTheDyingButterfly.`).
        if use_.ctx.ammo_exhausted {
            if use_
                .ctx
                .ammo_spent_by_status
                .contains_key(AMMO_SOLEMN_LAMENT)
            {
                let detail = reload_solitude(state, attacker_index);
                state.push_log("reload", detail);
            }
            break;
        }
        // "Then, Reuse this Coin (N times per Skill)": the Coin may be used again
        // while it is under its cap, which is granted while the Coin resolves.
        let used_times = *use_.ctx.coin_hits.get(&(coin_index as u32 + 1)).unwrap_or(&1);
        let cap = use_.ctx.reuse_caps.get(&(coin_index as u32 + 1)).copied().unwrap_or(0);
        let under_cap = used_times - 1 < cap;
        // "Reuse this Coin once for every N% missing HP (max K times)".
        let hp_budget = use_
            .ctx
            .reuse_hp
            .map(|(per, max)| {
                ((100 - state.units[attacker_index].hp_percent()) / per).clamp(0, max)
                    - use_.ctx.reuse_hp_used
            })
            .unwrap_or(0)
            .max(0);
        if under_cap || hp_budget > 0 {
            if !under_cap {
                use_.ctx.reuse_hp_used += 1;
            }
            // "The Coin is used again": it is tossed again (JA-wiki 守備スキル
            // "再使用はコインを投げるごとに「スキルを使用した」という判定").
            if let Some(coin) = use_.coins.get_mut(coin_index) {
                coin.state = CoinState::Fresh;
                coin.heads = None;
                coin.paralyzed = false;
            }
            use_.ctx.reuse_uses += 1;
            order.insert(0, coin_index);
            // "通常戦闘の場合、再使用や広域のサブターゲットと同じメカニズムで
            // 対象を自動で選択しなおす" (JA-wiki ダメージ): a Reuse picks a new
            // target once the current one is gone.
            if !state.units[defender_index].alive {
                let next = state
                    .units
                    .iter()
                    .enumerate()
                    .find(|(index, unit)| {
                        unit.alive
                            && *index != attacker_index
                            && unit.kind.is_sinner() != state.units[attacker_index].kind.is_sinner()
                    })
                    .map(|(index, _)| index);
                match next {
                    Some(next) => {
                        defender_index = next;
                    }
                    None => break,
                }
            }
            continue;
        }
        if !state.units[defender_index].alive {
            break;
        }
    }
    // Cracked Unbreakable Coins attack after getting hit (wiki.gg `Clash`):
    // they keep the Skill's Base Power and the Coin Power accumulation, and the
    // Coin keeps the result it was tossed with during the Clash.
    for coin_index in use_.cracked_coins() {
        tick_bleed(state, attacker_index);
        if !state.units[attacker_index].alive {
            break;
        }
        if use_.coins[coin_index].heads.is_none() {
            toss_single(state, attacker_index, use_, coin_index);
        }
        let heads = use_.coins[coin_index].heads.unwrap_or(false);
        if use_.ctx.accumulated == 0 {
            use_.ctx.accumulated = power_base(state, attacker_index, use_);
        }
        if heads {
            use_.ctx.accumulated += effective_coin_power(state, attacker_index, use_, coin_index);
        }
        let power = use_.ctx.accumulated;
        let hit = apply_hit(
            state,
            attacker_index,
            defender_index,
            use_,
            coin_index,
            power,
            heads,
            clash_count,
        );
        hits.push(hit);
        if !state.units[defender_index].alive {
            break;
        }
    }
    hits
}

/// Toss one coin of a skill (used by one-sided attacks).
fn toss_single(
    state: &mut BattleState,
    unit_index: usize,
    use_: &mut SkillUse,
    coin_index: usize,
) {
    let percent = heads_percent(state, &state.units[unit_index]);
    let heads = state.flip(percent);
    let paralyze = state.units[unit_index].statuses.potency("Paralyze");
    use_.coins[coin_index].heads = Some(heads);
    use_.coins[coin_index].paralyzed = paralyze > 0;
    if paralyze > 0 {
        state.units[unit_index].statuses.add_potency("Paralyze", -1);
    }
}



/// Damage-only calculation shared by normal hits and Attack Weight splash hits.
fn compute_hit_damage(
    state: &BattleState,
    attacker_index: usize,
    defender_index: usize,
    use_: &SkillUse,
    power: i32,
    crit: bool,
    clash_count: i32,
) -> i32 {
    let defender = &state.units[defender_index];
    let sin_resist = defender.resist_sin(use_.sin);
    let sin_name = sin_name(use_.sin);
    let stagger_bonus = if defender.is_staggered() {
        Some(defender.stagger.damage_resistance_bonus())
    } else {
        None
    };
    let inputs = DamageInputs {
        coin_roll: power,
        sin_resist,
        damage_type_resist: defender.resist(use_.damage_type),
        stagger_bonus,
        offense_level: state.units[attacker_index].offense_level() + use_.offense_level_mod,
        defense_level: active_defense_level(state, defender_index)
            .unwrap_or_else(|| defender.defense_level()),
        critical: crit,
        clash_count,
        dynamic_modifier: use_.ctx.damage_bonus
            + state.units[attacker_index].outgoing_damage_modifier()
            + incoming_damage_modifier(defender, sin_name)
            + passive_modifiers(state, defender_index, Some(attacker_index)).1
            + if crit {
                time_passives::crit_damage_bonus(state, attacker_index) + use_.ctx.crit_damage_bonus
            } else {
                0.0
            },
        ..Default::default()
    };
    let breakdown = compute_damage(&inputs);
    if use_.ctx.zero_damage {
        0
    } else {
        breakdown.final_damage
    }
}

/// Attack Weight: a Skill with N Attack Weight hits N Slots.  The main target
/// goes through the normal Clash/one-sided path; the remaining Slots take the
/// same Coin Rolls without Clashing (wiki.gg `Clash` / Attack Weight).
fn splash_attack(
    state: &mut BattleState,
    attacker_index: usize,
    main_target: usize,
    use_: &mut SkillUse,
    hits: &[HitResult],
    clash_count: i32,
) {
    let extra = use_.attack_weight.saturating_sub(1) as usize;
    if extra == 0 || hits.is_empty() {
        return;
    }
    let indiscriminate = use_.ctx.indiscriminate;
    let attacker_is_sinner = state.units[attacker_index].kind.is_sinner();
    let defenders: Vec<usize> = state
        .units
        .iter()
        .enumerate()
        .filter(|(index, unit)| {
            if !unit.alive || *index == main_target || *index == attacker_index {
                return false;
            }
            // Random Corrosion re-draws sub-targets indiscriminately; a normal
            // Attack Weight splash only reaches the opposing side.
            if indiscriminate {
                return true;
            }
            unit.kind.is_sinner() != attacker_is_sinner
        })
        .map(|(index, _)| index)
        .collect();
    for target in defenders.into_iter().take(extra) {
        for hit in hits {
            if !state.units[target].alive || !state.units[attacker_index].alive {
                break;
            }
            // The extra target shares the Coin's toss result and power but
            // resolves the hit like any other: its own On Hit effects, its own
            // Sinking / Rupture triggers and its own Guard or Evade.
            apply_hit_inner(
                state,
                attacker_index,
                target,
                use_,
                hit.coin_index,
                hit.power,
                hit.heads,
                clash_count,
                true,
            );
        }
    }
}

fn sin_name(sin: Sin) -> &'static str {
    match sin {
        Sin::Wrath => "Wrath",
        Sin::Lust => "Lust",
        Sin::Sloth => "Sloth",
        Sin::Gluttony => "Gluttony",
        Sin::Gloom => "Gloom",
        Sin::Pride => "Pride",
        Sin::Envy => "Envy",
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_hit(
    state: &mut BattleState,
    attacker_index: usize,
    defender_index: usize,
    use_: &mut SkillUse,
    coin_index: usize,
    power: i32,
    heads: bool,
    clash_count: i32,
) -> HitResult {
    apply_hit_inner(
        state,
        attacker_index,
        defender_index,
        use_,
        coin_index,
        power,
        heads,
        clash_count,
        false,
    )
}

/// `sub_target` marks an extra target of an Atk Weight splash: it shares the
/// Coin's toss result but runs the whole hit resolution for itself, while the
/// Skill's own events (ammo, self damage, Reuse) stay with the main target.
#[allow(clippy::too_many_arguments)]
fn apply_hit_inner(
    state: &mut BattleState,
    attacker_index: usize,
    defender_index: usize,
    use_: &mut SkillUse,
    coin_index: usize,
    power: i32,
    heads: bool,
    clash_count: i32,
    sub_target: bool,
) -> HitResult {
    let power_of_incoming = power;
    // A dead attacker deals no more damage (the Coin may have been forfeited to
    // self-inflicted damage such as Bleed).
    if !state.units[attacker_index].alive || !state.units[defender_index].alive {
        return HitResult {
            coin_index,
            heads,
            power,
            damage: 0,
            critical: false,
            staggered: false,
            killed: false,
        };
    }
    // A Coin destroyed by the Clash ("cracked") skips "[On Hit without
    // Cracking]" clauses.
    let coin_cracked = use_
        .coins
        .get(coin_index)
        .map(|coin| matches!(coin.state, CoinState::Cracked | CoinState::Destroyed))
        .unwrap_or(false);
    // Guard: on the first attack of the turn the Guard rolls its Coins and adds
    // Shield equal to its Final Power (wiki.gg `Battles` / Guard).
    if let Some(position) = state
        .defenses
        .iter()
        .position(|d| d.unit == state.units[defender_index].id && d.kind == DefenseKind::Guard && !d.activated)
    {
        let mut defense_use = {
            let defense = &state.defenses[position];
            SkillUse {
                actor: defense.unit.clone(),
                target: Some(state.units[attacker_index].id.clone()),
                slot: 0,
                skill: defense.skill.clone(),
                name: defense.name.clone(),
                sin: defense.sin,
                damage_type: defense.damage_type,
                base_power: defense.base_power,
                coin_power: defense.coin_power,
                offense_level_mod: defense.offense_level_mod,
                defense_level_mod: Some(defense.defense_level_mod),
                attack_weight: 1,
                coins: defense.coins.clone(),
                mechanics: defense.mechanics.clone(),
                ctx: defense.ctx.clone(),
                is_defense: true,
                is_ego: false,
                ego: None,
            }
        };
        let shield = {
            toss_all(state, defender_index, &mut defense_use);
            final_power(state, defender_index, &mut defense_use)
        };
        if let Some(defense) = state.defenses.get_mut(position) {
            defense.activated = true;
            defense.coins = defense_use.coins.clone();
        }
        state.units[defender_index].shield += shield;
        state.push_log(
            "guard",
            format!("{} gained {} Shield", defense_use.name, shield),
        );
    }
    // Bleed is applied when the coin is tossed; damage is computed for the hit.
    // Evade: flip against the incoming Coin; equal or higher negates the hit.
    if let Some(defense) = state
        .defenses
        .iter()
        .position(|d| d.unit == state.units[defender_index].id && d.kind == DefenseKind::Evade && !d.lost)
    {
        let evaded = {
            // The Evade rolls its own Coin and compares its **Final Power**,
            // which carries the Skill's modifiers, Paralysis and the defense
            // level correction like any other Skill
            // (JA-wiki 防御スキル: "回避スキルの値が攻撃スキルの値と同じかそれ以上なら
            // 攻撃は的中せず").
            let mut defense_use = {
                let defense = &state.defenses[defense];
                SkillUse {
                    actor: defense.unit.clone(),
                    target: Some(state.units[attacker_index].id.clone()),
                    slot: 0,
                    skill: defense.skill.clone(),
                    name: defense.name.clone(),
                    sin: defense.sin,
                    damage_type: defense.damage_type,
                    base_power: defense.base_power,
                    coin_power: defense.coin_power,
                    offense_level_mod: defense.offense_level_mod,
                    defense_level_mod: Some(defense.defense_level_mod),
                    attack_weight: 1,
                    coins: defense.coins.clone(),
                    mechanics: defense.mechanics.clone(),
                    ctx: defense.ctx.clone(),
                    is_defense: true,
                    is_ego: false,
                    ego: None,
                }
            };
            toss_all(state, defender_index, &mut defense_use);
            let power = final_power(state, defender_index, &mut defense_use);
            if let Some(entry) = state.defenses.get_mut(defense) {
                entry.coins = defense_use.coins.clone();
            }
            power >= power_of_incoming
        };
        if evaded {
            // "[On Evade]" clauses of the defense Skill.
            let on_evade = state.defenses[defense].mechanics.on_evade.clone();
            if !on_evade.is_empty() {
                let mut notes = Vec::new();
                let mut ctx = EffectContext {
                    actor_index: defender_index,
                    target_index: Some(attacker_index),
                    clash_count: 0,
                    clash_lost: false,
                    slot: 0,
                    mechanics_note: &mut notes,
                };
                let mut local = UseContext::default();
                apply_effects(state, &on_evade, &mut ctx, &mut local);
                for note in notes {
                    state.warnings.push(note);
                }
            }
            return HitResult {
                coin_index,
                heads,
                power: power_of_incoming,
                damage: 0,
                critical: false,
                staggered: false,
                killed: false,
            };
        }
        state.defenses[defense].lost = true;
    }
    let (crit, poise_potency) = {
        let attacker = &state.units[attacker_index];
        let potency = attacker.statuses.potency("Poise");
        let chance = potency + use_.ctx.crit_chance_bonus;
        let crit = chance > 0 && state.flip(chance);
        (crit, potency)
    };
    if crit
        && poise_potency > 0
        && !time_passives::keeps_poise_on_crit(state, attacker_index)
    {
        tick_status(state, attacker_index, "Poise", 0, -1);
    }
    let defender_snapshot = state.units[defender_index].clone();
    let defender = &defender_snapshot;
    let stagger_bonus = if defender.is_staggered() {
        Some(defender.stagger.damage_resistance_bonus())
    } else {
        None
    };
    let sin_resist = defender.resist_sin(use_.sin);
    let sin_name = sin_name(use_.sin);
    let mut type_resist = defender.resist(use_.damage_type);
    // [On Hit] coin effects.  "[Reuse - ...]" clauses only resolve on a Coin
    // that is being used again (wiki.gg `Clash`, trigger table).
    let hits_so_far = use_.ctx.coin_hits.get(&(coin_index as u32 + 1)).copied().unwrap_or(0);
    let reused = hits_so_far > 1;
    let mut effects: Vec<Effect> = use_
        .mechanics
        .coin(coin_index as u32 + 1)
        .iter()
        .filter(|effect| !effect.reuse_only || reused)
        // "[On Hit without Cracking]": a Coin that was destroyed by the Clash
        // does not resolve these clauses (wiki.gg `Clash`, Unbreakable Coins).
        .filter(|effect| !effect.only_without_cracking || !coin_cracked)
        .cloned()
        .collect();
    if heads {
        // "[Heads Hit]" clauses resolve on top of the Coin's On Hit effects.
        effects.extend_from_slice(use_.mechanics.heads_hit(coin_index as u32 + 1));
    }
    // Passives that trigger on a Tails Hit ("On Tails Hit, heal 5 SP").
    if !heads {
        let tails: Vec<Vec<Effect>> = state.units[attacker_index]
            .passives
            .iter()
            .map(|passive| passive.tails_hit.clone())
            .collect();
        for list in tails {
            effects.extend(list);
        }
    }

    // The Coin's own damage clauses are read before its damage is computed, and
    // they stay with this Coin instead of leaking into the next one.
    let coin_damage_bonus = {
        let mut notes = Vec::new();
        let mut ctx = EffectContext {
            actor_index: attacker_index,
            target_index: Some(defender_index),
            clash_count,
            clash_lost: use_.ctx.lost_clash,
            slot: use_.slot,
            mechanics_note: &mut notes,
        };
        let mut local = UseContext::default();
        let clauses: Vec<Effect> = effects
            .iter()
            .filter(|effect| is_coin_damage_clause(effect))
            .cloned()
            .collect();
        apply_effects(state, &clauses, &mut ctx, &mut local);
        for note in notes {
            state.warnings.push(note);
        }
        local.damage_bonus
    };
    for (kind, floor) in use_.ctx.resist_floor.iter() {
        let matches = match kind.as_str() {
            "slash" => use_.damage_type == DamageType::Slash,
            "pierce" => use_.damage_type == DamageType::Pierce,
            "blunt" => use_.damage_type == DamageType::Blunt,
            _ => false,
        };
        if matches && type_resist < *floor {
            type_resist = *floor;
        }
    }
    // Passives that ride on a Base Attack hit resolve **before** the hit, so a
    // "+50% damage with that Coin" clause can affect it ("If this unit has 3
    // [SwordCutwithTear]: ... deal +50% damage with that Coin").
    let rider_bonus = {
        let riding: Vec<Effect> = if use_.is_defense {
            Vec::new()
        } else {
            state.units[attacker_index]
                .passives
                .iter()
                .flat_map(|passive| {
                    passive
                        .passive
                        .iter()
                        .filter(|effect| effect.on_base_attack_hit)
                        .cloned()
                        .collect::<Vec<Effect>>()
                })
                .collect()
        };
        if riding.is_empty() {
            0.0
        } else {
            let mut notes = Vec::new();
            let mut ctx = EffectContext {
                actor_index: attacker_index,
                target_index: Some(defender_index),
                clash_count,
                clash_lost: use_.ctx.lost_clash,
                slot: use_.slot,
                mechanics_note: &mut notes,
            };
            let mut local = UseContext::default();
            apply_effects(state, &riding, &mut ctx, &mut local);
            for note in notes {
                state.warnings.push(note);
            }
            local.damage_bonus
        }
    };
    let inputs = DamageInputs {
        coin_roll: power,
        sin_resist,
        damage_type_resist: type_resist,
        stagger_bonus,
        offense_level: state.units[attacker_index].offense_level() + use_.offense_level_mod,
        defense_level: active_defense_level(state, defender_index)
            .unwrap_or_else(|| state.units[defender_index].defense_level()),
        critical: crit,
        clash_count,
        dynamic_modifier: use_.ctx.damage_bonus + rider_bonus + coin_damage_bonus
            + state.units[attacker_index].outgoing_damage_modifier()
            + incoming_damage_modifier(defender, sin_name)
            + passive_modifiers(state, defender_index, Some(attacker_index)).1
            + use_.ctx.damage_taken_bonus
            + if crit {
                time_passives::crit_damage_bonus(state, attacker_index) + use_.ctx.crit_damage_bonus
            } else {
                0.0
            },
        ..Default::default()
    };
    let breakdown = compute_damage(&inputs);
    let mut damage = if use_.ctx.zero_damage { 0 } else { breakdown.final_damage };
    if sub_target && use_.ctx.sub_target_damage_percent != 0 {
        // "Deal -50% damage against sub-targets".
        damage = damage * (100 + use_.ctx.sub_target_damage_percent) / 100;
    }
    let (_, hp_lost) = state.units[defender_index].take_damage(damage);

    *use_.ctx.coin_hits.entry(coin_index as u32 + 1).or_insert(0) += 1;
    // The order of a hit is fixed by the source: damage, then the **defender's**
    // own effects (its buffs/debuffs oldest first, passives and the "when hit"
    // triggers), then the attacker's "[On Hit]" clauses
    // (JA-wiki ダメージ / 攻撃の流れ: "ダメージ、被弾者のバフ/デバフ効果(古い順)、
    // パッシブやギフトの効果、的中時効果がこの順番通りに発動する").
    // A freshly inflicted Sinking must therefore survive the hit that applied it.
    // "[When hit]" clauses of the defender's statuses ("inflict 2 [Sinking] on
    // the attacker").
    apply_status_event(state, defender_index, "on_hit", Some(attacker_index));
    // Rupture: "When hit by an attack, take fixed damage by the effect's
    // Potency. Then, reduce its Count by 1." (wiki.gg `Status Effects`).
    let rupture = state.units[defender_index].statuses.potency("Rupture");
    if rupture > 0 {
        state.units[defender_index].take_damage(rupture);
        tick_status(state, defender_index, "Rupture", 0, -1);
    }
    // Sinking: when hit, SP damage by Potency then Count -1.
    apply_sinking(state, defender_index);

    // Butterfly (unique Sinking).  Source: in-game `BattleKeywords` /
    // `Bufs` (SinkingWhite) and wiki.gg `Status Effects`:
    //   * "When hit, the attacker heals (The Living / 4) SP (min 1; rounded down)"
    //   * "When hit, and if this unit's SP is at less than 0, take
    //     ([Sinking] Potency / 5) Gloom damage for every value of The Departed
    //     (max Gloom damage 30; rounded down; deals half damage to targets that
    //     are Non-SP Units)."
    let butterfly_living = state.units[defender_index].statuses.potency("Butterfly");
    let butterfly_departed = state.units[defender_index].statuses.count("Butterfly");
    if butterfly_living > 0 {
        let heal = (butterfly_living / 4).max(1);
        let sanity = state.units[attacker_index].sanity;
        state.units[attacker_index].sanity = sanity.add(heal);
    }
    if butterfly_departed > 0 {
        let non_sp_unit = matches!(state.units[defender_index].sanity, Sanity::None);
        // A Non-SP Unit has no SP to fall below zero; the game still lets the
        // effect apply (it is listed as taking half damage).
        let low_sp = non_sp_unit || state.units[defender_index].sanity.sp() < 0;
        if low_sp {
            let sinking = state.units[defender_index].statuses.potency("Sinking");
            let per_value = sinking / 5;
            let mut gloom = per_value * butterfly_departed;
            if non_sp_unit {
                gloom /= 2;
            }
            if gloom > 0 {
                let resist = state.units[defender_index].resist_sin(Sin::Gloom);
                let dealt = (gloom as f64
                    * (1.0 + crate::damage::resistance_modifier(resist)))
                .floor() as i32;
                // "max Gloom damage 30" bounds the damage actually taken.
                let damage = dealt.clamp(1, 30);
                state.units[defender_index].take_damage(damage);
                state.push_log(
                    "butterfly",
                    format!(
                        "Butterfly dealt {damage} Gloom damage ({per_value} x {butterfly_departed} The Departed)"
                    ),
                );
            }
        }
    }


    let mut notes = Vec::new();
    {
        let mut ctx = EffectContext {
            actor_index: attacker_index,
            target_index: Some(defender_index),
            clash_count,
            clash_lost: use_.ctx.lost_clash,
            slot: use_.slot,
            mechanics_note: &mut notes,
        };
        let mut local = UseContext::default();
        let clauses: Vec<Effect> = effects
            .iter()
            .filter(|effect| !is_coin_damage_clause(effect))
            .filter(|effect| !sub_target || is_target_facing_clause(effect))
            .cloned()
            .collect();
        apply_effects(state, &clauses, &mut ctx, &mut local);
        use_.ctx.ammo_spent = local.ammo_spent.max(use_.ctx.ammo_spent);
        // The ammo bookkeeping of this Coin travels back to the Skill use: the
        // split of the two-part pool, what each pool spent and whether the unit
        // ran dry (which cancels the remaining Coins).
        use_.ctx.ammo_living += local.ammo_living;
        use_.ctx.ammo_departed += local.ammo_departed;
        use_.ctx.ammo_exhausted |= local.ammo_exhausted;
        for (pool, spent) in local.ammo_spent_by_status {
            *use_.ctx.ammo_spent_by_status.entry(pool).or_insert(0) += spent;
        }
        // Coin-level clauses may add Reuse budget, consume statuses or lose HP.
        for (coin, cap) in local.reuse_caps {
            let entry = use_.ctx.reuse_caps.entry(coin).or_insert(0);
            *entry = (*entry).max(cap);
        }
        if local.reuse_hp.is_some() {
            use_.ctx.reuse_hp = local.reuse_hp;
        }
        use_.ctx.reuse_hp_used += local.reuse_hp_used;
        use_.ctx.self_damage_taken += local.self_damage_taken;
        use_.ctx.final_damage_percent += local.final_damage_percent;
        use_.ctx.consumed_status += local.consumed_status;
        // "Lower user's Stagger Threshold by N% of damage dealt" is written by a
        // hit clause, so it has to reach the Skill context that resolves it.
        use_.ctx.lower_stagger_percent += local.lower_stagger_percent;
    }
    for note in notes {
        state.warnings.push(note);
    }
    // Attack adders: "deal N% of this Coin's final damage as bonus damage".
    // Their conditions and their own damage type still apply ("At less than 33%
    // HP, deal +(50% of this Coin's damage)% bonus Pierce damage").
    for effect in use_.mechanics.coin(coin_index as u32 + 1).to_vec() {
        if effect.kind != "bonus_damage_percent_of_coin" {
            continue;
        }
        if !coin_clause_applies(
            state,
            attacker_index,
            defender_index,
            use_,
            &effect,
            coin_cracked,
            clash_count,
        ) {
            continue;
        }
        let percent = effect.percent.unwrap_or(0);
        if percent > 0 && damage > 0 {
            let type_resist = match effect.status.as_deref() {
                Some("slash") => state.units[defender_index].resist(DamageType::Slash),
                Some("pierce") => state.units[defender_index].resist(DamageType::Pierce),
                Some("blunt") => state.units[defender_index].resist(DamageType::Blunt),
                _ => 1.0,
            };
            let raw = (damage as f64 * percent as f64 / 100.0).floor();
            let extra = if effect.status.is_some() {
                (raw * (1.0 + crate::damage::resistance_modifier(type_resist))).floor() as i32
            } else {
                raw as i32
            };
            if extra > 0 {
                state.units[defender_index].take_damage(extra);
            }
        }
    }
    // "Deal Gloom damage equal to (N)% of this Coin's final damage" (the E.G.O
    // adders of Solemn Lament); the extra damage follows the same Gloom
    // resistance rule as any other Gloom hit.
    {
        let percent: i32 = use_
            .mechanics
            .coin(coin_index as u32 + 1)
            .iter()
            .filter(|e| e.kind == "extra_damage_percent_of_damage")
            .filter(|e| {
                coin_clause_applies(
                    state,
                    attacker_index,
                    defender_index,
                    use_,
                    e,
                    coin_cracked,
                    clash_count,
                )
            })
            .map(|e| {
                let base = e.final_damage_percent.unwrap_or(0);
                if e.per_ammo {
                    base + e.step.unwrap_or(0) * use_.ctx.ammo_spent
                } else {
                    base
                }
            })
            .sum();
        if percent > 0 && damage > 0 {
            let gloom = state.units[defender_index].resist_sin(crate::ids::Sin::Gloom);
            let raw = (damage as f64 * percent as f64 / 100.0).floor();
            let extra = (raw * (1.0 + crate::damage::resistance_modifier(gloom))).floor() as i32;
            if extra > 0 {
                state.units[defender_index].take_damage(extra);
            }
        }
    }
    // "The X - Segmentation": every hit as a main target knocks a Stack off the
    // Imago in the campaign and heals the attacker's SP once per turn; if the
    // unit is never hit this turn the Imago gains Stacks at Combat End.
    if let Some(segmentation) = state.units[defender_index].segmentation.clone() {
        state.units[defender_index].hits_taken += 1;
        let loss = segmentation.stack_loss_per_hit;
        let entry = state
            .campaign
            .time_stacks
            .entry(segmentation.stack_status.clone())
            .or_insert(0);
        *entry = (*entry - loss).max(0);
        let attacker_id = state.units[attacker_index].id.0.clone();
        if !state.units[defender_index]
            .segmentation_healed
            .contains(&attacker_id)
        {
            state.units[defender_index]
                .segmentation_healed
                .push(attacker_id);
            let sanity = state.units[attacker_index].sanity;
            state.units[attacker_index].sanity =
                sanity.add(segmentation.attacker_sp_heal);
        }
    }
    // "When hit while this unit has Shield, inflict N [X] against the attacker."
    if state.units[defender_index].shield > 0 {
        let retaliation: Vec<crate::state::RetaliateOnHit> =
            state.units[defender_index].retaliate_on_hit.clone();
        for entry in retaliation {
            state.units[attacker_index]
                .statuses
                .add_potency(&entry.status, entry.potency);
        }
    }
    // Past passive: hitting the Imago burns the attacker.
    time_passives::on_hit_by_sinner(state, defender_index, attacker_index);
    // Signature last-Coin effects of the active state.
    time_passives::signature_last_coin(
        state,
        attacker_index,
        &use_.skill.clone(),
        coin_index,
        use_.coins.len(),
        defender_index,
    );
    // A Counter skill strikes back at whoever attacked the unit.  It answers the
    // Skill that attacked it once and is then spent (wiki.gg `Battles` /
    // Defense Skills: a normal Counter triggers per incoming Skill, not per Coin).
    if state.units[defender_index].alive {
        if let Some(position) = state.defenses.iter().position(|d| {
            d.unit == state.units[defender_index].id
                && d.kind == DefenseKind::Counter
                && !d.lost
        }) {
            let counter = state.defenses[position].clone();
            let mut use_ = SkillUse {
                actor: counter.unit.clone(),
                target: Some(state.units[attacker_index].id.clone()),
                slot: 0,
                skill: counter.skill.clone(),
                name: counter.name.clone(),
                sin: counter.sin,
                damage_type: counter.damage_type,
                base_power: counter.base_power,
                coin_power: counter.coin_power,
                offense_level_mod: counter.offense_level_mod,
                defense_level_mod: None,
                attack_weight: 1,
                coins: counter.coins.clone(),
                mechanics: counter.mechanics.clone(),
                ctx: counter.ctx.clone(),
                is_defense: true,
                is_ego: false,
                ego: None,
            };
            state.push_log("counter", format!("{} counterattacked", counter.name));
            if let Some(entry) = state.defenses.get_mut(position) {
                entry.lost = true;
            }
            one_sided_attack(state, defender_index, attacker_index, &mut use_, 0);
        }
    }
    // "Lower user's Stagger Threshold by N% of damage dealt" (a self-inflicted
    // downside: the user becomes easier to Stagger).
    if use_.ctx.lower_stagger_percent > 0 && hp_lost > 0 {
        let reduction = (hp_lost * use_.ctx.lower_stagger_percent / 100).max(0);
        state.units[attacker_index]
            .stagger
            .shift_first_threshold(-reduction);
    }
    let staggered = if use_.ctx.no_stagger_target {
        false
    } else {
        check_stagger(state, defender_index)
    };
    let killed = !state.units[defender_index].alive;
    if killed {
        use_.ctx.kills += 1;
        state.units[attacker_index].statuses.add_potency("Poise", 0);
        // "[On Target Kill] / [On Kill]" clauses of this Skill.
        if !use_.ctx.on_kill_done && !use_.mechanics.on_kill.is_empty() {
            use_.ctx.on_kill_done = true;
            let on_kill = use_.mechanics.on_kill.clone();
            let mut notes = Vec::new();
            let mut ctx = EffectContext {
                actor_index: attacker_index,
                target_index: Some(defender_index),
                clash_count,
                clash_lost: use_.ctx.lost_clash,
                slot: use_.slot,
                mechanics_note: &mut notes,
            };
            let mut local = UseContext::default();
            apply_effects(state, &on_kill, &mut ctx, &mut local);
            for note in notes {
                state.warnings.push(note);
            }
        }
    }
    HitResult {
        coin_index,
        heads,
        power,
        damage: hp_lost,
        critical: crit,
        staggered,
        killed,
    }
}

/// Sinking tick (wiki.gg `Status Effects` / Sinking): SP damage to units with
/// Sanity, Gloom damage to units without.
pub fn apply_sinking(state: &mut BattleState, unit_index: usize) {
    let sinking = state.units[unit_index].statuses.potency("Sinking");
    if sinking <= 0 {
        return;
    }
    // "Gain 1 [AlriuneEGOWe] if the enemy takes [Sinking] damage" (Ryoshu's
    // Unwithering Flower).
    for other in 0..state.units.len() {
        if state.units[other].passive_ids.iter().any(|id| id == "1041411") {
            if state.units[other].kind.is_sinner() != state.units[unit_index].kind.is_sinner() {
                let cap = petals_gain_cap(state, other);
                if cap > 0 {
                    state.units[other].statuses.add_stack("Petals", 1);
                    state.units[other].petals_gained += 1;
                }
            }
        }
    }
    match state.units[unit_index].sanity {
        Sanity::None => {
            let gloom = state.units[unit_index].resist_sin(Sin::Gloom);
            let dmg = (sinking as f64 * (1.0 + crate::damage::resistance_modifier(gloom)))
                .floor()
                .max(1.0) as i32;
            state.units[unit_index].take_damage(dmg);
        }
        Sanity::Sane { .. } => {
            let sanity = state.units[unit_index].sanity;
            state.units[unit_index].sanity = sanity.add(-sinking);
        }
    }
    // "Then, reduce its Count by 1": a status whose Count reaches 0 is gone.
    tick_status(state, unit_index, "Sinking", 0, -1);
}

/// While a Guard is active, the unit's Defense Level is replaced by the skill's
/// (JA-wiki 守備スキル: "守備スキルを使用すると、以後そのターン中はキャラクターの
/// 防御レベルがスキルの防御レベルへと置き換わる").
fn active_defense_level(state: &BattleState, unit_index: usize) -> Option<i32> {
    let unit_id = &state.units[unit_index].id;
    let defense = state
        .defenses
        .iter()
        .find(|d| &d.unit == unit_id && d.kind == DefenseKind::Guard)?;
    Some((state.units[unit_index].level + defense.defense_level_mod).max(1))
}

/// Incoming damage modifiers of the defender: `Fragile`, `Protection`,
/// `<Sin> Fragility` (all 10% per Count, capped at 10) and Temporal Disjunction.
fn incoming_damage_modifier(defender: &Unit, sin_name: &str) -> f64 {
    let fragile = defender.statuses.count("Fragile").min(10) as f64 * 0.10;
    let protection = defender.statuses.count("Protection").min(10) as f64 * 0.10;
    let fragility = defender
        .statuses
        .count(&format!("{sin_name} Fragility"))
        .min(10) as f64
        * 0.10;
    fragile + fragility - protection + temporal_disjunction_bonus(defender)
}

/// Temporal Disjunction: "Take +(Stack x 15)% damage (max 150%)"
/// (in-game `Bufs_Refraction6` / `TimeGap`).
fn temporal_disjunction_bonus(defender: &Unit) -> f64 {
    let stack = defender.statuses.stack("Temporal Disjunction").min(10);
    stack as f64 * 0.15
}

/// Stagger check after damage (wiki.gg `Clash` / Stagger).
pub fn check_stagger(state: &mut BattleState, unit_index: usize) -> bool {
    let unit = &state.units[unit_index];
    if !unit.alive {
        return false;
    }
    // "一度混乱状態になってから立ち直ると、その混乱区間は消滅する" - a line that was
    // crossed is gone for the rest of the stage, so only the lines still in play
    // count.  Crossing a further line while already Staggered escalates the
    // level ("混乱 / 混乱+ / 混乱++") without refreshing the duration
    // ("混乱状態のターンは更新されない").
    let crossed = unit.stagger.remaining_crossed(unit.hp);
    if crossed == 0 {
        return false;
    }
    let unit = &mut state.units[unit_index];
    unit.stagger.consumed += crossed as usize;
    unit.stagger.level = (unit.stagger.level + crossed).min(crate::state::StaggerState::MAX_LEVEL);
    if unit.stagger.turns_remaining == 0 {
        // "unable to use their Skills for the current and subsequent Turn"
        unit.stagger.turns_remaining = 2;
    }
    true
}

// --------------------------------------------------------------------------- //
// turn loop
// --------------------------------------------------------------------------- //

/// Sanity, Low Morale and Panic (wiki.gg `Sanity` / `Clash`):
///   * SP lives in [-45, 45]; a Sinner that reaches -45 stays there until the
///     next Turn Start;
///   * at Turn Start, SP <= -45 is Panic (and E.G.O Corrosion when the Sinner
///     owns a Corrosion Skill), SP <= -30 is Low Morale;
///   * Low Morale / Panic effects apply once, on the earliest Turn Start;
///   * after a Panic turn the Sinner's SP resets to 0.
/// The wiki says Low Morale is chance-based but does not give the chance, so the
/// simulator applies it deterministically and records that as an assumption.
fn apply_sanity_states(state: &mut BattleState) {
    for index in 0..state.units.len() {
        if !state.units[index].alive {
            continue;
        }
        // The SP ceiling/floor applies to every SP Unit.
        if !matches!(state.units[index].sanity, Sanity::None) {
            let sanity = state.units[index].sanity;
            let sp = sanity.sp().clamp(-45, 45);
            state.units[index].sanity = sanity.set(sp);
        }
        if !state.units[index].kind.is_sinner() {
            continue;
        }
        // Recovery: a Sinner that spent a turn Panicked resets to 0 SP.
        if state.units[index].panic_recovering {
            state.units[index].panic_recovering = false;
            let sanity = state.units[index].sanity;
            state.units[index].sanity = sanity.set(0);
        }
        state.units[index].low_morale = false;
        state.units[index].panicked = false;
        state.units[index].corroded = false;
        let sp = state.units[index].sanity.sp();
        let at_limit = sp <= -45;
        // "At the next Turn Start, if a Sinner is at -45 SP and has an E.G.O with
        // a Corrosion type Skill, they are forced into E.G.O Corrosion" - that
        // replaces Panic (wiki.gg `Sanity`).
        let corroded = at_limit && !state.units[index].corrosion_egos.is_empty();
        state.units[index].corroded = corroded;
        let panicked = at_limit && !corroded;
        let low_morale = !at_limit && sp <= -30;
        if !panicked && !low_morale && !corroded {
            continue;
        }
        if corroded {
            state.units[index].panic_recovering = true;
            state.push_log(
                "corrosion",
                format!(
                    "{} is at -45 SP and Corrodes instead of Panicking",
                    state.units[index].name
                ),
            );
            continue;
        }
        state.units[index].low_morale = low_morale;
        state.units[index].panicked = panicked;
        let list = if panicked {
            state.units[index].panic_actions.clone()
        } else {
            state.units[index].panic_low_morale.clone()
        };
        if list.is_empty() && !panicked {
            continue;
        }
        // Effects that are marked as applying at Turn End wait for `end_turn`.
        let (now, _later): (Vec<Effect>, Vec<Effect>) = list
            .iter()
            .cloned()
            .partition(|effect| !effect.trigger_turn_end);
        // Only the immediate half applies now; the "[Turn End]" half waits for
        // `end_turn` (`apply_panic_turn_end`), or it would run twice per turn.
        for batch in [now] {
            if batch.is_empty() {
                continue;
            }
            let mut notes = Vec::new();
            let mut ctx = EffectContext {
                actor_index: index,
                target_index: None,
                clash_count: 0,
                clash_lost: false,
                slot: 0,
                mechanics_note: &mut notes,
            };
            let mut use_ctx = UseContext::default();
            apply_effects(state, &batch, &mut ctx, &mut use_ctx);
            for note in notes {
                state.warnings.push(note);
            }
        }
        if panicked {
            state.units[index].panic_recovering = true;
            state.units[index].turn_effect_usage.clear();
        }
        state.push_log(
            "sanity",
            format!(
                "{} is in {} (SP {})",
                state.units[index].name,
                if panicked { "Panic" } else { "Low Morale" },
                sp
            ),
        );
    }
}

/// Turn End half of a Panic Type ("Turn End: Gain 1 [Bind] ...").
fn apply_panic_turn_end(state: &mut BattleState) {
    for index in 0..state.units.len() {
        if !state.units[index].alive
            || !(state.units[index].panicked || state.units[index].low_morale)
        {
            continue;
        }
        let list: Vec<Effect> = if state.units[index].panicked {
            state.units[index].panic_actions.clone()
        } else {
            state.units[index].panic_low_morale.clone()
        };
        let later: Vec<Effect> = list
            .into_iter()
            .filter(|effect| effect.trigger_turn_end)
            .collect();
        if later.is_empty() {
            continue;
        }
        let mut notes = Vec::new();
        let mut ctx = EffectContext {
            actor_index: index,
            target_index: None,
            clash_count: 0,
            clash_lost: false,
            slot: 0,
            mechanics_note: &mut notes,
        };
        let mut use_ctx = UseContext::default();
        apply_effects(state, &later, &mut ctx, &mut use_ctx);
        for note in notes {
            state.warnings.push(note);
        }
    }
}

/// A status's own clauses refer to themselves as "self" (the extractor cannot
/// know the name while parsing one text): rewrite it to the status key.
fn resolve_self(effect: &Effect, status: &str) -> Effect {
    let mut effect = effect.clone();
    if effect.status.as_deref() == Some("self") {
        effect.status = Some(status.to_string());
    }
    if effect.status2.as_deref() == Some("self") {
        effect.status2 = Some(status.to_string());
    }
    if let Some(cond) = effect.condition.as_mut() {
        if cond.status.as_deref() == Some("self") {
            cond.status = Some(status.to_string());
        }
        for entry in cond.statuses.iter_mut() {
            if entry == "self" {
                *entry = status.to_string();
            }
        }
    }
    effect
}

/// "[Turn Start]" / "[Turn End]" clauses of the statuses a unit holds, plus
/// "Expires at Turn End" / "Turn End: Lose N Stack" upkeep.
fn apply_status_phase(
    state: &mut BattleState,
    book: &crate::effects::StatusBook,
    start: bool,
) {
    // The statuses a unit already holds before this Turn End pass: a one-turn
    // status granted *by* a Turn End clause must survive into the next turn, so
    // only the ones present beforehand expire.
    let before: Vec<Vec<String>> = state
        .units
        .iter()
        .map(|unit| {
            unit.statuses
                .iter()
                .filter(|(_, instance)| instance.potency + instance.count + instance.stack > 0)
                .map(|(name, _)| name.clone())
                .collect()
        })
        .collect();
    for index in 0..state.units.len() {
        if !state.units[index].alive {
            continue;
        }
        let held: Vec<(String, i32)> = state.units[index]
            .statuses
            .iter()
            .map(|(key, instance)| {
                (key.clone(), instance.potency + instance.count + instance.stack)
            })
            .collect();
        for (status, total) in held {
            if total <= 0 {
                continue;
            }
            let Some(behaviour) = book.get(&status) else { continue };
            let mut list: Vec<Effect> = if start {
                behaviour.effects.turn_start.clone()
            } else {
                behaviour.effects.turn_end.clone()
            };
            if !start {
                // "Expires at Turn End" / "Max Stack: N" upkeep.
                if behaviour
                    .effects
                    .passive
                    .iter()
                    .any(|effect| effect.kind == "remove_at_turn_end")
                {
                    state.units[index].statuses.remove(&status);
                    continue;
                }
            }
            if list.is_empty() {
                continue;
            }
            list = list.iter().map(|effect| resolve_self(effect, &status)).collect();
            let mut notes = Vec::new();
            let mut ctx = EffectContext {
                actor_index: index,
                target_index: None,
                clash_count: 0,
                clash_lost: false,
                slot: 0,
                mechanics_note: &mut notes,
            };
            let mut use_ctx = UseContext::default();
            apply_effects(state, &list, &mut ctx, &mut use_ctx);
            let shield = use_ctx.shield_gain;
            if shield > 0 {
                state.units[index].shield += shield;
            }
            for note in notes {
                state.warnings.push(note);
            }
        }
    }
    if !start {
        // "for one turn" statuses that were already on the unit expire now.
        for index in 0..state.units.len() {
            if !state.units[index].alive {
                continue;
            }
            let expiring: Vec<String> = state.units[index]
                .statuses
                .iter()
                .filter(|(name, instance)| {
                    instance.potency + instance.count + instance.stack > 0
                        && before
                            .get(index)
                            .map(|held| held.iter().any(|held| held == *name))
                            .unwrap_or(false)
                        && book
                            .get(name)
                            .map(|behaviour| behaviour.expires_at_turn_end)
                            .unwrap_or(false)
                })
                .map(|(name, _)| name.clone())
                .collect();
            for name in expiring {
                state.units[index].statuses.remove(&name);
            }
        }
    }
}

/// Consume Potency / Count of a status and remove the status once a value it
/// uses reaches 0.
///
/// Source: wiki.gg `Status Effects` (Overview): "In single-value or double-value
/// modes, if one or more of the values reach 0, the status effect is removed
/// from the unit."  Called at every tick that consumes a value, so a status a
/// Skill merely inflicted with Count 0 (the game writes "Inflict N [X]" for
/// Potency) is left alone until something actually consumes it.
/// Consume `amount` of a status' total value, taking Potency first and then
/// Count, and cleaning the status up through its own expiry policy.
///
/// "At 15+ [Deep Tears], consume 5" must leave 10 behind - removing the whole
/// status is only correct when the clause says "consume all".
fn consume_status_amount(
    state: &mut BattleState,
    unit_index: usize,
    status: &str,
    amount: i32,
) -> i32 {
    if amount <= 0 {
        return 0;
    }
    let mut left = amount;
    let potency = state.units[unit_index].statuses.potency(status);
    let take_potency = left.min(potency);
    if take_potency > 0 {
        tick_status(state, unit_index, status, -take_potency, 0);
        left -= take_potency;
    }
    if left > 0 {
        let count = state.units[unit_index].statuses.count(status);
        let take_count = left.min(count);
        if take_count > 0 {
            tick_status(state, unit_index, status, 0, -take_count);
            left -= take_count;
        }
    }
    amount - left
}

fn tick_status(
    state: &mut BattleState,
    unit_index: usize,
    status: &str,
    potency_delta: i32,
    count_delta: i32,
) {
    {
        let unit = &mut state.units[unit_index];
        if potency_delta != 0 {
            unit.statuses.add_potency(status, potency_delta);
        }
        if count_delta != 0 {
            unit.statuses.add_count(status, count_delta);
        }
    }
    let behaviour = state.status_book.as_ref().and_then(|book| book.get(status));
    let structure = behaviour
        .and_then(|behaviour| behaviour.structure.clone())
        .unwrap_or_else(|| "potency_count".to_string());
    let primary = behaviour
        .and_then(|behaviour| behaviour.primary.clone())
        .unwrap_or_else(|| "potency".to_string());
    let policy = behaviour
        .and_then(|behaviour| behaviour.expiry.clone())
        .unwrap_or_else(|| "either_zero".to_string());
    let instance = state.units[unit_index].statuses.get(status);
    let gone = match policy.as_str() {
        "none" => false,
        "both_zero" => instance.potency == 0 && instance.count == 0,
        "count_zero" => instance.count == 0,
        "potency_zero" => instance.potency == 0,
        _ => {
            if structure == "stack" {
                // A Stack status ends by its own rules ("Turn End: Lose 1
                // Stack"), never because a value it does not use is 0.
                false
            } else if structure == "potency_count" {
                instance.potency == 0 || instance.count == 0
            } else if primary == "count" {
                // A status with a single value: only that value can end it.
                instance.count == 0
            } else {
                instance.potency == 0
            }
        }
    };
    if gone {
        state.units[unit_index].statuses.remove(status);
    }
}

/// Run the `[When Clash ends]` / `[When hit]` clauses of a unit's statuses.
/// `target` is the other unit ("inflict 2 [Sinking] on the attacker").
fn apply_status_event(state: &mut BattleState, index: usize, phase: &str, target: Option<usize>) {
    let Some(book) = state.status_book.clone() else { return };
    let held: Vec<(String, i32)> = state.units[index]
        .statuses
        .iter()
        .map(|(key, instance)| {
            (key.clone(), instance.potency + instance.count + instance.stack)
        })
        .collect();
    for (status, total) in held {
        if total <= 0 {
            continue;
        }
        let Some(behaviour) = book.get(&status) else { continue };
        let list: Vec<Effect> = if phase == "clash_end" {
            behaviour.effects.clash_end.clone()
        } else {
            behaviour.effects.on_hit.clone()
        };
        if list.is_empty() {
            continue;
        }
        let list: Vec<Effect> = list
            .iter()
            .map(|effect| {
                let mut effect = resolve_self(effect, &status);
                if effect.on_attacker {
                    effect.source = Some("target".to_string());
                }
                effect
            })
            .collect();
        let mut notes = Vec::new();
        let mut ctx = EffectContext {
            actor_index: index,
            target_index: target,
            clash_count: 0,
            clash_lost: false,
            slot: 0,
            mechanics_note: &mut notes,
        };
        let mut use_ctx = UseContext::default();
        apply_effects(state, &list, &mut ctx, &mut use_ctx);
        for note in notes {
            state.warnings.push(note);
        }
    }
}

/// Continuous modifiers contributed by the statuses a unit holds.
fn status_modifiers(state: &BattleState, index: usize, target: Option<usize>) -> (f64, f64) {
    let Some(book) = state.status_book.as_ref() else {
        return (0.0, 0.0);
    };
    if std::env::var("LCB_DEBUG").is_ok() {
        eprintln!("status_modifiers unit {index}: {} statuses", state.units[index].statuses.iter().count());
        for (k, v) in state.units[index].statuses.iter() {
            eprintln!("   {k} p={} c={} s={} in_book={}", v.potency, v.count, v.stack, book.get(k).is_some());
        }
    }
    let actor = &state.units[index];
    let target_unit = target.map(|i| &state.units[i]);
    let mut outgoing = 0.0;
    let mut incoming = 0.0;
    for (status, instance) in actor.statuses.iter() {
        if instance.potency + instance.count + instance.stack <= 0 {
            continue;
        }
        let Some(behaviour) = book.get(status) else { continue };
        for effect in &behaviour.effects.passive {
            let effect = resolve_self(effect, status);
            let holds = match &effect.condition {
                Some(cond) => condition_holds(cond, actor, target_unit, 0, 0),
                None => true,
            };
            if !holds {
                continue;
            }
            match effect.kind.as_str() {
                "damage_percent" => {
                    let step = effect.step_f.unwrap_or(effect.step.unwrap_or(1) as f64);
                    let measured = measured_from_condition(&effect, actor, target_unit);
                    let max = effect.max.unwrap_or(i32::MAX) as f64;
                    outgoing += (step * measured as f64).min(max) / 100.0;
                }
                "damage_taken_percent" => {
                    let step = effect.step_f.unwrap_or(effect.step.unwrap_or(1) as f64);
                    let measured = measured_from_condition(&effect, actor, target_unit);
                    let max = effect.max.unwrap_or(i32::MAX) as f64;
                    outgoing += 0.0;
                    incoming += (step * measured as f64).min(max) / 100.0;
                }
                _ => {}
            }
        }
    }
    (outgoing, incoming)
}

/// Evaluate the passive clauses of one phase for one unit.
fn apply_passive_phase(
    state: &mut BattleState,
    index: usize,
    target: Option<usize>,
    phase: fn(&SkillMechanics) -> &[Effect],
) {
    let lists: Vec<Vec<Effect>> = state.units[index]
        .passives
        .iter()
        .map(|passive| phase(passive).to_vec())
        .collect();
    for list in lists {
        if list.is_empty() {
            continue;
        }
        let mut notes = Vec::new();
        let mut ctx = EffectContext {
            actor_index: index,
            target_index: target,
            clash_count: 0,
            clash_lost: false,
            slot: 0,
            mechanics_note: &mut notes,
        };
        let mut use_ctx = UseContext::default();
        apply_effects(state, &list, &mut ctx, &mut use_ctx);
        let shield = use_ctx.shield_gain;
        if shield > 0 {
            state.units[index].shield += shield;
        }
        for note in notes {
            state.warnings.push(note);
        }
    }
}

/// Continuous passive modifiers of a unit: damage it deals and damage it takes.
/// Only the modifier kinds are read here (a passive's grants are applied by the
/// phase triggers), so this stays a pure query.
fn passive_modifiers(state: &BattleState, index: usize, target: Option<usize>) -> (f64, f64) {
    let (status_out, status_in) = status_modifiers(state, index, target);
    let actor = &state.units[index];
    let target_unit = target.map(|i| &state.units[i]);
    let mut outgoing = 0.0;
    let mut incoming = 0.0;
    for passive in &actor.passives {
        for effect in &passive.passive {
            let holds = match &effect.condition {
                Some(cond) => condition_holds(cond, actor, target_unit, 0, 0),
                None => true,
            };
            if !holds {
                continue;
            }
            match effect.kind.as_str() {
                "damage_percent" => {
                    let measured = measured_from_condition(effect, actor, target_unit);
                    outgoing += scaled(effect, measured) as f64 / 100.0;
                }
                "damage_taken_percent" => {
                    // Either a fixed percentage or one that scales with SP
                    // ("Take -(SP / 2)% HP damage from attacks (max 20%)").
                    let max = effect.max.unwrap_or(i32::MAX);
                    let percent = match effect.per {
                        Some(per) if per > 0 => {
                            -(((actor.sanity.sp().max(0)) / per) * effect.step.unwrap_or(1))
                        }
                        _ => effect.value.unwrap_or(0),
                    };
                    incoming += percent.clamp(-max, max) as f64 / 100.0;
                }
                "damage_percent_from_negative_sp" => {
                    let per = effect.per.unwrap_or(1).max(1);
                    let step = effect.step.unwrap_or(1);
                    let max = effect.max.unwrap_or(i32::MAX);
                    let deficit = (-actor.sanity.sp()).max(0);
                    outgoing += ((deficit / per) * step).min(max) as f64 / 100.0;
                }
                _ => {}
            }
        }
    }
    (outgoing + status_out, incoming + status_in)
}

/// "[Turn Start]" / "[Turn End]" clauses of the Skills equipped on a unit's
/// Dashboard (`Skill Slot`), applied once per distinct Skill.
fn apply_dashboard_phase(state: &mut BattleState, mechanics: &MechanicsBook, start: bool) {
    for index in 0..state.units.len() {
        if !state.units[index].alive {
            continue;
        }
        let mut seen: Vec<String> = Vec::new();
        let mut slots: Vec<(u32, SkillId)> = if start {
            // At Turn Start the panel is what the unit is holding, so every
            // equipped Skill's upkeep applies.
            state.units[index]
                .dashboard
                .iter()
                .enumerate()
                .flat_map(|(slot, entry)| {
                    // Both visible Skills of a Slot are equipped, as is the preview.
                    [&entry.current, &entry.next, &entry.preview]
                        .into_iter()
                        .map(move |skill| (slot as u32 + 1, skill.clone()))
                })
                .collect()
        } else {
            // At Turn End only the Skills this unit actually used carry their
            // upkeep: a defense Skill that was never selected does not pay its
            // SP or hand out its Petals.
            state
                .actions
                .iter()
                .filter(|action| action.actor == state.units[index].id)
                .map(|action| (action.slot, action.skill.clone()))
                .collect()
        };
        // The defense Skill is not drawn on the Dashboard but carries its own
        // Turn Start / Turn End upkeep.
        for skill in &state.units[index].identity_skills {
            if !start
                && !state
                    .defenses
                    .iter()
                    .any(|defense| defense.unit == state.units[index].id && &defense.skill == skill)
            {
                continue;
            }
            if !slots.iter().any(|(_, owned)| owned == skill) {
                slots.push((4, skill.clone()));
            }
        }
        for (slot, skill) in slots {
            if seen.iter().any(|s| s == skill.as_str()) {
                continue;
            }
            seen.push(skill.as_str().to_string());
            let Some(record) = mechanics.get_for(&skill, Uptie(4)) else { continue };
            let list = if start {
                record.turn_start.clone()
            } else {
                record.turn_end.clone()
            };
            if list.is_empty() {
                continue;
            }
            let mut notes = Vec::new();
            let mut ctx = EffectContext {
                actor_index: index,
                target_index: None,
                clash_count: 0,
                clash_lost: false,
                slot,
                mechanics_note: &mut notes,
            };
            let mut use_ctx = UseContext::default();
            apply_effects(state, &list, &mut ctx, &mut use_ctx);
            let shield = use_ctx.shield_gain;
            if shield > 0 {
                state.units[index].shield += shield;
            }
            for note in notes {
                state.warnings.push(note);
            }
        }
    }
}

pub fn begin_turn(
    state: &mut BattleState,
    library: &Library,
    mechanics: &MechanicsBook,
    scripts: &crate::scripts::ScriptsBook,
) {
    state.turn += 1;
    state.phase = Phase::TurnStart;
    state.actions.clear();
    // Defense Slots armed last turn that never made it into a rotation.
    state.defense_slots_used.clear();
    // "Next turn" buffs arrive before this turn is resolved, so a queued Haste
    // or Bind moves the Speed of the turn it applies to.
    for index in 0..state.units.len() {
        let queued: Vec<crate::state::PendingStatus> =
            std::mem::take(&mut state.units[index].pending_next_turn);
        for entry in queued {
            state.units[index].statuses.add_potency(&entry.status, entry.potency);
            state.units[index].statuses.add_count(&entry.status, entry.count);
            state.units[index].statuses.add_stack(&entry.status, entry.stack);
        }
    }
    // Shield does not carry over between turns (wiki.gg `Clash` / Shield).
    for unit in state.units.iter_mut() {
        unit.shield = 0;
    }
    for index in 0..state.units.len() {
        if !state.units[index].alive {
            continue;
        }
        let (lo, hi) = state.units[index].speed_range;
        let speed = if hi > lo {
            lo + state.rng.below((hi - lo + 1) as u32) as i32
        } else {
            lo
        };
        // `Haste`: "Speed increases by the effect's Count for one turn."
        // `Bind`: "Speed decreases by the effect's Count for one turn."
        // (in-game `Bufs`: Agility / Binding)
        let haste = state.units[index].statuses.count("Haste");
        let bind = state.units[index].statuses.count("Bind");
        state.units[index].speed = (speed + haste - bind).max(1);
        // Bleed/other turn-start ticks handled by mechanics entries below.

    }
    // Extra Skill Slots: from turn 2 on, the Sinner with the lowest Deployment
    // Order who does not have one yet receives an additional slot, and so on
    // (wiki.gg `Battles` / Deployment Order).
    if state.turn >= 2 {
        let current: usize = state
            .units
            .iter()
            .filter(|u| u.kind.is_sinner())
            .map(|u| u.dashboard.len())
            .sum();
        if current < state.slot_target {
            if let Some(unit) = state
                .units
                .iter_mut()
                .filter(|u| u.alive && u.kind.is_sinner())
                .min_by_key(|u| u.dashboard.len())
            {
                if unit.dashboard.len() < state.slot_target {
                    let slot = unit.dashboard.len() as u32;
                    unit.dashboard.push(crate::state::DashboardSlot::new(
                        slot,
                        SkillId::new(EMPTY_SKILL),
                        SkillId::new(EMPTY_SKILL),
                        SkillId::new(EMPTY_SKILL),
                    ));
                }
            }
        }
        // Fill any empty slot on the panel (bottom then top, left to right).
        for index in 0..state.units.len() {
            if !state.units[index].kind.is_sinner() {
                continue;
            }
            let slots: Vec<u32> = state.units[index].dashboard.iter().map(|s| s.slot).collect();
            for slot in slots {
                let (needs_current, needs_next, needs_preview) = {
                    let entry = state.units[index]
                        .dashboard
                        .iter()
                        .find(|s| s.slot == slot)
                        .unwrap();
                    (
                        entry.current.0.is_empty(),
                        entry.next.0.is_empty(),
                        entry.preview.0.is_empty(),
                    )
                };
                if needs_current {
                    let drawn = crate::setup::draw_for_unit(state, index)
                        .unwrap_or_else(crate::setup::empty_skill);
                    if let Some(entry) = state.units[index]
                        .dashboard
                        .iter_mut()
                        .find(|s| s.slot == slot)
                    {
                        entry.current = drawn;
                    }
                }
                if needs_next {
                    let drawn = crate::setup::draw_for_unit(state, index)
                        .unwrap_or_else(crate::setup::empty_skill);
                    if let Some(entry) = state.units[index]
                        .dashboard
                        .iter_mut()
                        .find(|s| s.slot == slot)
                    {
                        entry.next = drawn;
                    }
                }
                if needs_preview {
                    let drawn = crate::setup::draw_for_unit(state, index)
                        .unwrap_or_else(crate::setup::empty_skill);
                    if let Some(entry) = state.units[index]
                        .dashboard
                        .iter_mut()
                        .find(|s| s.slot == slot)
                    {
                        entry.preview = drawn;
                    }
                }
            }
        }
    }
    // Per-turn bookkeeping.
    for index in 0..state.units.len() {
        state.units[index]
            .turn_effect_usage
            .retain(|key, _| key.starts_with("encounter:"));
        state.units[index].hits_taken = 0;
        state.units[index].petals_gained = 0;
        state.units[index].segmentation_healed.clear();
    }
    // The state of time for this turn is decided **before** the passives of that
    // state run, so a turn that switches from Present to Future already gains
    // Future's [In the Future] and Poise.
    for index in 0..state.units.len() {
        if state.units[index].alive && !state.units[index].kind.is_sinner() {
            update_time_state(state, scripts, index);
        }
    }
    // Past / Present / Future turn-start effects.
    for index in 0..state.units.len() {
        if state.units[index].alive && !state.units[index].kind.is_sinner() {
            time_passives::turn_start(state, index);
        }
        // Temporal Disjunction: at 10 Stack, gain 5 Fragile.
        if state.units[index].statuses.stack("Temporal Disjunction") >= 10 {
            state.units[index].statuses.add_count("Fragile", 5);
        }
    }
    // Equipped Skills carry their own "[Turn Start]" clauses; they resolve for
    // the Skill Slots the unit has on the Dashboard.
    apply_dashboard_phase(state, mechanics, true);
    // Status upkeep: "[Turn Start]" clauses of every status the unit holds.
    if let Some(book) = state.status_book.clone() {
        apply_status_phase(state, &book, true);
    }
    // Sanity: clamp SP, then Low Morale (-30) / Panic (-45) for Sinners.
    // Source: wiki.gg `Sanity` + `Clash`.
    apply_sanity_states(state);
    // Passives: "[Turn Start]" clauses, per unit.  The "[Combat Start]" half runs
    // once the Skills for the turn are chosen (see `resolve_combat`), because a
    // passive can read the turn's Sin Resonance.
    for index in 0..state.units.len() {
        if !state.units[index].alive {
            continue;
        }
        let target = state
            .units
            .iter()
            .position(|u| u.alive && u.kind.is_sinner() != state.units[index].kind.is_sinner());
        apply_passive_phase(state, index, target, |m| &m.turn_start);
    }
    // Enemy Skill Slots whose Skill says "Can Clash with this Skill regardless of
    // Speed": a Sinner may chain to them even when slower.
    for unit in state.units.iter_mut() {
        unit.clash_any_speed_slots.clear();
    }
    let enemy_slots: Vec<(usize, u32, SkillId)> = state
        .actions
        .iter()
        .filter_map(|action| {
            let index = state.index_of(&action.actor)?;
            if state.units[index].kind.is_sinner() {
                return None;
            }
            Some((index, action.slot, action.skill.clone()))
        })
        .collect();
    for (index, slot, skill) in enemy_slots {
        if mechanics
            .get_for(&skill, Uptie(4))
            .map(|list| list.tags.iter().any(|tag| tag == "clash_any_speed"))
            .unwrap_or(false)
        {
            state.units[index].clash_any_speed_slots.push(slot);
        }
    }
    // Enemy Skill Slots.  A unit with a documented action pattern (the Imago)
    // uses one action per listed slot for the current turn of its cycle.
    for index in 0..state.units.len() {
        if !state.units[index].alive || state.units[index].kind.is_sinner() {
            continue;
        }
        let skills = enemy_turn_skills(state, library, scripts, index);
        // "Targets the unit with the most HP" / "Targets randomly" / "Prioritizes
        // targets that have the most [X]" (skill tags from the effect text).
        let selections: Vec<Option<Vec<String>>> = skills
            .iter()
            .map(|skill| mechanics.get_for(skill, Uptie(4)).map(|m| m.tags.clone()))
            .collect();
        let targets = enemy_targets(state, index, &selections);
        for (slot, skill) in skills.into_iter().enumerate() {
            let target = targets.get(slot).cloned().flatten();
            state.actions.push(SubmittedAction {
                actor: state.units[index].id.clone(),
                slot: slot as u32,
                skill,
                target,
                is_ego: false,
                ego: None,
                ego_kind: None,
                enemy_slot: None,
                used_top: false,
            });
        }
    }
    state.phase = Phase::AwaitingActions;
}

/// Skills the enemy will use this turn.
///
/// With a script (`data/mechanics/enemy_scripts.json`) the turn comes straight
/// from the wiki's action pattern: the band is chosen by HP, the state of time
/// by the unit's Stack, and the cycle position advances every turn.  Without a
/// script the configured fallback policy is used.
fn enemy_turn_skills(
    state: &mut BattleState,
    library: &Library,
    scripts: &crate::scripts::ScriptsBook,
    unit_index: usize,
) -> Vec<SkillId> {
    let UnitKind::Abnormality { enemy, .. } = state.units[unit_index].kind.clone() else {
        return Vec::new();
    };
    update_time_state(state, scripts, unit_index);
    if let Some(script) = scripts.for_enemy(enemy.as_str()) {
        let hp_percent = state.units[unit_index].hp_percent();
        let time_state = state.units[unit_index]
            .time_state
            .unwrap_or(state.config.initial_time_state);
        let cycle = state.units[unit_index].skill_cursor;
        let branch_active = state.units[unit_index].barrier_broken;
        let skills = script.turn_skills_with_branch(hp_percent, time_state, cycle, branch_active);
        state.units[unit_index].skill_cursor = (cycle + 1) % script.cycle_turns.max(1);
        if !skills.is_empty() {
            return skills.into_iter().map(SkillId::new).collect();
        }
    }
    // Fallback policy for enemies without a documented pattern.
    let Some(record) = library.enemy(&enemy) else {
        return Vec::new();
    };
    if record.skills.is_empty() {
        return Vec::new();
    }
    let index = match state.config.enemy_policy {
        crate::state::EnemyPolicy::FirstListed => 0,
        crate::state::EnemyPolicy::Cyclic => {
            let cursor = state.units[unit_index].skill_cursor as usize % record.skills.len();
            state.units[unit_index].skill_cursor = ((cursor + 1) % record.skills.len()) as u32;
            cursor
        }
    };
    record
        .skills
        .get(index)
        .map(|s| vec![SkillId::new(s.skill_id())])
        .unwrap_or_default()
}

/// Imago: keep the three states of time on the unit and pick the active one.
///
/// "Turn Start: Activate the highest-Stacked state of time between In the Past,
/// In the Present, and In the Future ... If multiple states of time share the
/// highest Stack, the currently active state of time does not change"
/// (wiki.gg `Butterfly of Entangled Lives::Imago` / Moment of Entangled Lives).
fn update_time_state(
    state: &mut BattleState,
    scripts: &crate::scripts::ScriptsBook,
    unit_index: usize,
) {
    let UnitKind::Abnormality { enemy, .. } = state.units[unit_index].kind.clone() else {
        return;
    };
    let Some(_script) = scripts.for_enemy(enemy.as_str()) else {
        return;
    };
    // Encounter start: 10 Stacks of each state of time.
    if state.turn <= 1 && crate::scripts::TimeState::ALL.iter().all(|s| {
        state.units[unit_index].statuses.stack(s.stack_key()) == 0
    }) {
        for time_state in crate::scripts::TimeState::ALL {
            state.units[unit_index]
                .statuses
                .set_stack(time_state.stack_key(), 10);
        }
        state.units[unit_index].time_state = Some(state.config.initial_time_state);
    }
    // Crossing 66% / 33% HP grants 10 more Stacks of each state, once each.
    let hp_percent = state.units[unit_index].hp_percent();
    let flags = state.units[unit_index].time_threshold_flags;
    if hp_percent <= 66 && flags & 1 == 0 {
        for time_state in crate::scripts::TimeState::ALL {
            state.units[unit_index]
                .statuses
                .add_stack(time_state.stack_key(), 10);
        }
        state.units[unit_index].time_threshold_flags |= 1;
    }
    if hp_percent <= 33 && flags & 2 == 0 {
        for time_state in crate::scripts::TimeState::ALL {
            state.units[unit_index]
                .statuses
                .add_stack(time_state.stack_key(), 10);
        }
        state.units[unit_index].time_threshold_flags |= 2;
    }
    // Activate the highest Stack; ties keep the current state.
    let current = state.units[unit_index].time_state;
    let best = crate::scripts::TimeState::ALL
        .iter()
        .map(|s| (*s, state.units[unit_index].statuses.stack(s.stack_key())))
        .max_by_key(|(_, stack)| *stack);
    if let Some((candidate, stack)) = best {
        let current_stack = current
            .map(|s| state.units[unit_index].statuses.stack(s.stack_key()))
            .unwrap_or(-1);
        if stack > current_stack {
            if current != Some(candidate) {
                // Switching states adds Temporal Disjunction.
                state.units[unit_index].statuses.add_stack("Temporal Disjunction", 1);
            }
            state.units[unit_index].time_state = Some(candidate);
        }
    }
}

/// Enemy targeting: slots are spread over the living Sinners in speed order
/// (the real game picks by speed and threat; this is a documented stand-in).
fn enemy_targets(
    state: &BattleState,
    unit_index: usize,
    selections: &[Option<Vec<String>>],
) -> Vec<Option<UnitId>> {
    let slots = selections.len();
    // `Aggro`: "More likely to be targeted by enemies" - units with Aggro are
    // preferred, then the fastest (documented stand-in for the real targeting).
    let mut sinners: Vec<(i32, i32, usize, UnitId)> = state
        .units
        .iter()
        .enumerate()
        .filter(|(_, u)| u.alive && u.kind.is_sinner())
        .map(|(index, u)| {
            let aggro: i32 = u.statuses.stack("Aggro") + u.statuses.count("Aggro");
            (aggro, u.speed, index, u.id.clone())
        })
        .collect();
    sinners.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    if sinners.is_empty() {
        return vec![None; slots];
    }
    let _ = unit_index;
    let mut rng = state.rng.clone();
    (0..slots)
        .map(|slot| {
            let tags = selections.get(slot).and_then(|t| t.as_ref());
            let pick = match tags {
                Some(tags) if tags.iter().any(|t| t == "targets_random") => {
                    let index = rng.below(sinners.len() as u32) as usize;
                    &sinners[index]
                }
                Some(tags) if tags.iter().any(|t| t == "targets_most_hp") => sinners
                    .iter()
                    .max_by_key(|(_, _, index, _)| state.units[*index].hp)
                    .unwrap(),
                Some(tags) => {
                    let status = tags
                        .iter()
                        .find_map(|t| t.strip_prefix("targets_most_status:"))
                        .map(|s| s.to_string());
                    match status {
                        Some(status) => sinners
                            .iter()
                            .max_by_key(|(_, _, index, _)| {
                                let unit = &state.units[*index];
                                unit.statuses.potency(&status) + unit.statuses.count(&status)
                            })
                            .unwrap(),
                        None => &sinners[slot % sinners.len()],
                    }
                }
                None => &sinners[slot % sinners.len()],
            };
            Some(pick.3.clone())
        })
        .collect()
}

pub fn end_turn(state: &mut BattleState, mechanics: &MechanicsBook) {
    state.phase = Phase::TurnEnd;
    // The Skills the unit used read their "[Turn End]" clauses **before** the
    // statuses tick down, so a clause can still see the [Bind] it converts
    // ("Apply 1 [Haste] next turn to ([Bind] on self / 2) other allies").
    apply_dashboard_phase(state, mechanics, false);
    // Status upkeep: "[Turn End]" clauses of every status the unit holds.
    if let Some(book) = state.status_book.clone() {
        apply_status_phase(state, &book, false);
    }
    // The Turn End half of a Panic Type ("Turn End: Gain 1 [Bind] ...").
    apply_panic_turn_end(state);
    for index in 0..state.units.len() {
        if !state.units[index].alive {
            continue;
        }
        apply_passive_phase(state, index, None, |m| &m.turn_end);
    }
    // "[Attack End] For N turns, lose X SP at Combat End".
    for unit in state.units.iter_mut() {
        if unit.combat_end_sp_loss.is_empty() {
            continue;
        }
        let total: i32 = unit.combat_end_sp_loss.iter().map(|(amount, _)| *amount).sum();
        let sanity = unit.sanity;
        unit.sanity = sanity.add(-total);
        unit.combat_end_sp_loss.retain_mut(|(_, turns)| {
            *turns -= 1;
            *turns > 0
        });
    }
    // Segmentation: a butterfly that was never hit this turn gives the Imago
    // +N Stacks at Combat End.
    for index in 0..state.units.len() {
        let Some(segmentation) = state.units[index].segmentation.clone() else {
            continue;
        };
        if state.units[index].hits_taken == 0 {
            let entry = state
                .campaign
                .time_stacks
                .entry(segmentation.stack_status.clone())
                .or_insert(0);
            *entry += segmentation.gain_if_not_hit;
        }
    }
    state.defenses.clear();
    for unit in state.units.iter_mut() {
        unit.statuses.remove("No Damage Taken");
        unit.retaliate_on_hit.clear();
    }
    if state.encounter_ended {
        state.winner = Some(Winner::EncounterEnded);
        state.phase = Phase::Finished;
        return;
    }
    for index in 0..state.units.len() {
        if !state.units[index].alive {
            continue;
        }
        // Burn: end of turn, fixed damage by Potency, then Count -1.
        let burn = state.units[index].statuses.potency("Burn");
        if burn > 0 {
            state.units[index].take_damage(burn);
            tick_status(state, index, "Burn", 0, -1);
        }
        // Poise: end of turn, Count -1.
        if state.units[index].statuses.count("Poise") > 0 {
            tick_status(state, index, "Poise", 0, -1);
        }
        // Tremor: "At the end of the turn, reduce the Count by 1."
        if state.units[index].statuses.count("Tremor") > 0 {
            tick_status(state, index, "Tremor", 0, -1);
        }
        // Charge: "Count lowers by 1 at the end of each turn."
        if state.units[index].statuses.count("Charge") > 0 {
            tick_status(state, index, "Charge", 0, -1);
        }
        // Butterfly: "Turn End: Reset The Departed of this effect to 0; then,
        // gain [Sinking] equal to The Living and convert The Living into The
        // Departed."  Source: in-game `BattleKeywords` (SinkingWhite).
        let living = state.units[index].statuses.potency("Butterfly");
        let departed = state.units[index].statuses.count("Butterfly");
        if living > 0 || departed > 0 {
            if living > 0 {
                state.units[index].statuses.add_potency("Sinking", living);
            }
            state.units[index].statuses.set(
                "Butterfly",
                crate::state::StatusInstance {
                    potency: 0,
                    count: living,
                    stack: 0,
                },
            );
        }
        // Stagger recovery.
        if state.units[index].stagger.turns_remaining > 0 {
            state.units[index].stagger.turns_remaining -= 1;
            if state.units[index].stagger.turns_remaining == 0 {
                state.units[index].stagger.level = 0;
            }
        }
    }
    // Victory check.
    let sinners_alive = state.living_sinners().len();
    let enemies_alive = state.living_enemies().len();
    if enemies_alive == 0 && sinners_alive > 0 {
        state.winner = Some(Winner::Sinners);
        state.phase = Phase::Finished;
    } else if sinners_alive == 0 && enemies_alive > 0 {
        state.winner = Some(Winner::Enemies);
        state.phase = Phase::Finished;
    } else if sinners_alive == 0 && enemies_alive == 0 {
        state.winner = Some(Winner::Draw);
        state.phase = Phase::Finished;
    } else {
        state.phase = Phase::TurnStart;
    }
}

/// "the target that has the highest HP": the other units of the **opposing**
/// side (a Sinner's Skill reuses onto another enemy and vice versa).
fn next_reuse_target(state: &BattleState, defender_index: usize, attacker_index: usize) -> Option<usize> {
    let attacker_is_sinner = state.units[attacker_index].kind.is_sinner();
    state
        .units
        .iter()
        .enumerate()
        .filter(|(index, unit)| {
            unit.alive
                && *index != defender_index
                && *index != attacker_index
                && unit.kind.is_sinner() != attacker_is_sinner
        })
        .max_by_key(|(_, unit)| unit.hp)
        .map(|(index, _)| index)
}

/// Can this unit still take the action it submitted this turn?  A unit that died,
/// got Staggered or Panicked earlier in the same turn drops the rest of its
/// action ("[T]he queue is checked again when the turn comes").
/// "[Unclashable]" - the Skill cannot be matched against anything; the unit
/// attacks or is attacked on its own.
fn is_unclashable(mechanics: &MechanicsBook, action: &SubmittedAction) -> bool {
    mechanics
        .get_for(&action.skill, Uptie(4))
        .map(|list| list.tags.iter().any(|tag| tag == "unclashable"))
        .unwrap_or(false)
}

fn can_act_now(state: &BattleState, index: usize) -> bool {
    let unit = &state.units[index];
    if !unit.alive || unit.panicked {
        return false;
    }
    if unit.is_staggered() && !unit.acts_while_staggered {
        return false;
    }
    true
}

/// Resolve the combat phase: pair up skills into clashes in speed order.
pub fn resolve_combat(state: &mut BattleState, library: &Library, mechanics: &MechanicsBook) -> Vec<ClashResult> {
    let mut results = Vec::new();
    let mut actions = state.actions.clone();
    // A Corroding Sinner "will go out of control and use E.G.O Corrosion Skills
    // indiscriminately": their submitted action is replaced by the Corrosion
    // Skill of one of their E.G.O, aimed at a random enemy (wiki.gg `Sanity`).
    for action in actions.iter_mut() {
        let Some(actor) = state.index_of(&action.actor) else { continue };
        if !state.units[actor].corroded {
            continue;
        }
        let Some(ego) = state.units[actor].corrosion_egos.first().cloned() else {
            continue;
        };
        let Some(record) = library.ego(&ego) else { continue };
        if record.corrosion.is_none() {
            continue;
        }
        // E.G.O Skills are keyed `<ego id>.awakening` / `<ego id>.corrosion`.
        let skill = SkillId::new(format!("{}.corrosion", ego.as_str()));
        let enemies = state.living_enemies();
        if enemies.is_empty() {
            continue;
        }
        let pick = state.rng.below(enemies.len() as u32) as usize;
        action.skill = skill;
        action.target = Some(enemies[pick].clone());
        action.is_ego = true;
        action.ego = Some(ego);
        action.ego_kind = Some(EgoSkillKind::Corrosion);
    }
    let mut pending_draft: Vec<(usize, SubmittedAction, Option<usize>)> = Vec::new();
    for action in actions {
        let Some(actor) = state.index_of(&action.actor) else { continue };
        if !state.units[actor].alive {
            continue;
        }
        if state.units[actor].is_staggered() && !state.units[actor].acts_while_staggered {
            continue;
        }
        // Panic: "Does not act for this turn" (wiki.gg `Sanity`).
        if state.units[actor].panicked {
            state.push_log(
                "panic",
                format!("{} is Panicking and does not act", state.units[actor].name),
            );
            continue;
        }
        let target = action.target.as_ref().and_then(|t| state.index_of(t));
        pending_draft.push((actor, action, target));
    }
    // Sort by speed (descending), then by deployment order.
    pending_draft.sort_by_key(|(index, _, _): &(usize, SubmittedAction, Option<usize>)| {
        let speed = state.units[*index].speed;
        let order = state
            .deployment
            .iter()
            .position(|id| id == &state.units[*index].id)
            .unwrap_or(usize::MAX);
        (std::cmp::Reverse(speed), order)
    });

    compute_resonance(state, library);
    // Passives' "[Combat Start]" clauses: they belong to the turn's Skill
    // selection, so they run here - after the panel is submitted and the Sin
    // Resonance of the selection is known.
    for index in 0..state.units.len() {
        if !state.units[index].alive {
            continue;
        }
        let target = state
            .units
            .iter()
            .position(|u| u.alive && u.kind.is_sinner() != state.units[index].kind.is_sinner());
        apply_passive_phase(state, index, target, |m| &m.combat_start);
    }
    // Defense Skills are not attacks: they arm the unit for the turn.  A
    // "[Clashable Guard]" / "[Clashable Counter]" stays in the queue, so it can
    // be matched against an enemy attack like any other Skill, and only arms the
    // unit when nothing clashes with it (wiki.gg `Battles` / Defense Skills).
    let mut pending: Vec<(usize, SubmittedAction, Option<usize>)> = Vec::new();
    let _ = &mut pending;
    let mut clashable_defenses: Vec<(usize, u32, DefenseKind)> = Vec::new();
    for (index, action, target) in pending_draft {
        if let Some(kind) = action_defense_kind(state, library, &action) {
            let mut use_ = build_action_use(state, library, mechanics, index, &action);
            let mut is_clashable = false;
            if let Some(use_) = use_.as_mut() {
                prepare_use(state, library, mechanics, index, target, action.slot, use_);
                is_clashable = use_.ctx.clashable_defense;
            }
            if is_clashable {
                clashable_defenses.push((pending.len(), action.slot, kind));
                pending.push((index, action, target));
                continue;
            }
            if let Some(use_) = use_.as_ref() {
                arm_defense(state, index, action.slot, kind, use_);
            }
            continue;
        }
        pending.push((index, action, target));
    }
    // "味方のスキルによって敵のスキルの使用先を変更させるのを同じスロットに対して
    // 複数回行った場合、敵のスキルの使用先は当然「最後に行った使用先の変更」に準拠
    // する" (JA-wiki 戦闘システム詳細): when several Skills chain to the same enemy
    // Slot, the last one submitted owns it.
    let mut pull_owner: std::collections::BTreeMap<u32, crate::ids::UnitId> =
        std::collections::BTreeMap::new();
    for action in state.actions.iter() {
        if let Some(slot) = action.enemy_slot {
            pull_owner.insert(slot, action.actor.clone());
        }
    }
    let mut done: Vec<bool> = vec![false; pending.len()];
    // (unit index, slot) of skills that were actually used this turn; these are
    // the slot rotations the panel performs at the end of the turn.
    let mut executed: Vec<(usize, u32)> = Vec::new();
    for i in 0..pending.len() {
        if done[i] {
            continue;
        }
        let (actor_i, action_i, target_i) = pending[i].clone();
        // The unit may have been Staggered, killed or Panicked by an earlier
        // action of this same turn, so its turn is re-validated here.
        if !can_act_now(state, actor_i) {
            done[i] = true;
            continue;
        }
        let mut opponent_slot = None;
        let actor_unclashable = is_unclashable(mechanics, &action_i);
        // An explicit chain to an enemy Skill Slot (focused encounters) pairs
        // with exactly that Slot.
        if let Some(wanted) = action_i
            .enemy_slot
            .filter(|_| !actor_unclashable)
            .filter(|slot| {
            pull_owner
                .get(slot)
                .map(|owner| *owner == state.units[actor_i].id)
                .unwrap_or(false)
        }) {
            for (j, (actor_j, action_j, _)) in pending.iter().enumerate() {
                if done[j] || *actor_j == actor_i || action_j.slot != wanted {
                    continue;
                }
                if state.units[*actor_j].kind.is_sinner() {
                    continue;
                }
                opponent_slot = Some(j);
                break;
            }
        }
        // Otherwise: does the target also act against the actor with an attack
        // skill?  A Slot that someone else chained to is spoken for, so it can
        // no longer be picked up through its old target.
        if opponent_slot.is_none() && !actor_unclashable {
            if let Some(target) = target_i {
                for (j, (actor_j, action_j, target_j)) in pending.iter().enumerate() {
                    if done[j] || *actor_j != target {
                        continue;
                    }
                    if *target_j != Some(actor_i) {
                        continue;
                    }
                    // "[Unclashable]" Skills are never matched.
                    if is_unclashable(mechanics, action_j) {
                        continue;
                    }
                    let taken_by_other = pull_owner
                        .get(&action_j.slot)
                        .map(|owner| *owner != state.units[actor_i].id)
                        .unwrap_or(false);
                    if taken_by_other {
                        continue;
                    }
                    opponent_slot = Some(j);
                    break;
                }
            }
        }
        match opponent_slot {
            Some(j) => {
                if !state.units[actor_i].alive {
                    done[i] = true;
                    continue;
                }
                let (actor_j, action_j, _) = pending[j].clone();
                // A Clash needs both sides able to act ("A unit that was killed
                // or Staggered earlier in the turn drops its action").
                if !can_act_now(state, actor_j) {
                    // The partner dropped out, so this Skill falls back to a
                    // one-sided attack below.
                    done[j] = true;
                    continue;
                }
                let mut use_a = build_action_use(state, library, mechanics, actor_i, &action_i);
                let mut use_b = build_action_use(state, library, mechanics, actor_j, &action_j);
                if let (Some(a), Some(b)) = (use_a.as_mut(), use_b.as_mut()) {
                    let pre_a = prepare_use(state, library, mechanics, actor_i, target_i, action_i.slot, a);
                    let pre_b = prepare_use(state, library, mechanics, actor_j, Some(actor_i), action_j.slot, b);
                    let _ = (pre_a, pre_b);
                    {
                        let key = clash_key(state, actor_i, actor_j);
                        *state.clash_counts.entry(key).or_insert(0) += 1;
                    }
                    let outcome = resolve_clash(state, actor_i, actor_j, a, b);
                    if outcome.winner.as_ref() == Some(&state.units[actor_i].id) {
                        apply_clash_result(state, actor_i, Some(actor_j), a, true);
                        apply_clash_result(state, actor_j, Some(actor_i), b, false);
                        if let Some(target) = target_i {
                            let clash_count = outcome.rounds;
                            let hits = one_sided_attack(state, actor_i, target, a, clash_count);
                            splash_attack(state, actor_i, target, a, &hits, clash_count);
                            // The winner's attack ends like any other attack: its
                            // "[Attack End]" clauses must resolve here too, or a
                            // Skill behaves differently depending on whether it
                            // Clashed (wiki.gg `Clash` / Skill use phases).
                            apply_attack_end(state, actor_i, Some(target), action_i.slot, a);
                            clash_loser_follow_up(state, actor_j, actor_i, action_j.slot, b, clash_count);
                        }
                    } else if outcome.winner.as_ref() == Some(&state.units[actor_j].id) {
                        apply_clash_result(state, actor_j, Some(actor_i), b, true);
                        apply_clash_result(state, actor_i, Some(actor_j), a, false);
                        let clash_count = outcome.rounds;
                        let hits = one_sided_attack(state, actor_j, actor_i, b, clash_count);
                        splash_attack(state, actor_j, actor_i, b, &hits, clash_count);
                        apply_attack_end(state, actor_j, Some(actor_i), action_j.slot, b);
                        if ends_encounter(state, actor_j, &b.skill) {
                            state.encounter_ended = true;
                            state.push_log("end", format!("{} ended the encounter", b.name));
                        }
                        clash_loser_follow_up(state, actor_i, actor_j, action_i.slot, a, clash_count);
                    } else {
                        apply_clash_result(state, actor_i, Some(actor_j), a, false);
                        apply_clash_result(state, actor_j, Some(actor_i), b, false);
                    }
                    state.push_log(
                        "clash",
                        format!(
                            "{} ({}) vs {} ({}) over {} round(s); winner: {}",
                            a.name,
                            a.base_power,
                            b.name,
                            b.base_power,
                            outcome.rounds,
                            outcome
                                .winner
                                .as_ref()
                                .map(|w| w.to_string())
                                .unwrap_or_else(|| "draw".to_string())
                        ),
                    );
                    results.push(outcome);
                }
                executed.push((actor_i, action_i.slot));
                executed.push((actor_j, action_j.slot));
                done[i] = true;
                done[j] = true;
            }
            None => {
                if !can_act_now(state, actor_i) {
                    done[i] = true;
                    continue;
                }
                // A clashable defense that found no partner arms the unit.
                if let Some((_, slot, kind)) = clashable_defenses
                    .iter()
                    .find(|(pending_index, _, _)| *pending_index == i)
                    .copied()
                {
                    if let Some(use_) =
                        build_action_use(state, library, mechanics, actor_i, &action_i)
                    {
                        arm_defense(state, actor_i, slot, kind, &use_);
                    }
                    done[i] = true;
                    executed.push((actor_i, action_i.slot));
                    continue;
                }
                if let Some(target) = target_i.filter(|t| state.units[*t].alive) {
                    if let Some(mut use_) =
                        build_action_use(state, library, mechanics, actor_i, &action_i)
                    {
                        prepare_use(state, library, mechanics, actor_i, Some(target), action_i.slot, &mut use_);
                        let hits = one_sided_attack(state, actor_i, target, &mut use_, 0);
                        splash_attack(state, actor_i, target, &mut use_, &hits, 0);
                        apply_attack_end(state, actor_i, Some(target), action_i.slot, &mut use_);
                        let total: i32 = hits.iter().map(|h| h.damage).sum();
                        let rolls: Vec<i32> = hits.iter().map(|h| h.power).collect();
                        state.push_log(
                            "attack",
                            format!(
                                "{} -> {} hit(s), {} damage, coin rolls {:?}",
                                use_.name,
                                hits.len(),
                                total,
                                rolls
                            ),
                        );
                        executed.push((actor_i, action_i.slot));
                        if ends_encounter(state, actor_i, &use_.skill)
                            && !(state.units[actor_i].ends_encounter_unless_shield_broken
                                && state.units[actor_i].barrier_broken)
                        {
                            state.encounter_ended = true;
                            state.push_log(
                                "end",
                                format!("{} ended the encounter", use_.name),
                            );
                        }
                    }
                }
                done[i] = true;
            }
        }
    }
    rotate_used_slots(state, &executed);
    results
}

/// The panel keeps two skills per slot.  Using the bottom skill consumes it,
/// the top skill rotates down and a new skill is drawn into the top.  Slots
/// whose skill was never used are left untouched.
/// Source: Japanese wiki `戦闘システム詳細` ("スキル構成", "パネル上に空きが出ると
/// その分だけスキル構成から継ぎ足される").
fn rotate_used_slots(state: &mut BattleState, executed: &[(usize, u32)]) {
    let mut all: Vec<(usize, u32)> = executed.to_vec();
    for entry in state.defense_slots_used.clone() {
        if !all.contains(&entry) {
            all.push(entry);
        }
    }
    state.defense_slots_used.clear();
    for (unit_index, slot) in all.into_iter() {
        let Some(entry) = state.units[unit_index]
            .dashboard
            .iter()
            .find(|s| s.slot == slot)
            .cloned()
        else {
            continue;
        };
        // Which card did the unit actually use?  The panel was left untouched
        // when the action was submitted, so the answer is in the action.
        let actor = state.units[unit_index].id.clone();
        let used_top = state
            .actions
            .iter()
            .find(|a| a.actor == actor && a.slot == slot)
            .map(|a| a.used_top)
            .unwrap_or(false);
        if used_top {
            // The top card is consumed; the bottom card stays available and the
            // panel refills from the top.
            state.units[unit_index].deck.consume(&entry.next);
        } else if let Some(replaced) = entry.replaced.clone() {
            // A defense skill / E.G.O replaced the bottom card: both leave the
            // panel, so the card that was replaced is the one consumed.
            state.units[unit_index].deck.consume(&replaced);
        } else {
            // The bottom card is consumed: the top card rotates down.
            state.units[unit_index].deck.consume(&entry.current);
        }
        let drawn = crate::setup::draw_for_unit(state, unit_index)
            .unwrap_or_else(crate::setup::empty_skill);
        if let Some(target) = state.units[unit_index]
            .dashboard
            .iter_mut()
            .find(|s| s.slot == slot)
        {
            if used_top {
                target.next = entry.preview.clone();
                target.preview = drawn;
            } else {
                target.current = entry.next.clone();
                target.next = entry.preview.clone();
                target.preview = drawn;
            }
            target.converted = false;
            target.replaced = None;
            target.target = None;
        }
    }
}

/// What happens to the Skill that lost a Clash.
///
/// Its **Unbreakable Coins** are not destroyed by the Clash: they come back
/// "cracked" (Coin Power 1) and still land, so the order of a won Clash is
/// "our attack, then theirs" (JA-wiki 破壊不能コイン: 「破壊不能コインが含まれる攻撃
/// スキルにマッチ勝利すると、こちらの攻撃→相手の攻撃 という順で処理が行われる」).
///
/// "[Attack End] activates only once after an Attack Skill has used all of its
/// Coins" (wiki.gg `Clash`): Coins destroyed in the Clash do not count as used,
/// while Unbreakable Coins do, so a Skill that lost the Clash only reaches its
/// Attack End when every Coin it had was an Unbreakable Coin.
fn clash_loser_follow_up(
    state: &mut BattleState,
    loser_index: usize,
    winner_index: usize,
    loser_slot: u32,
    loser_use: &mut SkillUse,
    clash_count: i32,
) {
    let cracked = loser_use.cracked_coins();
    let all_unbreakable = !loser_use.coins.is_empty()
        && loser_use.coins.iter().all(|coin| coin.unbreakable);
    if !cracked.is_empty() && state.units[loser_index].alive {
        let hits = one_sided_attack(state, loser_index, winner_index, loser_use, clash_count);
        splash_attack(state, loser_index, winner_index, loser_use, &hits, clash_count);
    }
    if all_unbreakable {
        apply_attack_end(state, loser_index, Some(winner_index), loser_slot, loser_use);
    }
}

/// Skills whose Attack End ends the encounter (the Pupa's "The Quickening",
/// "Eclosion").  The stage is over, so the battle is marked as finished.
fn ends_encounter(state: &BattleState, unit_index: usize, skill: &SkillId) -> bool {
    state.units[unit_index]
        .ends_encounter_on
        .iter()
        .any(|id| id == skill.as_str())
}

/// Sin Resonance over the Skills selected on the Dashboard.
///
/// "occurs when 2 or more Skills of the same Affinity are selected on the
/// Dashboard"; Absolute Sin Resonance "occurs when 3 or more Skills of the same
/// Affinity are selected consecutively"; "Separate chains ... are counted
/// separately rather than in a sum" and "Absolute Sin Resonance also counts as
/// regular Sin Resonance".  Source: wiki.gg `Resonance`.
fn compute_resonance(state: &mut BattleState, library: &Library) {
    state.resonance.clear();
    state.a_resonance.clear();
    let mut table: Vec<(u32, u32)> = Vec::new();
    for unit in state.units.iter().filter(|u| u.kind.is_sinner()) {
        for action in state.actions.iter().filter(|a| a.actor == unit.id) {
            let Some(sin) = sin_of(library, &action.skill) else {
                continue;
            };
            table.push((action.slot, sin.index() as u32));
        }
    }
    // Sorted by slot index only (the Dashboard reads left to right); a stable
    // sort keeps the order the Skills were submitted in within a Slot, which is
    // what the Resonance chain is read from.
    table.sort_by_key(|(slot, _)| *slot);
    for (_, index) in table.iter() {
        *state
            .resonance
            .entry(sin_key_by_index(*index).to_string())
            .or_insert(0) += 1;
    }
    // Longest run of consecutive equal affinities.
    let mut run_key: Option<u32> = None;
    let mut run_len = 0;
    for (_, index) in table.iter() {
        if Some(*index) == run_key {
            run_len += 1;
        } else {
            run_key = Some(*index);
            run_len = 1;
        }
        if let Some(key) = run_key {
            let entry = state
                .a_resonance
                .entry(sin_key_by_index(key).to_string())
                .or_insert(0);
            *entry = (*entry).max(run_len);
        }
    }
    // The units read the finished tables (an Absolute Resonance value copied
    // before the chains were measured would always be 0).
    let highest = highest_resonance(state);
    let longest = state.a_resonance.values().copied().max().unwrap_or(0);
    let counts = state.resonance.clone();
    for unit in state.units.iter_mut() {
        unit.resonance_max = highest;
        unit.a_reson_max = longest;
        unit.resonance_of = counts.clone();
    }
}

/// Affinity of a Skill, looked up in the library (identity skills first, then
/// enemy skills).
fn sin_of(library: &Library, skill: &SkillId) -> Option<Sin> {
    for identity in library.identities.values() {
        if let Some(record) = identity.skills.iter().find(|s| s.id == skill.0) {
            return record.sin(Uptie(4));
        }
    }
    for enemy in library.enemies.values() {
        if let Some(record) = enemy
            .skills
            .iter()
            .find(|s| s.skill_id() == skill.0 || s.display_name() == skill.0)
        {
            return record.sin();
        }
    }
    // E.G.O Skills are submitted as `<ego id>.awakening` / `<ego id>.corrosion`
    // and take part in Resonance like any other Skill.
    if let Some(record) = library.ego(&crate::ids::EgoId::new(skill.0.clone())) {
        // An E.G.O action is submitted under its bare id; Awakening and
        // Corrosion share the same affinity.
        if let Some(part) = record.awakening.as_ref().or(record.corrosion.as_ref()) {
            return part.sin();
        }
    }
    if let Some((ego_id, kind)) = skill.0.split_once('.') {
        if let Some(record) = library.ego(&crate::ids::EgoId::new(ego_id)) {
            let part = if kind == "corrosion" {
                record.corrosion.as_ref()
            } else {
                record.awakening.as_ref()
            };
            if let Some(part) = part {
                return part.sin();
            }
        }
    }
    None
}

fn sin_key_by_index(index: u32) -> &'static str {
    match index {
        0 => "wrath",
        1 => "lust",
        2 => "sloth",
        3 => "gluttony",
        4 => "gloom",
        5 => "pride",
        _ => "envy",
    }
}

/// Highest Resonance among all affinities this turn.
pub fn highest_resonance(state: &BattleState) -> i32 {
    state.resonance.values().copied().max().unwrap_or(0)
}

/// Discard the second visible Skill of a slot (the wiki's "the other Skill in
/// the same Skill Slot"), refilling the panel from the composition.
fn discard_from_slot(state: &mut BattleState, unit_index: usize, slot: u32, only_if_different: bool) {
    let Some(entry) = state.units[unit_index]
        .dashboard
        .iter()
        .find(|s| s.slot == slot)
        .cloned()
    else {
        return;
    };
    if only_if_different && entry.next == entry.current {
        return;
    }
    let discarded = entry.next.clone();
    state.units[unit_index].deck.consume(&discarded);
    let drawn = crate::setup::draw_for_unit(state, unit_index)
        .unwrap_or_else(crate::setup::empty_skill);
    if let Some(target) = state.units[unit_index]
        .dashboard
        .iter_mut()
        .find(|s| s.slot == slot)
    {
        target.next = target.preview.clone();
        target.preview = drawn;
    }
    state.push_log("discard", format!("Discarded {}", discarded));
}

/// "[Discard] N Skills of the lowest rank in all of this unit's Skill Slots".
fn discard_lowest_rank(state: &mut BattleState, unit_index: usize, count: i32) {
    let rank_of = |skill: &SkillId| -> u8 {
        state.units[unit_index]
            .dashboard
            .iter()
            .find(|s| &s.current == skill || &s.next == skill)
            .map(|_| 0)
            .unwrap_or(0)
    };
    let _ = rank_of;
    let slots: Vec<u32> = state.units[unit_index]
        .dashboard
        .iter()
        .map(|s| s.slot)
        .collect();
    // The rank comes from the library; without it, discard the current skills of
    // the first slots (documented limitation).
    for slot in slots.into_iter().take(count.max(0) as usize) {
        let Some(entry) = state.units[unit_index]
            .dashboard
            .iter()
            .find(|s| s.slot == slot)
            .cloned()
        else {
            continue;
        };
        state.units[unit_index].deck.consume(&entry.current);
        let drawn = crate::setup::draw_for_unit(state, unit_index)
            .unwrap_or_else(crate::setup::empty_skill);
        if let Some(target) = state.units[unit_index]
            .dashboard
            .iter_mut()
            .find(|s| s.slot == slot)
        {
            target.current = target.next.clone();
            target.next = target.preview.clone();
            target.preview = drawn;
        }
    }
}

/// Classify a submitted action as a defense skill, if it is one.
fn action_defense_kind(
    state: &BattleState,
    library: &Library,
    action: &SubmittedAction,
) -> Option<DefenseKind> {
    if action.is_ego {
        return None;
    }
    let unit = state.unit(&action.actor)?;
    let UnitKind::Sinner { identity } = &unit.kind else { return None };
    let record = library.identity(identity)?;
    let skill = record.skills.iter().find(|s| s.id == action.skill.0)?;
    if skill.slot() != Some(crate::ids::SkillSlot::Defense) {
        return None;
    }
    Some(match skill.kind.as_deref() {
        Some("Guard") => DefenseKind::Guard,
        Some("Evade") => DefenseKind::Evade,
        _ => DefenseKind::Counter,
    })
}

/// Arm a defense Skill for the turn (Guard Shield, Evade coins, Counter stance).
fn arm_defense(
    state: &mut BattleState,
    index: usize,
    slot: u32,
    kind: DefenseKind,
    use_: &SkillUse,
) {
    state.defenses.push(ActiveDefense {
        unit: state.units[index].id.clone(),
        kind,
        skill: use_.skill.clone(),
        name: use_.name.clone(),
        sin: use_.sin,
        damage_type: use_.damage_type,
        base_power: use_.base_power,
        coin_power: use_.coin_power,
        offense_level_mod: use_.offense_level_mod,
        coins: use_.coins.clone(),
        mechanics: use_.mechanics.clone(),
        ctx: use_.ctx.clone(),
        lost: false,
        activated: false,
        defense_level_mod: use_.defense_level_mod.unwrap_or(0),
    });
    // Guards are not applied here: the Shield is gained when the unit is first
    // attacked this turn (see `activate_guard`).
    // The defense Skill consumes the Slot: it and the card it replaced leave the
    // panel ("使った守備スキルは守備スキルに変更した元のスキルごとパネルから消
    // え"), so the Slot rotates even when the defense never fires.
    state.defense_slots_used.push((index, slot));
}

/// Build a use from a submitted action, paying E.G.O costs when needed.
fn build_action_use(
    state: &mut BattleState,
    library: &Library,
    mechanics: &MechanicsBook,
    unit_index: usize,
    action: &SubmittedAction,
) -> Option<SkillUse> {
    if let (Some(ego_id), Some(kind)) = (action.ego.clone(), action.ego_kind) {
        let record = library.ego(&ego_id)?.clone();
        let mut use_ = build_ego_use(state, library, mechanics, unit_index, &ego_id, kind)?;
        use_.slot = action.slot;
        // The resources are spent here, the **SP** cost is paid at [Before Attack]
        // (see `one_sided_attack`): paying it before the Clash would lower the
        // Heads rate of the Clash it takes part in.
        let sp_cost = ego_sp_cost(&record, kind);
        use_.ctx.ego_sp_pending = Some(sp_cost);
        let mut detail = pay_ego(state, unit_index, &record, kind);
        // Random Corrosion: with negative SP an E.G.O may fire its Corrosion
        // Skill instead of its Awakening one.  The chance depends on the SP left
        // after paying: -24 or higher 0%, -25..-34 25%, -35..-44 75%, -45 100%
        // (JA-wiki 戦闘システム詳細 / ランダム侵蝕の仕様).
        let projected_sp = state.units[unit_index].sanity.sp() - sp_cost;
        if let Some(chance) = corrosion_chance_for_sp(projected_sp)
        {
            if state.flip(chance) {
                if let Some(corrosion) = build_ego_use(
                    state,
                    library,
                    mechanics,
                    unit_index,
                    &ego_id,
                    EgoSkillKind::Corrosion,
                ) {
                    let mut corrosion = corrosion;
                    corrosion.slot = action.slot;
                    corrosion.ctx.indiscriminate = true;
                    // "オーバークロックは…余計に払ったE.G.O資源が返却される":
                    // a random Corrosion during an Overclock refunds the surcharge
                    // and costs the normal Corrosion price.
                    if kind == EgoSkillKind::Overclock {
                        refund_overclock_surcharge(state, &record);
                        use_ = corrosion;
                        detail = format!("{detail} (random Corrosion; Overclock surcharge refunded)");
                    } else {
                        use_ = corrosion;
                        detail = format!("{detail} (random Corrosion)");
                    }
                }
            }
        }
        state.push_log("ego", format!("{} -> {}", use_.name, detail));
        return Some(use_);
    }
    let mut use_ = build_use(state, library, mechanics, unit_index, &action.skill)?;
    use_.slot = action.slot;
    Some(use_)
}

/// Run `[On Use]` / `[Combat Start]` effects and apply the resulting context.
fn prepare_use(
    state: &mut BattleState,
    library: &Library,
    mechanics: &MechanicsBook,
    unit_index: usize,
    target_index: Option<usize>,
    slot: u32,
    use_: &mut SkillUse,
) -> UseContext {
    let _ = (library, mechanics);
    // "When attacking just a single target" clauses read how many units this
    // Skill use reaches (its Atk Weight against the units still standing).
    {
        let actor_is_sinner = state.units[unit_index].kind.is_sinner();
        let standing = state
            .units
            .iter()
            .filter(|unit| unit.alive && unit.kind.is_sinner() != actor_is_sinner)
            .count() as u32;
        let weight = use_.attack_weight.max(1);
        state.units[unit_index].planned_targets = weight.min(standing.max(1)) as usize;
    }
    // "Base Power +1 for every [X] about to be spent by this Skill": the amount
    // this use will spend is known from its own effects.
    {
        let planned: i32 = use_
            .mechanics
            .on_use
            .iter()
            .filter(|e| e.kind == "spend_ammo")
            .map(|e| e.value.unwrap_or(1))
            .sum::<i32>()
            + use_
                .mechanics
                .coins
                .values()
                .flatten()
                .filter(|e| e.kind == "spend_ammo")
                .map(|e| e.value.unwrap_or(1))
                .sum::<i32>();
        use_.ctx.ammo_planned = planned;
    }
    let mut notes = Vec::new();
    let mut ctx = EffectContext {
        actor_index: unit_index,
        target_index,
        clash_count: 0,
            clash_lost: false,
        slot,
        mechanics_note: &mut notes,
    };
    // Phase order per the wiki: [Combat Start] (the Skill chosen for this
    // turn) -> [On Use] -> [Before Attack] -> Coin tosses.
    let combat_start = use_.mechanics.combat_start.clone();
    if !combat_start.is_empty() {
        apply_effects(state, &combat_start, &mut ctx, &mut use_.ctx);
    }
    let on_use = use_.mechanics.on_use.clone();
    apply_effects(state, &on_use, &mut ctx, &mut use_.ctx);
    let shield = use_.ctx.shield_gain;
    if shield > 0 {
        state.units[unit_index].shield += shield;
    }
    for tag in use_.mechanics.tags.clone() {
        if let Some(status) = tag.strip_prefix("status_count_floor:") {
            use_.ctx.status_count_floor.push(status.to_string());
        }
        if tag == "no_stagger_target" {
            use_.ctx.no_stagger_target = true;
        }
        if tag == "random_coin_targets" {
            use_.ctx.random_coin_targets = true;
        }
        if tag == "butterfly_split" {
            use_.ctx.butterfly_split = true;
        }
        if tag == "clashable_defense" {
            use_.ctx.clashable_defense = true;
        }
    }
    // "[Before Attack]" clauses resolve after On Use and before the first toss.
    let before_attack = use_.mechanics.before_attack.clone();
    if !before_attack.is_empty() {
        let mut ctx = EffectContext {
            actor_index: unit_index,
            target_index,
            clash_count: 0,
            clash_lost: false,
            slot,
            mechanics_note: &mut notes,
        };
        apply_effects(state, &before_attack, &mut ctx, &mut use_.ctx);
    }
    // "Hand - Pine Crane Suit: Skill 1 Base Power +2" and friends: the Suit in
    // this unit's Hand boosts the Base Power of the matching Skill.
    {
        let suit = state.units[unit_index].suit.clone();
        let skill_index: i32 = use_
            .skill
            .as_str()
            .chars()
            .rev()
            .take(2)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        let bonus = match (suit.as_deref(), skill_index) {
            (Some("HanafudaOne"), 1) => 2,
            (Some("HanafudaTwo"), 2) => 1,
            (Some("HanafudaThree"), 3) => 1,
            _ => 0,
        };
        use_.ctx.base_power_bonus += bonus;
    }
    use_.attack_weight = (use_.attack_weight as i32 + use_.ctx.attack_weight_bonus).max(1) as u32;
    // Continuous passive modifiers: "Deal +5% damage for every [Protection] on
    // self (max 15%)", "Deal +(-SP/2)% damage with Base Skills (max 20%)".
    {
        let (outgoing, _) = passive_modifiers(state, unit_index, target_index);
        use_.ctx.damage_bonus += outgoing;
    }
    use_.ctx.sin = use_.sin;
    // `Plus Coin Boost` / `Minus Coin Drop` are read when the skill is used.
    {
        let statuses = &state.units[unit_index].statuses;
        use_.ctx.coin_power_boost = statuses.count("Plus Coin Boost");
        use_.ctx.coin_power_drop = statuses.count("Minus Coin Drop");
    }
    // "On Use, an Attack Skill will generate 1 E.G.O Resource of a
    // corresponding Affinity" (wiki.gg `Clash` / Attack Skills).  E.G.O
    // Resources are the Sinners' team pool, so enemy Skills generate nothing.
    if !use_.is_defense && !use_.is_ego && state.units[unit_index].kind.is_sinner() {
        let key = crate::setup::sin_key(use_.sin).to_string();
        *state.ego_resources.entry(key).or_insert(0) += 1;
    }
    for note in notes {
        state.warnings.push(note);
    }
    // "convert the final Coin" / "convert all Coins" into [Unbreakable Coin]s:
    // the listed Coins are 1-based indexes, so each Coin is checked by its own
    // position (checking coin 1 only marked the first Coin for every clause).
    for (index, coin) in use_.coins.iter_mut().enumerate() {
        if use_.ctx.unbreakable_all
            || use_.ctx.unbreakable_coins.contains(&(index as u32 + 1))
        {
            coin.unbreakable = true;
        }
    }
    use_.ctx.clone()
}

fn apply_clash_result(
    state: &mut BattleState,
    unit_index: usize,
    target_index: Option<usize>,
    use_: &mut SkillUse,
    won: bool,
) {
    let slot = use_.slot;
    use_.ctx.lost_clash = !won;
    // SP: the current values are not documented - configurable, default 0.
    let delta = if won {
        state.config.sp_on_clash_win
    } else {
        state.config.sp_on_clash_lose
    };
    match delta {
        Some(value) => {
            let sanity = state.units[unit_index].sanity;
            state.units[unit_index].sanity = sanity.add(value);
        }
        None => {
            let warning = crate::state::UnknownRule::SanityGainOnClash.text().to_string();
            if !state.warnings.contains(&warning) {
                state.warnings.push(warning);
            }
        }
    }
    let list = if won {
        use_.mechanics.clash_win.clone()
    } else {
        use_.mechanics.clash_lose.clone()
    };
    if list.is_empty() {
        return;
    }
    let mut notes = Vec::new();
    let mut ctx = EffectContext {
        actor_index: unit_index,
        target_index,
        clash_count: 0,
            clash_lost: false,
        slot,
        mechanics_note: &mut notes,
    };
    apply_effects(state, &list, &mut ctx, &mut use_.ctx);
    for note in notes {
        state.warnings.push(note);
    }
    // "[Attack End] If target is killed, Reuse this Skill on the target that has
    // the highest HP (once per turn)" (Smite the Wicked).  The clause is written
    // under [Attack End], so the repeat can only start once that list resolved.
    let killed_target = target_index
        .map(|index| !state.units[index].alive)
        .unwrap_or(false);
    if use_.ctx.reuse_on_kill
        && killed_target
        && state.units[unit_index].alive
        && state.units[unit_index]
            .turn_effect_usage
            .get("reuse_on_kill")
            .copied()
            .unwrap_or(0)
            < 1
    {
        state.units[unit_index]
            .turn_effect_usage
            .insert("reuse_on_kill".to_string(), 1);
        if let Some(next) = target_index.and_then(|defender| {
            next_reuse_target(state, defender, unit_index)
        }) {
            let mut repeat = use_.clone();
            for coin in repeat.coins.iter_mut() {
                coin.state = CoinState::Fresh;
                coin.heads = None;
            }
            repeat.ctx.accumulated = 0;
            repeat.ctx.reuse_on_kill = false;
            state.push_log("reuse", format!("{} was reused", use_.name));
            one_sided_attack(state, unit_index, next, &mut repeat, 0);
        }
    }
}

fn apply_attack_end(
    state: &mut BattleState,
    unit_index: usize,
    target_index: Option<usize>,
    slot: u32,
    use_: &mut SkillUse,
) {
    let list = use_.mechanics.attack_end.clone();
    // "[Attack End] If 1 or more targets are killed" reads this Skill's kills.
    state.units[unit_index].skill_kills = use_.ctx.kills;
    if list.is_empty() {
        return;
    }
    let mut notes = Vec::new();
    let mut ctx = EffectContext {
        actor_index: unit_index,
        target_index,
        clash_count: 0,
            clash_lost: false,
        slot,
        mechanics_note: &mut notes,
    };
    apply_effects(state, &list, &mut ctx, &mut use_.ctx);
    for note in notes {
        state.warnings.push(note);
    }
    // "[Attack End] If target is killed, Reuse this Skill on the target that has
    // the highest HP (once per turn)" (Smite the Wicked).  The clause is written
    // under [Attack End], so the repeat can only start once that list resolved.
    let killed_target = target_index
        .map(|index| !state.units[index].alive)
        .unwrap_or(false);
    if use_.ctx.reuse_on_kill
        && killed_target
        && state.units[unit_index].alive
        && state.units[unit_index]
            .turn_effect_usage
            .get("reuse_on_kill")
            .copied()
            .unwrap_or(0)
            < 1
    {
        state.units[unit_index]
            .turn_effect_usage
            .insert("reuse_on_kill".to_string(), 1);
        if let Some(next) = target_index.and_then(|defender| {
            next_reuse_target(state, defender, unit_index)
        }) {
            let mut repeat = use_.clone();
            for coin in repeat.coins.iter_mut() {
                coin.state = CoinState::Fresh;
                coin.heads = None;
            }
            repeat.ctx.accumulated = 0;
            repeat.ctx.reuse_on_kill = false;
            state.push_log("reuse", format!("{} was reused", use_.name));
            one_sided_attack(state, unit_index, next, &mut repeat, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::EncounterBuilder;
    use crate::testsupport::test_library;

    #[test]
    fn clash_winner_keeps_coins() {
        let (library, mechanics) = test_library();
        let mut state = EncounterBuilder::new(&library, &mechanics)
            .seed(1)
            .build(&["10110"], &["9567"])
            .expect("encounter builds");
        let mut a = build_use(&state, &library, &mechanics, 0, &SkillId::new("1011001")).unwrap();
        let mut b = {
            let record = state.units[1].kind.clone();
            let UnitKind::Abnormality { enemy, .. } = record else { unreachable!() };
            let name = library
                .enemy(&enemy)
                .unwrap()
                .skills
                .first()
                .unwrap()
                .display_name();
            build_use(&state, &library, &mechanics, 1, &SkillId::new(name)).unwrap()
        };
        a.base_power = 50;
        b.base_power = 1;
        let result = resolve_clash(&mut state, 0, 1, &mut a, &mut b);
        assert_eq!(result.winner, Some(state.units[0].id.clone()));
        assert_eq!(b.remaining_coins(), 0);
    }

    #[test]
    fn stagger_triggers_below_threshold() {
        let (library, mechanics) = test_library();
        let mut state = EncounterBuilder::new(&library, &mechanics)
            .seed(2)
            .build(&["10110"], &["9567"])
            .unwrap();
        let max = state.units[0].max_hp;
        state.units[0].hp = max * 59 / 100;
        assert!(check_stagger(&mut state, 0));
        assert!(state.units[0].is_staggered());
    }
}

// --------------------------------------------------------------------------- //
// Hooks used by the mechanic tests (not part of the public simulation API).
// --------------------------------------------------------------------------- //

#[doc(hidden)]
pub fn break_coin_for_test(coin: &mut CoinRuntime) {
    break_coin(coin, 0)
}

#[doc(hidden)]
pub fn ego_affordable_for_test(
    state: &BattleState,
    unit_index: usize,
    ego: &crate::library::EgoRecord,
    kind: EgoSkillKind,
) -> bool {
    ego_affordable(state, &state.units[unit_index], ego, kind)
}

#[doc(hidden)]
pub fn apply_attack_end_for_test(
    state: &mut BattleState,
    unit_index: usize,
    target_index: Option<usize>,
    slot: u32,
    use_: &mut SkillUse,
) {
    apply_attack_end(state, unit_index, target_index, slot, use_)
}

#[doc(hidden)]
pub fn pay_ego_for_test(
    state: &mut BattleState,
    unit_index: usize,
    ego: &crate::library::EgoRecord,
    kind: EgoSkillKind,
) {
    pay_ego(state, unit_index, ego, kind);
}

#[doc(hidden)]
pub fn apply_sinking_for_test(state: &mut BattleState, unit_index: usize) {
    apply_sinking(state, unit_index)
}

#[doc(hidden)]
pub fn incoming_damage_modifier_for_test(unit: &Unit, sin_name: &str) -> f64 {
    incoming_damage_modifier(unit, sin_name)
}

#[doc(hidden)]
pub fn apply_effects_for_test(
    state: &mut BattleState,
    effects: &[Effect],
    actor_index: usize,
    target_index: Option<usize>,
    notes: &mut Vec<String>,
    use_ctx: &mut UseContext,
) {
    let mut ctx = EffectContext {
        actor_index,
        target_index,
        clash_count: 0,
            clash_lost: false,
        slot: 0,
        mechanics_note: notes,
    };
    apply_effects(state, effects, &mut ctx, use_ctx);
}

#[doc(hidden)]
pub fn passive_modifiers_for_test(
    state: &BattleState,
    index: usize,
    target: Option<usize>,
) -> (f64, f64) {
    passive_modifiers(state, index, target)
}

#[doc(hidden)]
pub fn splash_attack_for_test(
    state: &mut BattleState,
    attacker_index: usize,
    main_target: usize,
    use_: &mut SkillUse,
    hits: &[HitResult],
    clash_count: i32,
) {
    splash_attack(state, attacker_index, main_target, use_, hits, clash_count);
}

#[doc(hidden)]
pub fn prepare_use_for_test(
    state: &mut BattleState,
    library: &Library,
    mechanics: &MechanicsBook,
    unit_index: usize,
    target_index: Option<usize>,
    use_: &mut SkillUse,
) {
    prepare_use(state, library, mechanics, unit_index, target_index, use_.slot, use_);
}

#[doc(hidden)]
pub fn toss_all_for_test(state: &mut BattleState, unit_index: usize, use_: &mut SkillUse) {
    toss_all(state, unit_index, use_)
}
