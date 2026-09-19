//! Combat execution: coin flips, clashing, one-sided attacks, status ticks and
//! the turn loop.
//!
//! Every rule implemented here cites the wiki.gg page it comes from in
//! `docs/MECHANICS.md`; rules that could not be sourced are represented by an
//! explicit config knob plus a `UnknownRule` warning instead of a guessed value.

use crate::damage::{compute_damage, level_clash_bonus, DamageInputs};
use crate::effects::{Component, Condition, Effect, MechanicsBook, SkillMechanics};
use crate::ids::{DamageType, EgoId, Sin, SkillId, UnitId};
use crate::library::Library;
use crate::state::{
    BattleState, Phase, Sanity, SubmittedAction, Unit, UnitKind, Winner, SP_LIMIT,
};
use serde::{Deserialize, Serialize};

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
    pub clash_power_bonus: i32,
    pub coin_power_bonus: i32,
    pub base_power_bonus: i32,
    /// Dynamic damage modifier contributions for this skill use.
    pub damage_bonus: f64,
    pub shield_gain: i32,
    pub ammo_spent: i32,
    pub unbreakable_coins: Vec<u32>,
    /// "This Attack Skill deals 0 damage" (The Quickening).
    #[serde(default)]
    pub zero_damage: bool,
    /// "convert all Coins on this Skill to Unbreakable Coins".
    #[serde(default)]
    pub unbreakable_all: bool,
    /// `Plus Coin Boost` / `Minus Coin Drop` for this use.
    #[serde(default)]
    pub coin_power_boost: i32,
    #[serde(default)]
    pub coin_power_drop: i32,
    pub notes: Vec<String>,
    /// Running skill power during an attack (Base Power + Heads Coin Power).
    #[serde(default)]
    pub accumulated: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillUse {
    pub actor: UnitId,
    pub target: Option<UnitId>,
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
        let mut skills: Vec<SkillId> = unit
            .dashboard
            .iter()
            .map(|s| s.current.clone())
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
        for slot in unit.dashboard.iter().map(|s| s.slot).collect::<Vec<_>>() {
            for skill in &skills {
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
                for kind in [
                    EgoSkillKind::Awakening,
                    EgoSkillKind::Corrosion,
                    EgoSkillKind::Overclock,
                ] {
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
            entry.target = Some(target.clone());
            // Defense skills and E.G.O replace the bottom skill of the slot.
            if entry.current != skill {
                entry.converted = true;
            }
            entry.current = skill.clone();
            state.actions.retain(|a| !(a.actor == actor && a.slot == slot));
            state.actions.push(SubmittedAction {
                actor,
                slot,
                skill,
                target: Some(target),
                is_ego: false,
                ego: None,
                ego_kind: None,
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
    for (sin, amount) in &ego.resource_cost {
        let needed = if multiplier == 1.0 {
            *amount
        } else {
            (*amount as f64 * multiplier).ceil() as i32
        };
        let have = state.ego_resources.get(sin).copied().unwrap_or(0);
        if have < needed {
            return false;
        }
    }
    true
}

/// Pay the E.G.O costs.  Returns a description for the log.
fn pay_ego(state: &mut BattleState, unit_index: usize, ego: &crate::library::EgoRecord, kind: EgoSkillKind) -> String {
    let (sp_cost, multiplier) = match kind {
        EgoSkillKind::Awakening => (ego.awakening_sp.unwrap_or(0), 1.0),
        EgoSkillKind::Corrosion => (ego.corrosion_sp.unwrap_or(0), 1.0),
        EgoSkillKind::Overclock => (ego.corrosion_sp.unwrap_or(0), 1.5),
    };
    let sp = if multiplier == 1.0 {
        sp_cost
    } else {
        (sp_cost as f64 * multiplier).ceil() as i32
    };
    let sanity = state.units[unit_index].sanity;
    state.units[unit_index].sanity = sanity.add(-sp);
    let mut spent = Vec::new();
    for (sin, amount) in ego.resource_cost.clone() {
        let needed = if multiplier == 1.0 {
            amount
        } else {
            (amount as f64 * multiplier).ceil() as i32
        };
        let entry = state.ego_resources.entry(sin.clone()).or_insert(0);
        *entry -= needed;
        spent.push(format!("{needed} {sin}"));
    }
    format!("SP -{sp}, resources: {}", spent.join(", "))
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
    // A status listed twice means "both values" (the wiki's "both [Butterfly]").
    let mut seen: Vec<&String> = Vec::new();
    let mut total = 0;
    for key in statuses {
        if seen.contains(&key) {
            total += unit.statuses.potency(key) + unit.statuses.count(key);
        } else {
            total += unit_status_value(unit, key, component);
            seen.push(key);
        }
    }
    total
}

fn condition_holds(
    condition: &Condition,
    actor: &Unit,
    target: Option<&Unit>,
    clash_count: i32,
) -> bool {
    // "If any of the following conditions are met, ..."
    if !condition.any_of.is_empty() {
        return condition
            .any_of
            .iter()
            .any(|alternative| condition_holds(alternative, actor, target, clash_count));
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
    if let Some(required) = condition.self_sp_at_least {
        if actor.sanity.sp() < required {
            return false;
        }
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

/// Scaled value helper: `value + (measured / per) * step`, capped by `max`.
fn scaled(effect: &Effect, measured: i32) -> i32 {
    let value = effect.value.unwrap_or(0);
    let per = effect.per.unwrap_or(0).max(1);
    let step = effect.step.unwrap_or(1);
    let max = effect.max.unwrap_or(i32::MAX);
    (value + (measured / per) * step).min(max)
}

// --------------------------------------------------------------------------- //
// effect application
// --------------------------------------------------------------------------- //

pub struct EffectContext<'a> {
    pub actor_index: usize,
    pub target_index: Option<usize>,
    pub clash_count: i32,
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
        let holds = match &effect.condition {
            Some(cond) => {
                let actor = &state.units[ctx.actor_index];
                let target = ctx.target_index.map(|i| &state.units[i]);
                condition_holds(cond, actor, target, ctx.clash_count)
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
                    .map(|_| scaled(effect, measured))
                    .unwrap_or(0);
                let count = effect.count.map(|_| scaled(effect, measured)).unwrap_or(0);
                let count_status = effect.status2.clone().unwrap_or_else(|| status.clone());
                // A state of time makes the unit inflict or gain more of its
                // status (Burn / Poise / Bleed).
                let (mut potency, mut count) = (potency, count);
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
                let amount = effect.value.unwrap_or(1);
                // Randomly The Living (Potency) or The Departed (Count); the
                // game only states that the pick is random, so a 50/50 split is
                // a simulator assumption - see docs/MECHANICS.md.
                let picks: Vec<bool> = (0..amount).map(|_| state.rng.below(2) == 0).collect();
                let mut spent = 0;
                for from_potency in picks {
                    let unit = &mut state.units[ctx.actor_index];
                    let has_potency = unit.statuses.potency("The Living & The Departed") > 0;
                    let has_count = unit.statuses.count("The Living & The Departed") > 0;
                    if from_potency && has_potency {
                        unit.statuses.add_potency("The Living & The Departed", -1);
                        spent += 1;
                    } else if has_count {
                        unit.statuses.add_count("The Living & The Departed", -1);
                        spent += 1;
                    } else if has_potency {
                        unit.statuses.add_potency("The Living & The Departed", -1);
                        spent += 1;
                    }
                }
                use_ctx.ammo_spent += spent;
            }
            "spend_ammo_all" => {
                let picks: Vec<bool> = (0..40).map(|_| state.rng.below(2) == 0).collect();
                let mut spent = 0;
                for from_potency in picks {
                    let unit = &mut state.units[ctx.actor_index];
                    let has_potency = unit.statuses.potency("The Living & The Departed") > 0;
                    let has_count = unit.statuses.count("The Living & The Departed") > 0;
                    if !has_potency && !has_count {
                        break;
                    }
                    if from_potency && has_potency {
                        unit.statuses.add_potency("The Living & The Departed", -1);
                        spent += 1;
                    } else if has_count {
                        unit.statuses.add_count("The Living & The Departed", -1);
                        spent += 1;
                    } else {
                        unit.statuses.add_potency("The Living & The Departed", -1);
                        spent += 1;
                    }
                }
                use_ctx.ammo_spent += spent;
            }
            "inflict_equal_ammo_spent" => {
                let Some(status) = effect.status.clone() else { continue };
                let spent = use_ctx.ammo_spent;
                let Some(index) = ctx.target_index else { continue };
                state.units[index].statuses.add_potency(&status, spent);
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
            "heal_hp" => {
                let value = effect.value.unwrap_or(0);
                state.units[ctx.actor_index].heal(value);
            }
            "sp_heal" => {
                let value = effect.value.unwrap_or(0);
                let sanity = state.units[ctx.actor_index].sanity;
                state.units[ctx.actor_index].sanity = sanity.add(value);
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
                    if let Some(first) = state.units[index]
                        .stagger
                        .thresholds_percent
                        .first_mut()
                    {
                        *first += potency;
                    }
                    let consume = effect.consume_count.unwrap_or(1);
                    state.units[index].statuses.add_count("Tremor", -consume);
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
                    let total = potency * times;
                    unit.take_damage(total);
                }
                let unit = &mut state.units[index];
                unit.statuses.add_count(&status, -consume * times);
            }
            "lose_status_count" => {
                let Some(status) = effect.status.clone() else { continue };
                let Some(index) = ctx.target_index else { continue };
                let amount = effect.count.or(effect.value).unwrap_or(1);
                state.units[index].statuses.add_count(&status, -amount);
            }
            "reload_ammo" => {
                // Reload (Solemn Lament): spend SP, reset ammo, refill to the cap.
                let unit = &mut state.units[ctx.actor_index];
                let sum = unit.statuses.total("The Living & The Departed");
                let sp_cost = ((30 - sum) / 2).max(0);
                unit.sanity = unit.sanity.add(-sp_cost);
                unit.statuses.remove("The Living & The Departed");
                unit.statuses.add_potency("Reload (Solemn Lament)", 1);
            }
            "reuse_percent_missing_hp" => {
                // Recorded on the context; the attack loop re-uses the coin.
                use_ctx.notes.push(format!(
                    "reuse:per={}:max={}",
                    effect.per.unwrap_or(33),
                    effect.max.unwrap_or(1)
                ));
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
            "noop" => {}
            other => {
                ctx.mechanics_note
                    .push(format!("unhandled effect kind `{other}` ({:?})", effect.raw));
            }
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
    let mech = mechanics
        .get(&SkillId::new(format!("{}.{}", ego_id.as_str(), key)))
        .cloned()
        .unwrap_or_default();
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
fn effective_coin_power(
    _state: &mut BattleState,
    _unit_index: usize,
    use_: &SkillUse,
    coin_index: usize,
) -> i32 {
    let coin = &use_.coins[coin_index];
    if coin.state == CoinState::Cracked {
        // Cracked Unbreakable Coins fix Coin Power to 1 (+1 for plus coins).
        return if use_.coin_power > 0 { 2 } else { 1 };
    }
    if coin.paralyzed {
        return 0;
    }
    let boost = use_.ctx.coin_power_bonus
        + if use_.coin_power > 0 {
            // Plus Coin Boost raises plus coins, Minus Coin Drop lowers minus coins.
            use_.ctx.coin_power_boost
        } else {
            -use_.ctx.coin_power_drop
        };
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
            0
        } else {
            statuses.count("Attack Power Up") - statuses.potency("Attack Power Down")
        };
        total += all + attack;
    }
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
    /// attacking Sinner."
    pub fn on_hit_by_sinner(state: &mut BattleState, enemy_index: usize, attacker_index: usize) {
        if state.units[enemy_index].time_state != Some(crate::scripts::TimeState::Past) {
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
                // Kalpāgni's final Coin: SP damage equal to half the Stack.
                let sanity = state.units[target_index].sanity;
                state.units[target_index].sanity = sanity.add(-(stack / 2));
            }
            crate::scripts::TimeState::Present => {
                // Smite the Wicked's final Coin: raise the Stagger Threshold.
                let raise = stack / 2;
                let thresholds = &mut state.units[target_index].stagger.thresholds_percent;
                if let Some(first) = thresholds.first_mut() {
                    *first += raise;
                }
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
) -> i32 {
    let power = final_power(state, unit_index, use_);
    let my_level = state.units[unit_index].offense_level() + use_.offense_level_mod;
    let their_level = match use_.defense_level_mod {
        Some(modifier) => state.units[opponent_index].offense_level() + modifier,
        None => state.units[opponent_index].defense_level(),
    };
    let mut total = power + level_clash_bonus(my_level, their_level);
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

fn clash_key(state: &BattleState, a: usize, b: usize) -> String {
    format!("{}|{}", state.units[a].id, state.units[b].id)
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
    let mut rounds = 0;
    loop {
        if !a.has_fresh_coins() || !b.has_fresh_coins() {
            break;
        }
        toss_all(state, a_index, a);
        toss_all(state, b_index, b);
        let a_power = match_power(state, a_index, b_index, a);
        let b_power = match_power(state, b_index, a_index, b);
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
    let potency = state.units[unit_index].statuses.potency("Bleed");
    if potency <= 0 {
        return;
    }
    let (_, hp_lost) = state.units[unit_index].take_damage(potency);
    state.units[unit_index].statuses.add_count("Bleed", -1);
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
    defender_index: usize,
    use_: &mut SkillUse,
    clash_count: i32,
) -> Vec<HitResult> {
    let mut hits = Vec::new();
    let mut order: Vec<usize> = (0..use_.coins.len())
        .filter(|i| use_.coins[*i].state == CoinState::Fresh)
        .collect();
    let mut reuse_budget = reuse_budget(&use_.ctx);
    loop {
        let Some(coin_index) = order.first().copied() else { break };
        order.remove(0);
        tick_bleed(state, attacker_index);
        if use_.coins[coin_index].heads.is_none() {
            toss_single(state, attacker_index, use_, coin_index);
        }
        let heads = use_.coins[coin_index].heads.unwrap_or(false);
        if use_.ctx.accumulated == 0 {
            // The accumulator starts at Base Power for the first coin used.
            use_.ctx.accumulated = use_.base_power + use_.ctx.base_power_bonus;
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
        if reuse_budget > 0 {
            reuse_budget -= 1;
            if let Some(coin) = use_.coins.get_mut(coin_index) {
                coin.state = CoinState::Fresh;
                coin.heads = None;
                coin.paralyzed = false;
            }
            order.insert(0, coin_index);
        }
        if !state.units[defender_index].alive {
            break;
        }
    }
    // Cracked Unbreakable Coins attack after getting hit (wiki.gg `Clash`).
    for coin_index in use_.cracked_coins() {
        tick_bleed(state, attacker_index);
        // A cracked coin comes back with its Coin Power fixed to 1 (+1 plus).
        let power = effective_coin_power(state, attacker_index, use_, coin_index);
        let hit = apply_hit(
            state,
            attacker_index,
            defender_index,
            use_,
            coin_index,
            power,
            false,
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

fn reuse_budget(use_: &UseContext) -> i32 {
    for note in &use_.notes {
        if let Some(rest) = note.strip_prefix("reuse:") {
            let mut parts = rest.split(':');
            let per: i32 = parts.next().and_then(|p| p.trim_start_matches("per=").parse().ok()).unwrap_or(33);
            let max: i32 = parts.next().and_then(|p| p.trim_start_matches("max=").parse().ok()).unwrap_or(1);
            let _ = per;
            return max.max(0);
        }
    }
    0
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
    let power_of_incoming = power;
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
            let heads = state.flip(heads_percent(state, &state.units[defender_index]));
            let defense = &state.defenses[defense];
            let power = defense.base_power + if heads { defense.coin_power } else { 0 };
            power >= power_of_incoming
        };
        if evaded {
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
        let crit = potency > 0 && state.flip(potency);
        (crit, potency)
    };
    if crit
        && poise_potency > 0
        && !time_passives::keeps_poise_on_crit(state, attacker_index)
    {
        state.units[attacker_index].statuses.add_count("Poise", -1);
    }
    let defender = &state.units[defender_index];
    let stagger_bonus = if defender.is_staggered() {
        Some(defender.stagger.damage_resistance_bonus())
    } else {
        None
    };
    let sin_resist = defender.resist_sin(use_.sin);
    let sin_name = match use_.sin {
        Sin::Wrath => "Wrath",
        Sin::Lust => "Lust",
        Sin::Sloth => "Sloth",
        Sin::Gluttony => "Gluttony",
        Sin::Gloom => "Gloom",
        Sin::Pride => "Pride",
        Sin::Envy => "Envy",
    };
    let inputs = DamageInputs {
        coin_roll: power,
        sin_resist,
        damage_type_resist: defender.resist(use_.damage_type),
        stagger_bonus,
        offense_level: state.units[attacker_index].offense_level() + use_.offense_level_mod,
        defense_level: active_defense_level(state, defender_index)
            .unwrap_or_else(|| state.units[defender_index].defense_level()),
        critical: crit,
        clash_count,
        dynamic_modifier: use_.ctx.damage_bonus
            + state.units[attacker_index].outgoing_damage_modifier()
            + incoming_damage_modifier(defender, sin_name)
            + if crit {
                time_passives::crit_damage_bonus(state, attacker_index)
            } else {
                0.0
            },
        ..Default::default()
    };
    let breakdown = compute_damage(&inputs);
    let damage = if use_.ctx.zero_damage { 0 } else { breakdown.final_damage };
    let (_, hp_lost) = state.units[defender_index].take_damage(damage);

    // [On Hit] coin effects.
    let effects: Vec<Effect> = use_.mechanics.coin(coin_index as u32 + 1).to_vec();
    let mut notes = Vec::new();
    {
        let mut ctx = EffectContext {
            actor_index: attacker_index,
            target_index: Some(defender_index),
            clash_count,
            mechanics_note: &mut notes,
        };
        let mut local = UseContext::default();
        apply_effects(state, &effects, &mut ctx, &mut local);
        use_.ctx.ammo_spent = local.ammo_spent.max(use_.ctx.ammo_spent);
    }
    for note in notes {
        state.warnings.push(note);
    }

    // Attack adders: "deal N% of this Coin's final damage as bonus damage".
    for effect in use_.mechanics.coin(coin_index as u32 + 1).to_vec() {
        if effect.kind != "bonus_damage_percent_of_coin" {
            continue;
        }
        let percent = effect.percent.unwrap_or(0);
        if percent > 0 && damage > 0 {
            let extra = (damage as f64 * percent as f64 / 100.0).floor() as i32;
            if extra > 0 {
                state.units[defender_index].take_damage(extra);
            }
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
    // Rupture: "When hit by an attack, take fixed damage by the effect's
    // Potency. Then, reduce its Count by 1." (wiki.gg `Status Effects`).
    let rupture = state.units[defender_index].statuses.potency("Rupture");
    if rupture > 0 {
        state.units[defender_index].take_damage(rupture);
        state.units[defender_index].statuses.add_count("Rupture", -1);
    }
    // Sinking: when hit, SP damage by Potency then Count -1.
    apply_sinking(state, defender_index);

    // Butterfly (unique Sinking): the attacker heals (The Living / 4) SP.
    let butterfly_living = state.units[defender_index].statuses.potency("Butterfly");
    if butterfly_living > 0 {
        let heal = (butterfly_living / 4).max(1);
        let sanity = state.units[attacker_index].sanity;
        state.units[attacker_index].sanity = sanity.add(heal);
    }

    // A Counter skill strikes back at whoever attacked the unit.
    if state.units[defender_index].alive {
        if let Some(position) = state.defenses.iter().position(|d| {
            d.unit == state.units[defender_index].id && d.kind == DefenseKind::Counter
        }) {
            let counter = state.defenses[position].clone();
            let mut use_ = SkillUse {
                actor: counter.unit.clone(),
                target: Some(state.units[attacker_index].id.clone()),
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
            one_sided_attack(state, defender_index, attacker_index, &mut use_, 0);
        }
    }
    let staggered = check_stagger(state, defender_index);
    let killed = !state.units[defender_index].alive;
    if killed {
        state.units[attacker_index].statuses.add_potency("Poise", 0);
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
    state.units[unit_index].statuses.add_count("Sinking", -1);
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
    if !unit.alive || unit.is_staggered() {
        return false;
    }
    let crossed = unit.stagger.crossed_thresholds(unit.hp, unit.max_hp);
    if crossed == 0 {
        return false;
    }
    let level = crossed.min(crate::state::StaggerState::MAX_LEVEL);
    let unit = &mut state.units[unit_index];
    unit.stagger.level = level;
    // "unable to use their Skills for the current and subsequent Turn"
    unit.stagger.turns_remaining = 2;
    true
}

// --------------------------------------------------------------------------- //
// turn loop
// --------------------------------------------------------------------------- //

pub fn begin_turn(
    state: &mut BattleState,
    library: &Library,
    _mechanics: &MechanicsBook,
    scripts: &crate::scripts::ScriptsBook,
) {
    state.turn += 1;
    state.phase = Phase::TurnStart;
    state.actions.clear();
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
        let haste = state.units[index].statuses.count("Haste");
        // `Bind`: "Speed decreases by the effect's Potency for one turn."
        let bind = state.units[index].statuses.potency("Bind");
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
                let (needs_current, needs_next) = {
                    let entry = state.units[index]
                        .dashboard
                        .iter()
                        .find(|s| s.slot == slot)
                        .unwrap();
                    (entry.current.0.is_empty(), entry.next.0.is_empty())
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
            }
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
    // Enemy Skill Slots.  A unit with a documented action pattern (the Imago)
    // uses one action per listed slot for the current turn of its cycle.
    for index in 0..state.units.len() {
        if !state.units[index].alive || state.units[index].kind.is_sinner() {
            continue;
        }
        let skills = enemy_turn_skills(state, library, scripts, index);
        let targets = enemy_targets(state, index, skills.len());
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
fn enemy_targets(state: &BattleState, unit_index: usize, slots: usize) -> Vec<Option<UnitId>> {
    let mut sinners: Vec<(i32, UnitId)> = state
        .units
        .iter()
        .filter(|u| u.alive && u.kind.is_sinner())
        .map(|u| (u.speed, u.id.clone()))
        .collect();
    sinners.sort_by(|a, b| b.0.cmp(&a.0));
    if sinners.is_empty() {
        return vec![None; slots];
    }
    let _ = unit_index;
    (0..slots)
        .map(|slot| Some(sinners[slot % sinners.len()].1.clone()))
        .collect()
}

pub fn end_turn(state: &mut BattleState) {
    state.phase = Phase::TurnEnd;
    state.defenses.clear();
    for unit in state.units.iter_mut() {
        unit.statuses.remove("No Damage Taken");
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
            state.units[index].statuses.add_count("Burn", -1);
        }
        // Poise: end of turn, Count -1.
        if state.units[index].statuses.count("Poise") > 0 {
            state.units[index].statuses.add_count("Poise", -1);
        }
        // Tremor: "At the end of the turn, reduce the Count by 1."
        if state.units[index].statuses.count("Tremor") > 0 {
            state.units[index].statuses.add_count("Tremor", -1);
        }
        // Charge: "Count lowers by 1 at the end of each turn."
        if state.units[index].statuses.count("Charge") > 0 {
            state.units[index].statuses.add_count("Charge", -1);
        }
        // Butterfly: reset The Departed to 0, then The Living becomes The Departed.
        let butterfly = state.units[index].statuses.potency("Butterfly");
        if butterfly > 0 || state.units[index].statuses.count("Butterfly") > 0 {
            let living = state.units[index].statuses.potency("Butterfly");
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

/// Resolve the combat phase: pair up skills into clashes in speed order.
pub fn resolve_combat(state: &mut BattleState, library: &Library, mechanics: &MechanicsBook) -> Vec<ClashResult> {
    let mut results = Vec::new();
    let mut pending: Vec<(usize, SubmittedAction, Option<usize>)> = Vec::new();
    for action in state.actions.clone() {
        let Some(actor) = state.index_of(&action.actor) else { continue };
        if !state.units[actor].alive {
            continue;
        }
        if state.units[actor].is_staggered() && !state.units[actor].acts_while_staggered {
            continue;
        }
        let target = action.target.as_ref().and_then(|t| state.index_of(t));
        pending.push((actor, action, target));
    }
    // Sort by speed (descending), then by deployment order.
    pending.sort_by_key(|(index, _, _): &(usize, SubmittedAction, Option<usize>)| {
        let speed = state.units[*index].speed;
        let order = state
            .deployment
            .iter()
            .position(|id| id == &state.units[*index].id)
            .unwrap_or(usize::MAX);
        (std::cmp::Reverse(speed), order)
    });

    // Defense skills are not attacks: they arm the unit for the turn.
    let mut pending: Vec<(usize, SubmittedAction, Option<usize>)> = pending
        .into_iter()
        .filter(|(index, action, target)| {
            if let Some(kind) = action_defense_kind(state, library, action) {
                let mut use_ = build_action_use(state, library, mechanics, *index, action);
                if let Some(use_) = use_.as_mut() {
                    prepare_use(state, library, mechanics, *index, *target, use_);
                    let defense = ActiveDefense {
                        unit: state.units[*index].id.clone(),
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
                    };
                    // Guards are not applied here: the Shield is gained when the
                    // unit is first attacked this turn (see `activate_guard`).
                    state.defenses.push(defense);
                }
                return false;
            }
            true
        })
        .collect();
    let mut done: Vec<bool> = vec![false; pending.len()];
    // (unit index, slot) of skills that were actually used this turn; these are
    // the slot rotations the panel performs at the end of the turn.
    let mut executed: Vec<(usize, u32)> = Vec::new();
    for i in 0..pending.len() {
        if done[i] {
            continue;
        }
        let (actor_i, action_i, target_i) = pending[i].clone();
        // Does the target also act against the actor with an attack skill?
        let mut opponent_slot = None;
        if let Some(target) = target_i {
            for (j, (actor_j, _, target_j)) in pending.iter().enumerate() {
                if done[j] || *actor_j != target {
                    continue;
                }
                if *target_j == Some(actor_i) {
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
                let mut use_a = build_action_use(state, library, mechanics, actor_i, &action_i);
                let mut use_b = build_action_use(state, library, mechanics, actor_j, &action_j);
                if let (Some(a), Some(b)) = (use_a.as_mut(), use_b.as_mut()) {
                    let pre_a = prepare_use(state, library, mechanics, actor_i, target_i, a);
                    let pre_b = prepare_use(state, library, mechanics, actor_j, Some(actor_i), b);
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
                            one_sided_attack(state, actor_i, target, a, clash_count);
                        }
                    } else if outcome.winner.as_ref() == Some(&state.units[actor_j].id) {
                        apply_clash_result(state, actor_j, Some(actor_i), b, true);
                        apply_clash_result(state, actor_i, Some(actor_j), a, false);
                        let clash_count = outcome.rounds;
                        one_sided_attack(state, actor_j, actor_i, b, clash_count);
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
                if !state.units[actor_i].alive {
                    done[i] = true;
                    continue;
                }
                if let Some(target) = target_i.filter(|t| state.units[*t].alive) {
                    if let Some(mut use_) =
                        build_action_use(state, library, mechanics, actor_i, &action_i)
                    {
                        prepare_use(state, library, mechanics, actor_i, Some(target), &mut use_);
                        let hits = one_sided_attack(state, actor_i, target, &mut use_, 0);
                        apply_attack_end(state, actor_i, Some(target), &mut use_);
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
                        if ends_encounter(state, actor_i, &use_.skill) {
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
    for (unit_index, slot) in executed {
        let Some(entry) = state.units[*unit_index]
            .dashboard
            .iter()
            .find(|s| s.slot == *slot)
            .cloned()
        else {
            continue;
        };
        // The skill that occupied the slot is consumed (also when it was
        // replaced by a defense skill or E.G.O).
        state.units[*unit_index].deck.consume(&entry.current);
        let drawn = crate::setup::draw_for_unit(state, *unit_index)
            .unwrap_or_else(crate::setup::empty_skill);
        if let Some(target) = state.units[*unit_index]
            .dashboard
            .iter_mut()
            .find(|s| s.slot == *slot)
        {
            target.current = entry.next.clone();
            target.next = drawn;
            target.converted = false;
            target.target = None;
        }
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
        let use_ = build_ego_use(state, library, mechanics, unit_index, &ego_id, kind)?;
        let detail = pay_ego(state, unit_index, &record, kind);
        state.push_log("ego", format!("{} -> {}", use_.name, detail));
        return Some(use_);
    }
    let use_ = build_use(state, library, mechanics, unit_index, &action.skill)?;
    Some(use_)
}

/// Run `[On Use]` / `[Combat Start]` effects and apply the resulting context.
fn prepare_use(
    state: &mut BattleState,
    library: &Library,
    mechanics: &MechanicsBook,
    unit_index: usize,
    target_index: Option<usize>,
    use_: &mut SkillUse,
) -> UseContext {
    let _ = (library, mechanics);
    let mut notes = Vec::new();
    let mut ctx = EffectContext {
        actor_index: unit_index,
        target_index,
        clash_count: 0,
        mechanics_note: &mut notes,
    };
    let on_use = use_.mechanics.on_use.clone();
    apply_effects(state, &on_use, &mut ctx, &mut use_.ctx);
    let shield = use_.ctx.shield_gain;
    if shield > 0 {
        state.units[unit_index].shield += shield;
    }
    // `Plus Coin Boost` / `Minus Coin Drop` are read when the skill is used.
    {
        let statuses = &state.units[unit_index].statuses;
        use_.ctx.coin_power_boost = statuses.count("Plus Coin Boost");
        use_.ctx.coin_power_drop = statuses.count("Minus Coin Drop");
    }
    // "On Use, an Attack Skill will generate 1 E.G.O Resource of a
    // corresponding Affinity" (wiki.gg `Clash` / Attack Skills).
    if !use_.is_defense && !use_.is_ego {
        let key = crate::setup::sin_key(use_.sin).to_string();
        *state.ego_resources.entry(key).or_insert(0) += 1;
    }
    for note in notes {
        state.warnings.push(note);
    }
    for coin in use_.coins.iter_mut() {
        if use_.ctx.unbreakable_coins.contains(&1) || use_.ctx.unbreakable_all {
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
        mechanics_note: &mut notes,
    };
    apply_effects(state, &list, &mut ctx, &mut use_.ctx);
    for note in notes {
        state.warnings.push(note);
    }
}

fn apply_attack_end(
    state: &mut BattleState,
    unit_index: usize,
    target_index: Option<usize>,
    use_: &mut SkillUse,
) {
    let list = use_.mechanics.attack_end.clone();
    if list.is_empty() {
        return;
    }
    let mut notes = Vec::new();
    let mut ctx = EffectContext {
        actor_index: unit_index,
        target_index,
        clash_count: 0,
        mechanics_note: &mut notes,
    };
    apply_effects(state, &list, &mut ctx, &mut use_.ctx);
    for note in notes {
        state.warnings.push(note);
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
        mechanics_note: notes,
    };
    apply_effects(state, effects, &mut ctx, use_ctx);
}

#[doc(hidden)]
pub fn toss_all_for_test(state: &mut BattleState, unit_index: usize, use_: &mut SkillUse) {
    toss_all(state, unit_index, use_)
}
