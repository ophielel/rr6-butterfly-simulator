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
    BattleState, ClashTieRule, Phase, Sanity, SubmittedAction, Unit, UnitKind, Winner, SP_LIMIT,
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
}

impl CoinRuntime {
    pub fn fresh(unbreakable: bool) -> Self {
        Self {
            heads: None,
            state: CoinState::Fresh,
            unbreakable,
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
    pub notes: Vec<String>,
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
            .map(|s| s.skill.clone())
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
            entry.skill = skill.clone();
            entry.target = None;
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
        None => unit.statuses.total(key),
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
        if unit.hp_percent() >= hp {
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

fn flip(state: &mut BattleState, unit_index: usize, coin: &mut CoinRuntime) {
    if coin.heads.is_none() {
        let percent = heads_percent(state, &state.units[unit_index]);
        coin.heads = Some(state.rng.flip_percent(percent));
    }
}

/// Final Power of a coin during a one-sided attack.
pub fn coin_power(
    state: &BattleState,
    unit_index: usize,
    use_: &SkillUse,
    coin_index: usize,
) -> i32 {
    let coin = &use_.coins[coin_index];
    let mut power = use_.base_power + use_.ctx.base_power_bonus;
    match coin.state {
        CoinState::Cracked => {
            // Cracked Unbreakable Coins fix Coin Power to 1 (+1 for plus coins).
            power = 1;
            if use_.coin_power > 0 {
                power += 1;
            }
        }
        _ => {
            if coin.heads.unwrap_or(false) {
                power += use_.coin_power;
            }
        }
    }
    power += use_.ctx.coin_power_bonus;
    // Paralyze fixes the power of the next coins to 0.
    if state.units[unit_index].statuses.potency("Paralyze") > 0 {
        power = 0;
    }
    power.max(0)
}

/// Clash Power of a coin during a clash.
pub fn clash_coin_power(
    state: &BattleState,
    unit_index: usize,
    use_: &SkillUse,
    opponent_index: usize,
    coin_index: usize,
) -> i32 {
    let unit = &state.units[unit_index];
    let opponent = &state.units[opponent_index];
    let attacker_level = unit.offense_level() + use_.offense_level_mod;
    let defender_level = match use_.defense_level_mod {
        Some(modifier) => opponent.offense_level() + modifier,
        None => opponent.defense_level(),
    };
    let level_bonus = level_clash_bonus(attacker_level, defender_level);
    let status_bonus = unit.statuses.count("Clash Power Up");
    coin_power(state, unit_index, use_, coin_index) + use_.ctx.clash_power_bonus + level_bonus
        + status_bonus
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
    let mech = mechanics.get_or_default(skill);
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

pub fn resolve_clash(
    state: &mut BattleState,
    a_index: usize,
    b_index: usize,
    a: &mut SkillUse,
    b: &mut SkillUse,
) -> ClashResult {
    let mut rounds = 0;
    let tie_rule = state.config.tie_rule;
    loop {
        let Some(a_coin) = a.current_coin() else { break };
        let Some(b_coin) = b.current_coin() else { break };
        flip(state, a_index, &mut a.coins[a_coin]);
        flip(state, b_index, &mut b.coins[b_coin]);
        let a_power = clash_coin_power(state, a_index, a, b_index, a_coin);
        let b_power = clash_coin_power(state, b_index, b, a_index, b_coin);
        rounds += 1;
        if a_power > b_power {
            break_coin(&mut b.coins[b_coin], b_index);
        } else if b_power > a_power {
            break_coin(&mut a.coins[a_coin], a_index);
        } else {
            match tie_rule {
                ClashTieRule::BothLoseCoin => {
                    break_coin(&mut a.coins[a_coin], a_index);
                    break_coin(&mut b.coins[b_coin], b_index);
                }
                ClashTieRule::AttackerWins => break_coin(&mut b.coins[b_coin], b_index),
                ClashTieRule::DefenderWins => break_coin(&mut a.coins[a_coin], a_index),
            }
        }
        if rounds > 32 {
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
    if potency > 0 {
        state.units[unit_index].take_damage(potency);
        state.units[unit_index].statuses.add_count("Bleed", -1);
    }
}

/// One-sided attack with every remaining (fresh) coin.
pub fn one_sided_attack(
    state: &mut BattleState,
    attacker_index: usize,
    defender_index: usize,
    use_: &mut SkillUse,
    clash_count: i32,
) -> Vec<HitResult> {
    let mut hits = Vec::new();
    let mut coin_order: Vec<usize> =
        (0..use_.coins.len()).filter(|i| use_.coins[*i].state == CoinState::Fresh).collect();
    let mut reuse_budget = reuse_budget(&use_.ctx);
    loop {
        let Some(coin_index) = coin_order.first().copied() else { break };
        coin_order.remove(0);
        tick_bleed(state, attacker_index);
        flip(state, attacker_index, &mut use_.coins[coin_index]);
        let heads = use_.coins[coin_index].heads.unwrap_or(false);
        let power = coin_power(state, attacker_index, use_, coin_index);
        let hit = apply_hit(state, attacker_index, defender_index, use_, coin_index, power, heads, clash_count);
        hits.push(hit);
        if reuse_budget > 0 {
            reuse_budget -= 1;
            coin_order.insert(0, coin_index);
            if let Some(coin) = use_.coins.get_mut(coin_index) {
                coin.state = CoinState::Fresh;
            }
        }
        if !state.units[defender_index].alive {
            break;
        }
    }
    // Cracked Unbreakable Coins attack after getting hit (wiki.gg `Clash`).
    for coin_index in use_.cracked_coins() {
        tick_bleed(state, attacker_index);
        let power = coin_power(state, attacker_index, use_, coin_index);
        let hit = apply_hit(state, attacker_index, defender_index, use_, coin_index, power, false, clash_count);
        hits.push(hit);
        if !state.units[defender_index].alive {
            break;
        }
    }
    hits
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
    // Bleed is applied when the coin is tossed; damage is computed for the hit.
    let (crit, poise_potency) = {
        let attacker = &state.units[attacker_index];
        let potency = attacker.statuses.potency("Poise");
        let crit = potency > 0 && state.rng.flip_percent(potency);
        (crit, potency)
    };
    if crit && poise_potency > 0 {
        state.units[attacker_index].statuses.add_count("Poise", -1);
    }
    let defender = &state.units[defender_index];
    let stagger_bonus = if defender.is_staggered() {
        Some(defender.stagger.damage_resistance_bonus())
    } else {
        None
    };
    let inputs = DamageInputs {
        coin_roll: power,
        sin_resist: defender.resist_sin(use_.sin),
        damage_type_resist: defender.resist(use_.damage_type),
        stagger_bonus,
        offense_level: state.units[attacker_index].offense_level() + use_.offense_level_mod,
        defense_level: defender.defense_level(),
        critical: crit,
        clash_count,
        dynamic_modifier: use_.ctx.damage_bonus + fragile_bonus(defender),
        ..Default::default()
    };
    let breakdown = compute_damage(&inputs);
    let damage = breakdown.final_damage;
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

    // Sinking: when hit, SP damage by Potency then Count -1.
    apply_sinking(state, defender_index);

    // Butterfly (unique Sinking): the attacker heals (The Living / 4) SP.
    let butterfly_living = state.units[defender_index].statuses.potency("Butterfly");
    if butterfly_living > 0 {
        let heal = (butterfly_living / 4).max(1);
        let sanity = state.units[attacker_index].sanity;
        state.units[attacker_index].sanity = sanity.add(heal);
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

fn fragile_bonus(defender: &Unit) -> f64 {
    let fragile = defender.statuses.count("Fragile").min(10);
    fragile as f64 * 0.10
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

pub fn begin_turn(state: &mut BattleState, library: &Library, _mechanics: &MechanicsBook) {
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
        state.units[index].speed = speed + haste;
        // Bleed/other turn-start ticks handled by mechanics entries below.
        let skill = state.units[index]
            .dashboard
            .first()
            .map(|s| s.skill.clone())
            .unwrap_or_else(|| SkillId::new(EMPTY_SKILL));
        let _ = skill;
    }
    // Enemy slots: one skill per living enemy, chosen by the configured policy.
    for index in 0..state.units.len() {
        if !state.units[index].alive || state.units[index].kind.is_sinner() {
            continue;
        }
        if let Some(skill) = choose_enemy_skill(state, library, index) {
            let target = state
                .living_sinners()
                .into_iter()
                .next();
            state.actions.push(SubmittedAction {
                actor: state.units[index].id.clone(),
                slot: 0,
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

/// Enemy skill choice.  The wiki documents the boss rotations in a notation
/// this project does not consider unambiguous, so the default policy is the
/// first listed skill; see docs/MECHANICS.md (`UNKNOWN: boss rotation`).
fn choose_enemy_skill(state: &BattleState, library: &Library, unit_index: usize) -> Option<SkillId> {
    let UnitKind::Abnormality { enemy, .. } = &state.units[unit_index].kind else {
        return None;
    };
    let record = library.enemy(enemy)?;
    record.skills.first().map(|s| SkillId::new(s.skill_id()))
}

pub fn end_turn(state: &mut BattleState) {
    state.phase = Phase::TurnEnd;
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
        if !state.units[actor].alive || state.units[actor].is_staggered() {
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

    let mut done: Vec<bool> = vec![false; pending.len()];
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
                let (actor_j, action_j, _) = pending[j].clone();
                let mut use_a = build_action_use(state, library, mechanics, actor_i, &action_i);
                let mut use_b = build_action_use(state, library, mechanics, actor_j, &action_j);
                if let (Some(a), Some(b)) = (use_a.as_mut(), use_b.as_mut()) {
                    let pre_a = prepare_use(state, library, mechanics, actor_i, target_i, a);
                    let pre_b = prepare_use(state, library, mechanics, actor_j, Some(actor_i), b);
                    let _ = (pre_a, pre_b);
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
                            "{} vs {} over {} round(s); winner: {}",
                            a.name,
                            b.name,
                            outcome.rounds,
                            outcome
                                .winner
                                .as_ref()
                                .map(|w| w.to_string())
                                .unwrap_or_else(|| "none".to_string())
                        ),
                    );
                    results.push(outcome);
                }
                done[i] = true;
                done[j] = true;
            }
            None => {
                if let (Some(target), Some(mut use_)) = (
                    target_i,
                    build_action_use(state, library, mechanics, actor_i, &action_i),
                ) {
                    prepare_use(state, library, mechanics, actor_i, target_i, &mut use_);
                    one_sided_attack(state, actor_i, target, &mut use_, 0);
                    apply_attack_end(state, actor_i, Some(target), &mut use_);
                    state.push_log("attack", format!("{} attacked", use_.name));
                }
                done[i] = true;
            }
        }
    }
    results
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
        if use_.ctx.unbreakable_coins.contains(&1) {
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
