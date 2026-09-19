//! Damage calculation, implemented directly from wiki.gg `Damage` / `Clash`.
//!
//! ```text
//! Coin Roll x (1 + Static) x (1 + Dynamic) = Final Damage
//! Static  = SinRes + DamageRes + Offense/Defense Level Advantage + Crit
//!           + ClashCount x 0.03 (+ Observation Level, always 0 in game)
//! Dynamic = Status Effects + Passives + Skill Damage Bonuses + E.G.O Gifts
//! ```
//! Final damage is floored, cannot go below 1, and cannot go below
//! `0.05 x Coin Roll`.

use crate::ids::{DamageType, Sin};

/// Resistance -> static modifier (wiki.gg `Damage`, "Sin Resistances").
pub fn resistance_modifier(value: f64) -> f64 {
    if value < 0.0 {
        -0.5
    } else if value < 1.0 {
        (value - 1.0) / 2.0
    } else {
        value - 1.0
    }
}

/// `M = (Off - Def) / (|Off - Def| + 25)`
pub fn offense_defense_advantage(offense: i32, defense: i32) -> f64 {
    let diff = (offense - defense) as f64;
    diff / (diff.abs() + 25.0)
}

/// Clash power from level difference: +1 per 3 levels, rounded down.
pub fn level_clash_bonus(offense: i32, defense: i32) -> i32 {
    let diff = offense - defense;
    if diff <= 0 {
        return 0;
    }
    diff / 3
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DamageInputs {
    pub coin_roll: i32,
    pub sin_resist: f64,
    pub damage_type_resist: f64,
    /// Set when the defender is staggered; the larger of the two applies.
    pub stagger_bonus: Option<f64>,
    pub offense_level: i32,
    pub defense_level: i32,
    pub critical: bool,
    /// Static critical modifiers (flat) and multipliers, default 0 / 0.
    pub static_crit_modifier: f64,
    pub static_crit_multiplier: f64,
    pub clash_count: i32,
    pub dynamic_modifier: f64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DamageBreakdown {
    pub static_modifier: f64,
    pub dynamic_modifier: f64,
    pub raw: f64,
    pub final_damage: i32,
}

pub fn compute_damage(inputs: &DamageInputs) -> DamageBreakdown {
    let sin_mod = resistance_modifier(inputs.sin_resist);
    let type_mod = match inputs.stagger_bonus {
        Some(bonus) => resistance_modifier(inputs.damage_type_resist).max(bonus),
        None => resistance_modifier(inputs.damage_type_resist),
    };
    let level_mod = offense_defense_advantage(inputs.offense_level, inputs.defense_level);
    let crit_mod = if inputs.critical {
        (0.2 + inputs.static_crit_modifier) * (1.0 + inputs.static_crit_multiplier)
    } else {
        0.0
    };
    let static_modifier =
        sin_mod + type_mod + level_mod + crit_mod + inputs.clash_count as f64 * 0.03;
    let dynamic_modifier = inputs.dynamic_modifier;
    let raw = inputs.coin_roll as f64 * (1.0 + static_modifier) * (1.0 + dynamic_modifier);
    let floor = raw.floor().max(1.0);
    let min_by_coin_roll = inputs.coin_roll as f64 * 0.05;
    DamageBreakdown {
        static_modifier,
        dynamic_modifier,
        raw,
        final_damage: floor.max(min_by_coin_roll).max(1.0) as i32,
    }
}

/// Convenience wrapper for a plain physical/sin resistance pair.
pub fn attack_modifier(
    sin: Sin,
    damage_type: DamageType,
    sin_resist: f64,
    type_resist: f64,
) -> f64 {
    let _ = (sin, damage_type);
    resistance_modifier(sin_resist) + resistance_modifier(type_resist)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resistance_formula_matches_source() {
        // Ineff. [x0.5] -> -0.25
        assert!((resistance_modifier(0.5) + 0.25).abs() < 1e-9);
        // Weak [x1.5] -> +0.5
        assert!((resistance_modifier(1.5) - 0.5).abs() < 1e-9);
        // Immune [x0] -> -0.5
        assert!((resistance_modifier(0.0) + 0.5).abs() < 1e-9);
        assert!((resistance_modifier(2.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn level_advantage_matches_table() {
        assert!((offense_defense_advantage(1, 0) - 0.038).abs() < 1e-2);
        assert!((offense_defense_advantage(25, 0) - 0.5).abs() < 1e-9);
        assert!((offense_defense_advantage(0, 25) + 0.5).abs() < 1e-9);
    }

    #[test]
    fn clash_level_bonus_is_one_per_three_levels() {
        assert_eq!(level_clash_bonus(4, 0), 1);
        assert_eq!(level_clash_bonus(6, 0), 2);
        assert_eq!(level_clash_bonus(2, 0), 0);
        assert_eq!(level_clash_bonus(0, 5), 0);
    }

    #[test]
    fn damage_has_floor_of_one() {
        // extreme negatives drive the product to 0, the floor lifts it to 1
        let inputs = DamageInputs {
            coin_roll: 5,
            sin_resist: 0.0,
            damage_type_resist: 0.0,
            dynamic_modifier: -1.0,
            ..Default::default()
        };
        assert_eq!(compute_damage(&inputs).final_damage, 1);
    }

    #[test]
    fn resistances_are_additive_and_then_multiplied() {
        // 1.5x slash and 0.75x wrath -> +37% as documented on the wiki
        let inputs = DamageInputs {
            coin_roll: 100,
            sin_resist: 0.75,
            damage_type_resist: 1.5,
            ..Default::default()
        };
        let breakdown = compute_damage(&inputs);
        assert!((breakdown.static_modifier - 0.375).abs() < 1e-9);
        assert_eq!(breakdown.final_damage, 137);
    }

    #[test]
    fn damage_never_below_five_percent_of_coin_roll() {
        // heavy negative modifiers cannot push damage under 5% of the coin roll
        let inputs = DamageInputs {
            coin_roll: 40,
            sin_resist: 1.0,
            damage_type_resist: 1.0,
            dynamic_modifier: -1.0,
            ..Default::default()
        };
        assert_eq!(compute_damage(&inputs).final_damage, 2);
    }
}
