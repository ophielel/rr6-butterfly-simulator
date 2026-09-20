"""Extract machine-readable mechanics from the skill effect text.

The input is the generated library (`data/identities/*.json`,
`data/enemies/*.json`), whose `on_use_text` / `coin_texts` fields already carry
the wiki effect text with status names in `[brackets]`.

Every produced effect keeps the source line in `raw`.  A line that matches no
pattern is recorded in `unmodeled`, which the simulator surfaces (and strict mode
refuses).  Nothing is inferred from a skill's name or from a similar skill.
"""

from __future__ import annotations

import json
import os
import re
import sys
from typing import Dict, List, Optional, Tuple

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
DATA = os.path.join(ROOT, "data")

INPUT_GLOBS = ["identities", "enemies"]

TRIGGER_RE = re.compile(r"^\[(?P<trigger>[^\]]+)\]\s*(?P<rest>.*)$", re.S)

TRIGGERS = {
    "on use": "on_use",
    "clash win": "clash_win",
    "clash lose": "clash_lose",
    "attack end": "attack_end",
    "combat start": "combat_start",
    "start battle": "combat_start",
    "before attack": "before_attack",
    "on hit": "coin",
    "on succeed attack": "coin",
    "onhit": "coin",
    "on hit without cracking": "coin",
    "heads hit": "heads_hit",
    "on evade": "on_evade",
    "hit after clash lose": "coin_clash_lose",
    "on kill": "on_kill",
    "on target kill": "on_kill",
    "reuse - on hit": "coin_reuse",
    "reuse - on crit": "coin_reuse",
    "turn end": "turn_end",
    "turn start": "turn_start",
    "end skill": "attack_end",
    "win duel": "clash_win",
    "lose duel": "clash_lose",
}

# All patterns use positional groups only (no repeated named groups).
ST = r"\[([^\]]+)\]"          # a status name
N = r"([+-]?\d+)"             # an integer, possibly signed

PATTERNS = [
    # Inflict <potency> [X] and +<count> [Y] Count
    (re.compile(rf"^Inflict {N} {ST} and \+{N} {ST} Count$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1)),
                "count": int(m.group(3)), "status2": m.group(4)}),
    (re.compile(rf"^Inflict \+{N} {ST} Count$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "count": int(m.group(1))}),
    (re.compile(rf"^Inflict {N} {ST}$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1))}),
    # "Inflict 1 random [Butterfly]" - the game splits The Living/The Departed
    # randomly; the simulator models the potency half (documented assumption).
    (re.compile(rf"^Inflict {N} random {ST}$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1)),
                "assumption": "random"}),
    (re.compile(rf"^Inflict {ST} equal to {ST} spent$"),
     lambda m: {"kind": "inflict_equal_ammo_spent", "status": m.group(1), "ammo": m.group(2)}),
    (re.compile(rf"^Gain {N} {ST} and \+{N} {ST} Count$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "count": int(m.group(3))}),
    (re.compile(rf"^Gain \+{N} {ST} Count$"),
     lambda m: {"kind": "gain", "status": m.group(2), "count": int(m.group(1))}),
    (re.compile(rf"^Gain {N} {ST}$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1))}),
    (re.compile(rf"^Spend {N} {ST}$"),
     lambda m: {"kind": "spend_ammo", "status": m.group(2), "value": int(m.group(1))}),
    (re.compile(rf"^Coin Power \+{N} for every {N} \({ST} \+ {ST}\) on (?:the main )?target \(max {N}\)$"),
     lambda m: {"kind": "coin_power", "value": 0, "step": int(m.group(1)), "per": int(m.group(2)),
                "max": int(m.group(5)),
                "condition": {"source": "target", "statuses": [m.group(3), m.group(4)],
                              "component": "potency"}}),
    (re.compile(rf"^Clash Power \+{N} for every {N} \({ST} \+ {ST}\) on target \(max {N}\)$"),
     lambda m: {"kind": "clash_power", "value": 0, "step": int(m.group(1)), "per": int(m.group(2)),
                "max": int(m.group(5)),
                "condition": {"source": "target", "statuses": [m.group(3), m.group(4)],
                              "component": "potency"}}),
    (re.compile(rf"^Clash Power \+{N} for every {N} {ST} on (self|target|the main target) \(max {N}[^)]*\)$"),
     lambda m: {"kind": "clash_power", "value": 0, "step": int(m.group(1)), "per": int(m.group(2)),
                "max": int(m.group(5)),
                "condition": {"source": "target" if m.group(4) in ("target", "the main target") else "self",
                              "status": m.group(3), "component": "potency"}}),
    (re.compile(rf"^Final Power \+{N} for every {N} {ST} on (self|target|the main target) \(max {N}[^)]*\)$"),
     lambda m: {"kind": "base_power", "value": 0, "step": int(m.group(1)), "per": int(m.group(2)),
                "max": int(m.group(5)),
                "condition": {"source": "target" if m.group(4) in ("target", "the main target") else "self",
                              "status": m.group(3), "component": "potency"}}),
    (re.compile(rf"^Coin Power \+{N} for every {N} value of the sum of the target's {ST} and (?:both )?{ST} \(max {N}\)$"),
     lambda m: {"kind": "coin_power", "value": int(m.group(1)), "per": int(m.group(2)),
                "max": int(m.group(5)),
                "condition": {"source": "target",
                              "statuses": [m.group(3), m.group(4), m.group(4)],
                              "component": "potency"}}),
    (re.compile(rf"^If the sum of the target's {ST} and (?:both )?{ST} is {N} or higher, Coin Power \+{N}$"),
     lambda m: {"kind": "coin_power", "value": int(m.group(4)),
                "condition": {"source": "target",
                              "statuses": [m.group(1), m.group(2), m.group(2)],
                              "component": "potency", "gte": int(m.group(3))}}),
    (re.compile(rf"^At {N}\+ {ST}, Clash Power \+{N}$"),
     lambda m: {"kind": "clash_power", "value": int(m.group(3)),
                "condition": {"source": "self", "status": m.group(2), "gte": int(m.group(1))}}),
    (re.compile(rf"^Deal \+{N}% damage for every {N} {ST} on (self|target) \(max {N}%\)$"),
     lambda m: {"kind": "damage_percent", "value": 0, "step": int(m.group(1)), "per": int(m.group(2)),
                "max": int(m.group(5)),
                "condition": {"source": m.group(4), "status": m.group(3), "component": "potency"}}),
    (re.compile(r"^At less than (\d+)% HP, convert (the final Coin|the second Coin|all Coins) into \[Unbreakable Coin\]s?$"),
     lambda m: {"kind": "unbreakable_coin", "which": m.group(2), "hp_below_percent": int(m.group(1))}),
    (re.compile(r"^Reuse this Coin once for every (\d+)% missing HP \(max (\d+) times\)$"),
     lambda m: {"kind": "reuse_percent_missing_hp", "per": int(m.group(1)), "max": int(m.group(2))}),
    (re.compile(r"^convert all Coins on this Skill to \[Unbreakable Coin\]s and gain Clash Power \+(\d+)$"),
     lambda m: {"kind": "convert_unbreakable_and_clash", "value": int(m.group(1))}),
    (re.compile(r"^deal (?:Gloom|Wrath|Lust|Sloth|Gluttony|Pride|Envy|Slash|Pierce|Blunt) damage equal to (\d+)% of this Coin's final damage$"),
     lambda m: {"kind": "bonus_damage_percent_of_coin", "percent": int(m.group(1))}),
    # Plain, unconditional power / damage lines (also used inside condition
    # blocks like "If any of the following conditions are met, Coin Power +1").
    (re.compile(r"^Coin Power \+(\d+)$"),
     lambda m: {"kind": "coin_power", "value": int(m.group(1))}),
    (re.compile(r"^Base Power \+(\d+)$"),
     lambda m: {"kind": "base_power", "value": int(m.group(1))}),
    (re.compile(r"^Final Power \+(\d+)$"),
     lambda m: {"kind": "base_power", "value": int(m.group(1))}),
    (re.compile(r"^Clash Power \+(\d+)$"),
     lambda m: {"kind": "clash_power", "value": int(m.group(1))}),
    (re.compile(r"^deal \+(\d+)% damage$"),
     lambda m: {"kind": "damage_percent", "value": int(m.group(1))}),
    # "At 15+ [Deep Tears], consume 5 [Deep Tears] to deal +15% damage"
    (re.compile(rf"^At {N}\+ {ST}, consume {N} {ST} to deal \+{N}% damage$"),
     lambda m: {"kind": "consume_status_for_damage", "status": m.group(2),
                "threshold": int(m.group(1)), "value": int(m.group(3)),
                "percent": int(m.group(5))}),
    # "Gain Shield equal to (SP / 5)% of this unit's max HP"
    (re.compile(r"^Gain Shield equal to \(SP / (\d+)\)% of this unit's max HP$"),
     lambda m: {"kind": "shield_percent_from_sp", "value": int(m.group(1))}),
    # "For every [Protection] on self, gain Shield equal to 5% of max HP (max 15%)"
    (re.compile(rf"^For every {ST} on self, gain Shield equal to {N}% of this unit's max HP \(max {N}%\)$"),
     lambda m: {"kind": "shield_percent_per_status", "status": m.group(1),
                "percent": int(m.group(2)), "max": int(m.group(3)),
                "condition": {"source": "self", "status": m.group(1), "component": "count"}}),
    # "When hit while this unit has Shield, inflict 3 [Sinking] against the
    # attacker (3 times per turn)"
    (re.compile(rf"^When hit while this unit has Shield, inflict {N} {ST} against the attacker$"),
     lambda m: {"kind": "inflict_on_attacker", "status": m.group(2), "potency": int(m.group(1)),
                "requires_shield": True}),
    # "[On Hit] Deal +2% damage for every value of [X] spent by this Skill"
    (re.compile(rf"^Deal \+{N}% damage for every value of {ST} spent by this Skill$"),
     lambda m: {"kind": "damage_percent_per_ammo_spent", "status": m.group(2),
                "step": int(m.group(1))}),
    # "[On Hit] Inflict Gloom Damage equal to "All" [Butterfly] on target"
    (re.compile(rf'^Inflict Gloom Damage equal to "All" {ST} on target$'),
     lambda m: {"kind": "gloom_damage_equal_target_status", "status": m.group(1)}),
    # Explanatory footnote for the line above it.
    (re.compile(r'^"All" = the sum of both The Living and The Departed on target$'),
     lambda m: {"kind": "noop"}),
    # Sin Resonance payoffs (Yi Sang's Solemn Lament).
    (re.compile(rf"^Gain \(highest Reson\.\) of {ST} \(max {N}\)$"),
     lambda m: {"kind": "gain_from_resonance", "status": m.group(1), "max": int(m.group(2)),
                "multiplier": 1}),
    (re.compile(rf"^Gain \(highest Reson\. x {N}\) of {ST} \(max {N}\)$"),
     lambda m: {"kind": "gain_from_resonance", "status": m.group(2), "max": int(m.group(3)),
                "multiplier": int(m.group(1))}),
    (re.compile(rf"^If the said Reson\. was an A-Reson\., gain \(highest Reson\. x {N}\) of {ST} \(max {N}\)$"),
     lambda m: {"kind": "gain_from_resonance", "status": m.group(2), "max": int(m.group(3)),
                "multiplier": int(m.group(1)), "requires_a_reson": True}),
    (re.compile(rf"^If the said Reson\. was at {N}\+ A-Reson\., \[Reload \(Solemn Lament\)\] instead$"),
     lambda m: {"kind": "reload_ammo", "a_reson_gte": int(m.group(1))}),
    (re.compile(rf"^If the said Reson\. was an A-Reson\., {ST} instead$"),
     lambda m: {"kind": "reload_ammo", "requires_a_reson": True}),
    # Discard ("[Discard] that Skill" / "[Discard] 2 Skills of the lowest rank").
    (re.compile(r"^If the other Skill in the same Skill Slot is a different Skill, \[Discard\] that Skill$"),
     lambda m: {"kind": "discard_other_in_slot"}),
    (re.compile(rf"^\[Discard\] {N} Skills? of the lowest rank in all of this unit's Skill Slots$"),
     lambda m: {"kind": "discard_lowest_rank", "value": int(m.group(1))}),
    # "Gain +3 [Aggro] to this Skill Slot next turn" (modelled unit-wide).
    (re.compile(rf"^Gain \+{N} {ST} to this Skill Slot$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "count": int(m.group(1)), "next_turn": True,
                "assumption": "slot_scoped"}),
    (re.compile(rf"^Gain \+{N} {ST} to this Skill Slot next turn$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "count": int(m.group(1)), "next_turn": True,
                "assumption": "slot_scoped"}),
    # "At 2 or fewer [LCA Fracture Round], [Reload]"
    (re.compile(rf"^At {N} or fewer {ST}, \[Reload\]$"),
     lambda m: {"kind": "reload_ammo",
                "condition": {"source": "self", "status": m.group(2), "lte": int(m.group(1))}}),
    # "Base Power +1 for every [X] about to be spent by this Skill"
    (re.compile(rf"^Base Power \+{N} for every {ST} about to be spent by this Skill$"),
     lambda m: {"kind": "base_power_per_ammo_planned", "status": m.group(2),
                "step": int(m.group(1))}),
    # "Deal +([X] spent x 30)% damage"
    (re.compile(rf"^Deal \+\({ST} spent x {N}\)% damage$"),
     lambda m: {"kind": "damage_percent_per_ammo_spent", "status": m.group(1),
                "step": int(m.group(2))}),
    # "Coin Power +1 for every 2 [Protecting Sword] (max 2)"
    (re.compile(rf"^Coin Power \+{N} for every {N} {ST} \(max {N}\)$"),
     lambda m: {"kind": "coin_power", "value": 0, "step": int(m.group(1)), "per": int(m.group(2)),
                "max": int(m.group(4)),
                "condition": {"source": "self", "status": m.group(3), "component": "stack"}}),
    # "Deal +1% damage for every [Faint Aroma] on target" (no max)
    (re.compile(rf"^Deal \+{N}% damage for every {ST} on (self|target)$"),
     lambda m: {"kind": "damage_percent", "value": 0, "step": int(m.group(1)), "per": 1,
                "condition": {"source": m.group(3), "status": m.group(2), "component": "potency"}}),
    # "Inflict Gloom damage equal to [Sinking] on target"
    (re.compile(rf"^Inflict Gloom damage equal to {ST} on target$"),
     lambda m: {"kind": "gloom_damage_equal_target_status", "status": m.group(1),
                "component": "potency"}),
    # "Deal (30% of this Coin's final damage) Blunt damage"
    (re.compile(r"^Deal \((\d+)% of this Coin's final damage\) (?:Slash|Pierce|Blunt) damage$"),
     lambda m: {"kind": "bonus_damage_percent_of_coin", "percent": int(m.group(1))}),
    # "Lower user's Stagger Threshold by 50% of damage dealt"
    (re.compile(r"^Lower user’s Stagger Threshold by (\d+)% of damage dealt$"),
     lambda m: {"kind": "lower_own_stagger_threshold", "percent": int(m.group(1))}),
    # "If user has 5+ [Poise] Count, +30% Critical Damage"
    (re.compile(rf"^If user has {N}\+ {ST} Count, \+{N}% Critical Damage$"),
     lambda m: {"kind": "crit_damage_bonus", "percent": int(m.group(3)),
                "condition": {"source": "self", "status": m.group(2), "component": "count",
                              "gte": int(m.group(1))}}),
    # "If this unit has [Tear-sharpened], lose ([Tear-sharpened] Stack x 15) more SP"
    (re.compile(rf"^If this unit has {ST}, lose \({ST} Stack x {N}\) more SP$"),
     lambda m: {"kind": "sp_damage_self_per_stack", "status": m.group(2),
                "step": int(m.group(3))}),
    # "At less than 3 [Tear-sharpened], lose 15 SP and gain 1 [Tear-sharpened]"
    (re.compile(rf"^At less than {N} {ST}, lose {N} SP and gain {N} {ST}$"),
     lambda m: {"kind": "turn_end_sp_and_gain", "status": m.group(2),
                "threshold": int(m.group(1)), "value": int(m.group(3)),
                "count": int(m.group(4))}),
    # "At 10+ [Deep Tears], consume up to 20 [Deep Tears]"
    (re.compile(rf"^At {N}\+ {ST}, consume up to {N} {ST}$"),
     lambda m: {"kind": "consume_status_up_to", "status": m.group(2),
                "threshold": int(m.group(1)), "value": int(m.group(3))}),
    # "At 15+ [X], consume 5 [X] to deal +15% damage" already handled
    # "If target's SP is below 0, boost crit chance proportional to target's SP"
    (re.compile(r"^boost crit chance proportional to target's SP$"),
     lambda m: {"kind": "crit_chance_from_target_sp"}),
    # "Deal +(Stack consumed x 1.5)% damage"
    (re.compile(r"^Deal \+\(Stack consumed x ([\d.]+)\)% damage$"),
     lambda m: {"kind": "damage_percent_per_consumed_status",
                "step": int(float(m.group(1)) * 10)}),
    # Damage scaling off the user's own Stacks / Potency / Count.
    (re.compile(rf"^Deal \+\({ST} x {N}\)% damage \(max {N}%\)$"),
     lambda m: {"kind": "damage_percent", "value": 0, "step": int(m.group(2)), "per": 1,
                "max": int(m.group(3)),
                "condition": {"source": "self", "status": m.group(1), "component": "stack"}}),
    (re.compile(rf"^Deal \+\({ST} on self\)% damage \(max {N}%\)$"),
     lambda m: {"kind": "damage_percent", "value": 0, "step": 1, "per": 1,
                "max": int(m.group(2)),
                "condition": {"source": "self", "status": m.group(1), "component": "potency"}}),
    (re.compile(rf"^[Dd]eal \+\({ST} Count on self\)% damage \(max {N}%\)$"),
     lambda m: {"kind": "damage_percent", "value": 0, "step": 1, "per": 1,
                "max": int(m.group(2)),
                "condition": {"source": "self", "status": m.group(1), "component": "count"}}),
    (re.compile(rf"^Deal \+\({ST} on self x {N}\)% damage \(max {N}%\)$"),
     lambda m: {"kind": "damage_percent", "value": 0, "step": int(m.group(2)), "per": 1,
                "max": int(m.group(3)),
                "condition": {"source": "self", "status": m.group(1), "component": "potency"}}),
    (re.compile(rf"^Halve {ST} \(rounded down\)$"),
     lambda m: {"kind": "halve_status", "status": m.group(1)}),
    (re.compile(rf"^While Clashing with this Skill, the main target's {ST} Count does not drop below 1$"),
     lambda m: {"kind": "tag", "tag": format_status_count_floor(m.group(1))}),
    (re.compile(rf"^Deal \({ST} on self / {N}\) (Wrath|Lust|Sloth|Gluttony|Gloom|Pride|Envy) damage on target and lose {N} {ST} Count(?: \(rounded down\))?$"),
     lambda m: {"kind": "damage_from_status_divisor", "status": m.group(1),
                "value": int(m.group(2)), "sin": m.group(3).lower(), "count": int(m.group(4))}),
    # "Target cannot be Staggered until this Skill's Attack End"
    (re.compile(r"^Target cannot be Staggered until this Skill's Attack End$"),
     lambda m: {"kind": "tag", "tag": "no_stagger_target"}),
    # "If target's Pierce Resist. is below Weak (1.5), treat is as Weak (1.5)"
    (re.compile(r"^If target's (Slash|Pierce|Blunt) Resist\. is below \"Weak\" \(1\.5\), treat is? as Weak \(1\.5\)(?: \(max \d+%\))?"),
     lambda m: {"kind": "resist_floor", "status": m.group(1).lower(), "value": 15}),
    # "[Attack End] If target is killed, Reuse this Skill on the target that has
    # the highest HP (once per turn)"
    (re.compile(r"^If target is killed, Reuse this Skill on the target that has the highest HP$"),
     lambda m: {"kind": "reuse_on_kill"}),
    # "[Before Attack] If this unit has 20+ [Poise] Potency, consume up to 20
    # surplus [Poise] Potency past 20 to deal +([Poise] consumed x 5)% damage"
    (re.compile(rf"^If this unit has {N}\+ {ST} Potency, consume up to {N} surplus {ST} Potency past {N} to deal \+\({ST} consumed x {N}\)% damage(?:\(max {N}%\)| \(max {N}%\))?$"),
     lambda m: {"kind": "consume_surplus_status", "status": m.group(2),
                "threshold": int(m.group(1)), "value": int(m.group(3)),
                "step": int(m.group(7)), "max": 100}),
    # Implicit gain from a bullet: "4 [Poise] and +4 [Poise] Count"
    (re.compile(rf"^{N} {ST} and \+{N} {ST} Count$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "count": int(m.group(3)), "status2": m.group(4)}),
    (re.compile(r"^On Clash Lose, this effect does not activate$"),
     lambda m: {"kind": "noop", "note": "clash-lose gate"}),
    (re.compile(r"^deal \+\((\d+)% of this Coin's damage\)% bonus (?:Slash|Pierce|Blunt) damage$", re.I),
     lambda m: {"kind": "bonus_damage_percent_of_coin", "percent": int(m.group(1))}),
    (re.compile(r"^This Attack Skill deals 0 damage$"),
     lambda m: {"kind": "zero_damage"}),
    (re.compile(r"^Does not take damage for this turn$"),
     lambda m: {"kind": "no_damage_taken"}),
    # Trigger [Tremor Burst]; then, reduce target's [Tremor] Count by 1
    (re.compile(r"^Trigger \[([^\]]+)\]; then, reduce target's \[([^\]]+)\] Count by (\d+)$"),
     lambda m: {"kind": "tremor_burst" if m.group(1) == "Tremor Burst" else "activate_status",
                "status": m.group(1), "consume_count": int(m.group(3))}),
    (re.compile(r"^[Aa]ctivate \[([^\]]+)\] on (?:the main )?target (once|twice|\d+ times?)\. Target loses (\d+) \[([^\]]+)\] Count$"),
     lambda m: {"kind": "activate_status", "status": m.group(1),
                "times": {"once": 1, "twice": 2}.get(m.group(2), None) or int(re.sub(r"\D", "", m.group(2)) or 1),
                "consume_count": int(m.group(3))}),
    (re.compile(rf"^Gain Shield equal to \(the sum of (?:both )?{ST} on the selected target\)% HP \(max {N}% per turn\)$"),
     lambda m: {"kind": "shield_percent_hp", "percent": 1, "max": int(m.group(2)),
                "condition": {"source": "target", "statuses": [m.group(1), m.group(1)]}}),
    (re.compile(r"^\[?Reload \(Solemn Lament\)\]?( \(once per turn\))?$"),
     lambda m: {"kind": "reload_ammo"}),
    # a bare [Unbreakable Coin] on a coin means that coin is unbreakable
    (re.compile(r"^\[Unbreakable Coin\]$"),
     lambda m: {"kind": "unbreakable_coin", "which": "current"}),
    (re.compile(rf"^Spend all of {ST} on self$"),
     lambda m: {"kind": "spend_ammo_all", "status": m.group(1)}),
    (re.compile(rf"^If target has {N}\+ \({ST} \+ {ST}\), Coin Power \+{N}$"),
     lambda m: {"kind": "coin_power", "value": int(m.group(4)),
                "condition": {"source": "target", "statuses": [m.group(2), m.group(3)],
                              "component": "potency", "gte": int(m.group(1))}}),
    (re.compile(rf"^If target has {N}\+ {ST}, Coin Power \+{N}$"),
     lambda m: {"kind": "coin_power", "value": int(m.group(3)),
                "condition": {"source": "target", "status": m.group(2), "component": "potency",
                              "gte": int(m.group(1))}}),
    (re.compile(rf"^Base Power \+{N} for every {N} value of {ST} \(max {N}\)$"),
     lambda m: {"kind": "base_power", "value": 0, "step": int(m.group(1)), "per": int(m.group(2)),
                "max": int(m.group(4)),
                "condition": {"source": "self", "status": m.group(3), "component": "potency"}}),
    (re.compile(rf"^Clash Power \+{N} for every {N} {ST} \(max {N}\)$"),
     lambda m: {"kind": "clash_power", "value": 0, "step": int(m.group(1)), "per": int(m.group(2)),
                "max": int(m.group(4)),
                "condition": {"source": "self", "status": m.group(3), "component": "stack"}}),
    (re.compile(rf"^Deal \+{N}% damage for every {ST} Potency on self$"),
     lambda m: {"kind": "damage_percent", "value": 0, "step": int(m.group(1)), "per": 1,
                "condition": {"source": "self", "status": m.group(2), "component": "potency"}}),
    (re.compile(rf"^Gain {N} additional {ST} for every {N} {ST} \(max {N}\)$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "value": int(m.group(1)), "step": 0, "per": int(m.group(5)), "max": int(m.group(1)) + int(m.group(5)),
                "condition": {"source": "self", "status": m.group(4), "component": "stack"}}),

    # ------------------------------------------------------------------ #
    # Heals (HP / SP) and their ally scoping
    # ------------------------------------------------------------------ #
    (re.compile(rf"^Heal {N} SP to self and {N} other allies with the least SP$"),
     lambda m: {"kind": "sp_heal", "value": int(m.group(1)), "ally": "lowest_sp",
                "ally_count": int(m.group(2)), "include_self": True}),
    (re.compile(rf"^Heal {N} SP for {N} other allies with the least SP$"),
     lambda m: {"kind": "sp_heal", "value": int(m.group(1)), "ally": "lowest_sp",
                "ally_count": int(m.group(2)), "include_self": False}),
    (re.compile(rf"^Heal {N} SP for \(1 \+ highest Reson\.\) other allies with the least SP \(max {N} units\)$"),
     lambda m: {"kind": "sp_heal", "value": int(m.group(1)), "ally": "lowest_sp",
                "ally_count": 1, "from_resonance": True, "max": int(m.group(2)),
                "include_self": False}),
    (re.compile(rf"^Heal \(# of Coin {N} hits x {N}\) SP$"),
     lambda m: {"kind": "heal_per_coin_hits", "value": int(m.group(2)),
                "ally": "self"}),
    (re.compile(rf"^Heal {N} SP$"),
     lambda m: {"kind": "sp_heal", "value": int(m.group(1))}),
    (re.compile(rf"^Heal {N} (?:other )?allies with the lowest HP percentages by \({N} \+ \({ST} on the main target \+ {ST} Count on the main target\)/{N}\)% HP \(max {N}%\)$"),
     lambda m: {"kind": "heal_percent_hp", "value": int(m.group(2)), "ally": "lowest_hp",
                "ally_count": int(m.group(1)), "per": int(m.group(5)), "step": 1,
                "max": int(m.group(6)), "include_self": "other" not in m.group(0),
                "condition": {"source": "target", "statuses": [m.group(3), m.group(4)]}}),
    (re.compile(rf"^Heal additional SP \(to self & affected allies\) equal to {ST} Potency on target \(Max SP heal: {N}\)$"),
     lambda m: {"kind": "heal_from_status", "heal_from_status": m.group(1),
                "heal_from_component": "potency", "max": int(m.group(2)),
                "ally": "all_allies", "include_self": True}),
    # ------------------------------------------------------------------ #
    # Tremor amplitudes
    # ------------------------------------------------------------------ #
    (re.compile(rf"^If target isn't in an {ST} state, trigger {ST} into {ST}$"),
     lambda m: {"kind": "amplitude_conversion", "amplitude_into": m.group(3),
                "condition": {"source": "target", "lacks_status": [m.group(1)]}}),
    (re.compile(rf"^If target is in either {ST} or {ST} states, this Coin deals \+{N}% damage$"),
     lambda m: {"kind": "damage_percent", "value": int(m.group(3)),
                "condition": {"target_has_amplitude": True}}),
    # ------------------------------------------------------------------ #
    # Chance-based gains and self-inflicted damage
    # ------------------------------------------------------------------ #
    (re.compile(rf"^{N}% chance to gain {N} {ST} Potency$"),
     lambda m: {"kind": "gain", "status": m.group(3), "potency": int(m.group(2)),
                "chance": int(m.group(1))}),
    (re.compile(rf"^Lose HP by {N}% of Max HP$"),
     lambda m: {"kind": "self_damage", "self_damage_percent": int(m.group(1)),
                "no_stagger": True}),
    (re.compile(rf"^At {N}%\+ HP, take {N} ~ {N} HP damage$"),
     lambda m: {"kind": "self_damage", "self_damage_min": int(m.group(2)),
                "self_damage_max": int(m.group(3)),
                "condition": {"hp_above_percent": int(m.group(1))}}),
    (re.compile(rf"^Lose {N} ~ {N} SP$"),
     lambda m: {"kind": "self_sp_damage", "self_sp_damage_min": int(m.group(1)),
                "self_sp_damage_max": int(m.group(2))}),
    (re.compile(rf"^For {N} turns?, lose {N} SP at Combat End$"),
     lambda m: {"kind": "combat_end_sp_damage", "value": int(m.group(2)),
                "turns": int(m.group(1))}),
    # ------------------------------------------------------------------ #
    # Rodion: Deep Tears / Tear-sharpened / Protecting Sword
    # ------------------------------------------------------------------ #
    (re.compile(rf"^If target is defeated, regain half of {ST} consumed by this Skill$"),
     lambda m: {"kind": "refund_consumed_status", "status": m.group(1), "refund_percent": 50,
                "condition": {"target_defeated": True}}),
    (re.compile(rf"^Base Power \+{N} for every {N} Stack consumed$"),
     lambda m: {"kind": "base_power_from_consumed", "step": int(m.group(1)),
                "per": int(m.group(2))}),
    (re.compile(rf"^Clash Power \+{N} for every {ST} \(max {N}\)$"),
     lambda m: {"kind": "clash_power", "value": 0, "step": int(m.group(1)), "per": 1,
                "max": int(m.group(3)),
                "condition": {"source": "self", "status": m.group(2), "component": "stack"}}),
    (re.compile(rf"^lose \({ST} Stack x {N}\) more SP$"),
     lambda m: {"kind": "sp_damage_self_per_stack", "status": m.group(1),
                "step": int(m.group(2))}),
    (re.compile(rf"^At less than {N} {ST}, lose {N} SP to gain {N} {ST}$"),
     lambda m: {"kind": "turn_end_sp_and_gain", "status": m.group(2),
                "threshold": int(m.group(1)), "value": int(m.group(3)),
                "count": int(m.group(4))}),
    # ------------------------------------------------------------------ #
    # Outis: The Udjat -Vanguard-, Tremor Burst, Protection
    # ------------------------------------------------------------------ #
    (re.compile(rf"^If this unit has {ST}, Clash Power \+{N} and deal \+{N}% damage$"),
     lambda m: {"kind": "compound", "condition": {"source": "self", "status": m.group(1), "gte": 1},
                "sub_effects": [{"kind": "clash_power", "value": int(m.group(2))},
                                {"kind": "damage_percent", "value": int(m.group(3))}]}),
    (re.compile(rf"^If target is a SP Unit, trigger {ST}; then, reduce target's {ST} Count by {N}$"),
     lambda m: {"kind": "tremor_burst", "status": m.group(1), "consume_count": int(m.group(3)),
                "condition": {"target_is_sp_unit": True}}),
    (re.compile(rf"^At {N}\+ \({ST} \+ {ST}\) on target, Coin Power \+{N}$"),
     lambda m: {"kind": "coin_power", "value": int(m.group(4)),
                "condition": {"source": "target", "statuses": [m.group(2), m.group(3)],
                              "gte": int(m.group(1))}}),
    (re.compile(rf"^If this unit has {ST}, gain {N} {ST}$"),
     lambda m: {"kind": "gain", "status": m.group(3), "potency": int(m.group(2)),
                "condition": {"source": "self", "status": m.group(1), "gte": 1}}),
    # ------------------------------------------------------------------ #
    # Gregor: Dazzle conditions, Lamp, Haste to the slowest allies
    # ------------------------------------------------------------------ #
    (re.compile(rf"^If the target\(Core\) has {N}% or less HP, or if target has {ST}, deal \+{N}% damage$"),
     lambda m: {"kind": "damage_percent", "value": int(m.group(3)),
                "condition": {"any_of": [
                    {"source": "target", "hp_below_percent": int(m.group(1)), "hp_or_equal": True},
                    {"source": "target", "status": m.group(2), "gte": 1}]}}),
    (re.compile(rf"^If target is defeated, or if it has {N} {ST}, gain {N} {ST}$"),
     lambda m: {"kind": "gain", "status": m.group(4), "potency": int(m.group(3)),
                "condition": {"any_of": [{"target_defeated": True},
                    {"source": "target", "status": m.group(2), "gte": int(m.group(1))}]}}),
    (re.compile(rf"^Gain {ST} up to {N} Stack; for every Stack gained, take HP damage equal to {N}% of max HP$"),
     lambda m: {"kind": "gain_up_to_with_self_damage", "status": m.group(1),
                "up_to": int(m.group(2)), "self_damage_percent": int(m.group(3)),
                "hp_floor_one": True}),
    (re.compile(rf"^Apply {N} {ST} next turn to \({ST} on self / {N}\) other allies with the slowest Speed$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "next_turn": True, "ally": "slowest", "ally_from_status": m.group(3),
                "ally_from_divisor": int(m.group(4)), "include_self": False}),
    # ------------------------------------------------------------------ #
    # E.G.O: Sin Resonance gates, attack adders and Butterfly
    # ------------------------------------------------------------------ #
    (re.compile(rf"^Randomly inflict \({N} \+ Gloom Reson\.\) {ST} between targets$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "value": int(m.group(1)),
                "multiplier": 1, "resonance_of": "Gloom"}),
    (re.compile(rf"^If target has {N}\+ {ST}, inflict {N} {ST}$"),
     lambda m: {"kind": "inflict", "status": m.group(4), "potency": int(m.group(3)),
                "condition": {"source": "target", "status": m.group(2), "gte": int(m.group(1))}}),
    (re.compile(rf"^At {N}\+ \(Gloom Reson\.\), Atk Weight \+{N}$"),
     lambda m: {"kind": "attack_weight", "value": int(m.group(2)),
                "condition": {"resonance_gte": int(m.group(1)), "resonance_of": "Gloom"}}),
    (re.compile(rf"^At {N}\+ highest Reson\., gain \+{N} Atk Weight$"),
     lambda m: {"kind": "attack_weight", "value": int(m.group(2)),
                "condition": {"resonance_gte": int(m.group(1))}}),
    (re.compile(rf"^Gain \+\(highest Reson\. / {N}\) Atk Weight \(max {N}, rounded down\)$"),
     lambda m: {"kind": "attack_weight", "value": 0, "per": int(m.group(1)), "step": 1,
                "max": int(m.group(2)), "from_resonance": True}),
    (re.compile(rf"^Gain \+{N} Atk Weight \(max {N}\)$"),
     lambda m: {"kind": "attack_weight", "value": int(m.group(1))}),
    (re.compile(rf"^If target[’']s HP is above {N}%, (?:deal )?\+{N}% [Dd]amage$"),
     lambda m: {"kind": "damage_percent", "value": int(m.group(2)),
                "condition": {"source": "target", "hp_above_percent": int(m.group(1))}}),
    (re.compile(rf"^Deal \+{N}% damage \(max {N}%\)$"),
     lambda m: {"kind": "damage_percent", "value": int(m.group(1)), "max": int(m.group(2))}),
    (re.compile(r"^Trigger \[Tremor Burst\]$"),
     lambda m: {"kind": "tremor_burst"}),
    (re.compile(rf"^Deal more damage based on missing HP on self \(max {N}%\)$"),
     lambda m: {"kind": "damage_percent_missing_hp",
                "damage_percent_missing_hp": int(m.group(1))}),
    (re.compile(rf"^Then, Reuse this Coin \({N} times? per Skill\)$"),
     lambda m: {"kind": "reuse_coin", "reuse_coin": int(m.group(1))}),
    (re.compile(rf"^Then, Reuse this Coin \(once per Skill\)$"),
     lambda m: {"kind": "reuse_coin", "reuse_coin": 1}),
    (re.compile(rf"^Then, Reuse this Coin \({N} times? max\)$"),
     lambda m: {"kind": "reuse_coin", "reuse_coin": int(m.group(1))}),
    (re.compile(rf"^Reuse this Coin \({N} times? max\)$"),
     lambda m: {"kind": "reuse_coin", "reuse_coin": int(m.group(1))}),
    (re.compile(rf"^Reuse this Coin \((?:once|{N} times?) per Skill\)$"),
     lambda m: {"kind": "reuse_coin", "reuse_coin": int(m.group(1) or 1)}),
    (re.compile(r"^Reuse this Coin$"),
     lambda m: {"kind": "reuse_coin", "reuse_coin": 1}),
    # "Reuse this Coin ([X] Potency - 1) times (4 times max)" - the count is the
    # status minus one, capped.
    (re.compile(rf"^Reuse this Coin \({ST} Potency - {N}\) times \({N} times max\)$"),
     lambda m: {"kind": "reuse_coin", "reuse_coin": int(m.group(3)),
                "reuse_from_status": m.group(1), "reuse_minus": int(m.group(2))}),
    (re.compile(rf"^At {N}\+ SP, Reuse this Coin \({N} times? max per Skill\)$"),
     lambda m: {"kind": "reuse_coin", "reuse_coin": int(m.group(2)),
                "condition": {"self_sp_at_least": int(m.group(1))}}),
    (re.compile(rf"^At {N}\+ SP, Reuse this Coin$"),
     lambda m: {"kind": "reuse_coin", "reuse_coin": 1,
                "condition": {"self_sp_at_least": int(m.group(1))}}),
    (re.compile(rf"^Reuse this Coin \({N} times? max\)$"),
     lambda m: {"kind": "reuse_coin", "reuse_coin": int(m.group(1))}),
    (re.compile(r"^This damage does not Stagger or reduce this unit's HP below 1$"),
     lambda m: {"kind": "noop", "note": "hp floor 1"}),
    (re.compile(rf"^Deal -{N}% damage against sub-targets$"),
     lambda m: {"kind": "sub_target_damage", "percent": -int(m.group(1))}),
    (re.compile(rf"^For targets that are Non-SP Units, deal \+{N}% damage for every {N} {ST} on target \(max {N}%\)$"),
     lambda m: {"kind": "damage_percent", "value": 0, "step": int(m.group(1)),
                "per": int(m.group(2)), "max": int(m.group(4)),
                "condition": {"source": "target", "status": m.group(3),
                              "component": "potency", "target_is_non_sp_unit": True}}),
    (re.compile(rf"^If target has less than {N} SP, deal more damage the further their SP value is from 0 \(\+{N}% damage for every missing SP, max {N}%\)$"),
     lambda m: {"kind": "damage_percent_from_missing_sp", "step": int(m.group(2)),
                "max": int(m.group(3))}),
    (re.compile(rf"^deal more damage the further their SP value is from 0 \(\+{N}% damage for every missing SP, max {N}%\)$"),
     lambda m: {"kind": "damage_percent_from_missing_sp", "step": int(m.group(1)),
                "max": int(m.group(2))}),
    (re.compile(rf"^If target is defeated, inflict {N} {ST} and {N} {ST} on {N} random enemies \(For Focused Encounters, random Parts\)$"),
     lambda m: {"kind": "compound", "condition": {"target_defeated": True}, "sub_effects": [
        {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1)),
         "ally": "random", "ally_count": int(m.group(5))},
        {"kind": "inflict", "status": m.group(4), "potency": int(m.group(3)),
         "ally": "random", "ally_count": int(m.group(5))}]}),
    (re.compile(rf"^Inflict {N} {ST}\. Inflict {N} {ST}$"),
     lambda m: {"kind": "compound", "sub_effects": [
        {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1))},
        {"kind": "inflict", "status": m.group(4), "potency": int(m.group(3))}]}),
    (re.compile(rf"^All allies gain {N} {ST}, {N} {ST}, {N} {ST}, {N} {ST}, {N} {ST}$"),
     lambda m: {"kind": "compound", "sub_effects": [
        {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
         "ally": "all_allies", "include_self": True},
        {"kind": "gain", "status": m.group(4), "potency": int(m.group(3)),
         "ally": "all_allies", "include_self": True},
        {"kind": "gain", "status": m.group(6), "potency": int(m.group(5)),
         "ally": "all_allies", "include_self": True},
        {"kind": "gain", "status": m.group(8), "potency": int(m.group(7)),
         "ally": "all_allies", "include_self": True},
        {"kind": "gain", "status": m.group(10), "potency": int(m.group(9)),
         "ally": "all_allies", "include_self": True}]}),
    (re.compile(rf"^Apply {N} {ST} next turn to \(highest Reson\.\) random allies(?: \(including this unit; max {N} allies(?:; once per turn)?\))?$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "next_turn": True, "ally": "random", "ally_count": 1, "from_resonance": True,
                "max": int(m.group(3)) if m.group(3) else 4, "include_self": True}),
    (re.compile(r"^At 4\+ highest Reson\., heal 1 additional ally$"),
     lambda m: {"kind": "heal_extra_ally", "value": 1,
                "condition": {"resonance_gte": 4}}),
    (re.compile(rf"^Gain the following effects for every {N} highest Reson\.$"),
     lambda m: {"kind": "noop", "note": "per-Resonance block", "per": int(m.group(1)),
                "from_resonance": True}),
    (re.compile(rf"^convert the Suit in this unit's Hand to a random Suit that corresponds to one of this unit's Base Attack Skills$"),
     lambda m: {"kind": "suit_convert"}),
    (re.compile(rf"^If this Skill was equipped on this unit's leftmost Skill Slot, convert the Suit in this unit's Hand to a random Suit that corresponds to one of this unit's Base Attack Skills$"),
     lambda m: {"kind": "suit_convert", "condition": {"slot": "leftmost"}}),
    (re.compile(rf"^Inflict {N} {ST}\(The (Living|Departed)\)$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1)),
                "butterfly_part": m.group(3).lower()}),
    (re.compile(rf"^Deal Gloom damage equal to the sum of {ST} on target$"),
     lambda m: {"kind": "gloom_damage_equal_target_status", "status": m.group(1)}),
    (re.compile(rf"^Deal Gloom damage equal to \({ST} spent by this Coin x {N}\)% of this Coin's final damage$"),
     lambda m: {"kind": "extra_damage_percent_of_damage", "final_damage_percent": 0,
                "step": int(m.group(2)), "per_ammo": True}),
    (re.compile(rf"^Spend all {ST} on self$"),
     lambda m: {"kind": "spend_ammo_all", "status": m.group(1)}),
    (re.compile(rf"^Gain a random assortment of \(Gloom Reson\. \+ {N}\) {ST} \(max {N}\)$"),
     lambda m: {"kind": "gain_from_resonance", "status": m.group(2), "value": int(m.group(1)),
                "multiplier": 1, "max": int(m.group(3)), "resonance_of": "Gloom"}),
    (re.compile(r"^\(Chance to flip Heads\)% chance to inflict The Departed"),
     lambda m: {"kind": "tag", "tag": "butterfly_split"}),
    (re.compile(rf"^A random ally gains {N} ~ {N} {ST}$"),
     lambda m: {"kind": "gain", "status": m.group(3), "range_min": int(m.group(1)),
                "range_max": int(m.group(2)), "ally": "random", "ally_count": 1}),
    (re.compile(rf"^All allies gain {N} {ST}$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "ally": "all_allies", "include_self": True}),
    (re.compile(rf"^Inflict {N} {ST}\. Inflict {N} additional {ST} for every {N} Gloom Reson\. \(max {N}\)$"),
     lambda m: {"kind": "compound", "sub_effects": [
        {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1))},
        {"kind": "inflict", "status": m.group(4), "potency": int(m.group(3)),
         "per": int(m.group(5)), "step": 1, "max": int(m.group(6)),
         "multiplier": 1, "resonance_of": "Gloom"}]}),
    (re.compile(rf"^At {N}\+ Gloom Reson\., inflict {N} {ST}$"),
     lambda m: {"kind": "inflict", "status": m.group(3), "potency": int(m.group(2)),
                "condition": {"resonance_gte": int(m.group(1)), "resonance_of": "Gloom"}}),
    (re.compile(rf"^If target is defeated, inflict {N} {ST} and {N} {ST} on {N} random enemies$"),
     lambda m: {"kind": "compound", "condition": {"target_defeated": True}, "sub_effects": [
        {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1)),
         "ally": "random", "ally_count": int(m.group(5))},
        {"kind": "inflict", "status": m.group(4), "potency": int(m.group(3)),
         "ally": "random", "ally_count": int(m.group(5))}]}),
    (re.compile(rf"^If target survives this attack, consume {N} {ST} Count on target, then heal additional SP \(to self & affected allies\) equal to {ST} Potency on target \(Max SP heal: {N}\)$"),
     lambda m: {"kind": "compound", "condition": {"target_survived": True}, "sub_effects": [
        {"kind": "lose_status_count", "status": m.group(2), "value": int(m.group(1))},
        {"kind": "heal_from_status", "heal_from_status": m.group(3),
         "heal_from_component": "potency", "max": int(m.group(4)),
         "ally": "all_allies", "include_self": True}]}),
    (re.compile(rf"^If there are Staggered, Part broken, or killed units among the targets, inflict {N} {ST} against a random non-targeted enemy$"),
     lambda m: {"kind": "inflict_on_random_other", "status": m.group(2),
                "potency": int(m.group(1))}),
    (re.compile(rf"^If there are Staggered, Part broken, or killed units among the targets, inflict {N} {ST} next turn against a random non-targeted enemy$"),
     lambda m: {"kind": "inflict_on_random_other", "status": m.group(2),
                "potency": int(m.group(1)), "next_turn": True}),
    (re.compile(rf"^Apply {N} {ST} next turn to \(highest Reson\.\) random allies \(including this unit; max {N} allies\)$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "next_turn": True, "ally": "random", "ally_count": 1,
                "from_resonance": True, "max": int(m.group(3)), "include_self": True}),
    (re.compile(rf"^If the said Reson\. was a Gloom Reson\. or an A-Reson\., apply {N} {ST} as well$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "condition": {"resonance_gte": 1, "resonance_of": "Gloom"}}),
    (re.compile(rf"^If the main target has higher than {N} Gloom Resist\., deal \+{N}% damage for every 0\.1 excess Resist\. \(max {N}%\)$"),
     lambda m: {"kind": "damage_percent_from_resist", "value": int(m.group(1)) * 10,
                "step": int(m.group(2)), "max": int(m.group(3))}),
    (re.compile(rf"^At {N}\+ (Gloom|Wrath|Lust|Sloth|Gluttony|Pride|Envy) Reson\., deal \+{N}% damage for every (?:Gloom|Wrath|Lust|Sloth|Gluttony|Pride|Envy) Reson\. \(max {N}%\)$"),
     lambda m: {"kind": "damage_percent_per_resonance", "step": int(m.group(3)),
                "max": int(m.group(4)), "resonance_of": m.group(2),
                "condition": {"resonance_gte": int(m.group(1)), "resonance_of": m.group(2)}}),

    # ------------------------------------------------------------------ #
    # Remaining clauses (second pass)
    # ------------------------------------------------------------------ #
    (re.compile(rf"^[Dd]eal Gloom damage equal to {ST} on target$"),
     lambda m: {"kind": "gloom_damage_equal_target_status", "status": m.group(1),
                "component": "potency"}),
    (re.compile(rf"^Coin Power \+{N} for every {N} \({ST} \+ {ST}\), on target \(max {N}\)$"),
     lambda m: {"kind": "coin_power", "value": 0, "step": int(m.group(1)), "per": int(m.group(2)),
                "max": int(m.group(5)),
                "condition": {"source": "target", "statuses": [m.group(3), m.group(4)]}}),
    (re.compile(rf"^Coin Power \+{N} for every {N} {ST} Potency on self \(max {N}\)$"),
     lambda m: {"kind": "coin_power", "value": 0, "step": int(m.group(1)), "per": int(m.group(2)),
                "max": int(m.group(4)),
                "condition": {"source": "self", "status": m.group(3), "component": "potency"}}),
    (re.compile(rf"^Clash Power \+{N} and deal \+{N}% damage$"),
     lambda m: {"kind": "compound", "sub_effects": [
        {"kind": "clash_power", "value": int(m.group(1))},
        {"kind": "damage_percent", "value": int(m.group(2))}]}),
    (re.compile(rf"^Inflict {N} {ST}\. Inflict {N} {ST}$"),
     lambda m: {"kind": "compound", "sub_effects": [
        {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1))},
        {"kind": "inflict", "status": m.group(4), "potency": int(m.group(3))}]}),
    (re.compile(r"^If Lobotomy E\.G\.O::Solemn Lament Yi Sang used this Skill:$"),
     lambda m: {"kind": "noop", "note": "identity gate (always true for this team)"}),
    (re.compile(rf"^If the said Reson\. was a Gloom Reson\., gain \+{N} Atk Weight$"),
     lambda m: {"kind": "attack_weight", "value": int(m.group(1)),
                "condition": {"resonance_gte": 1, "resonance_of": "Gloom"}}),
    (re.compile(r"^When attacking just a single target, apply it to 3 targets instead$"),
     lambda m: {"kind": "noop", "note": "single-target skills hit 3"}),
    (re.compile(rf"^Gain Atk Weight equal to \(highest Reson\. / {N}\) \(max {N}$"),
     lambda m: {"kind": "attack_weight", "value": 0, "per": int(m.group(1)), "step": 1,
                "max": int(m.group(2)), "from_resonance": True}),
    (re.compile(r"^All 'On Hit' effects and all damage dealt by each Coin are inflicted only against the random target the Coin selected$"),
     lambda m: {"kind": "tag", "tag": "random_coin_targets"}),
    (re.compile(r"^The first Coin always targets the main target$"),
     lambda m: {"kind": "noop", "note": "first Coin keeps the main target"}),
    (re.compile(r"^When this Skill flips Coins, each Coin flips against a random enemy among its targets\.?$"),
     lambda m: {"kind": "noop", "note": "random Coin targets"}),
    (re.compile(r"^Then, Reuse this Coin$"),
     lambda m: {"kind": "reuse_coin", "reuse_coin": 1}),
    (re.compile(rf"^When inflicting {ST} using this Skill's effects: \(Chance to flip Heads\)% chance to inflict The Departed.*$"),
     lambda m: {"kind": "tag", "tag": "butterfly_split"}),
    (re.compile(rf"^Clash Power \+{N} for every {N} \({ST} \+ {ST}\) on the main target \(max {N}\)$"),
     lambda m: {"kind": "clash_power", "value": 0, "step": int(m.group(1)), "per": int(m.group(2)),
                "max": int(m.group(5)),
                "condition": {"source": "target", "statuses": [m.group(3), m.group(4)],
                              "component": "potency"}}),
    (re.compile(rf"^Gain Atk Weight equal to \(highest Reson\. / {N}\) \(max {N}\)$"),
     lambda m: {"kind": "attack_weight", "value": 0, "per": int(m.group(1)), "step": 1,
                "max": int(m.group(2)), "from_resonance": True}),
    (re.compile(rf"^If 1 or more targets are killed, deal \({ST} Potency on each target / {N}\) Gloom damage against {N} random enemies \(max {N};.*\)$"),
     lambda m: {"kind": "gloom_damage_equal_target_status", "status": m.group(1),
                "value": int(m.group(2)), "max": int(m.group(4)),
                "ally": "random", "ally_count": int(m.group(3)),
                "condition": {"any_target_killed": True}}),
    (re.compile(r"^When this Skill flips Coins, each Coin flips against a random enemy among its targets\.?$"),
     lambda m: {"kind": "noop", "note": "random Coin targets"}),
    (re.compile(r"^Then, Reuse this Coin$"),
     lambda m: {"kind": "reuse_coin", "reuse_coin": 1}),
    (re.compile(rf"^Inflict {N}~{N} random {ST}$"),
     lambda m: {"kind": "inflict", "status": m.group(3), "range_min": int(m.group(1)),
                "range_max": int(m.group(2)), "assumption": "random part"}),
    (re.compile(rf"^Lose {N}~{N} SP$"),
     lambda m: {"kind": "self_sp_damage", "self_sp_damage_min": int(m.group(1)),
                "self_sp_damage_max": int(m.group(2))}),
    (re.compile(rf"^At {N}%\+ HP, take {N}~{N} HP damage$"),
     lambda m: {"kind": "self_damage", "self_damage_min": int(m.group(2)),
                "self_damage_max": int(m.group(3)),
                "condition": {"hp_above_percent": int(m.group(1))}}),
    (re.compile(rf"^If the main target has {N}\+ {ST}, Clash Power \+{N}$"),
     lambda m: {"kind": "clash_power", "value": int(m.group(3)),
                "condition": {"source": "target", "status": m.group(2), "gte": int(m.group(1))}}),
    (re.compile(rf"^If there are Staggered, Part broken, or killed units among the targets, inflict {N} {ST} against a random non-targeted enemy$"),
     lambda m: {"kind": "inflict_on_random_other", "status": m.group(2),
                "potency": int(m.group(1))}),
    (re.compile(rf"^If there are Staggered, Part broken, or killed units among the targets, inflict {N} {ST}$"),
     lambda m: {"kind": "inflict_on_random_other", "status": m.group(2),
                "potency": int(m.group(1))}),
    (re.compile(rf"^Apply {N} {ST} next turn to \(highest Reson\.\) random allies \(including this unit; max {N} allies$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "next_turn": True, "ally": "random", "ally_count": 1,
                "from_resonance": True, "max": int(m.group(3)), "include_self": True}),
    (re.compile(rf"^If target survives this attack, consume {N} {ST} Count on target, then heal additional SP\(to self & affected allies\) equal to {ST} Potency on target \(Max SP heal: {N}\)$"),
     lambda m: {"kind": "compound", "condition": {"target_survived": True}, "sub_effects": [
        {"kind": "lose_status_count", "status": m.group(2), "value": int(m.group(1))},
        {"kind": "heal_from_status", "heal_from_status": m.group(3),
         "heal_from_component": "potency", "max": int(m.group(4)),
         "ally": "all_allies", "include_self": True}]}),
    (re.compile(rf"^If target is defeated, inflict {N} {ST} and {N} {ST} on {N} random enemies$"),
     lambda m: {"kind": "compound", "condition": {"target_defeated": True}, "sub_effects": [
        {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1)),
         "ally": "random", "ally_count": int(m.group(5))},
        {"kind": "inflict", "status": m.group(4), "potency": int(m.group(3)),
         "ally": "random", "ally_count": int(m.group(5))}]}),
    (re.compile(rf"^Then, Reuse this Coin if the sum of HP lost due to this effect is less than {N}$"),
     lambda m: {"kind": "reuse_coin", "reuse_coin": 4, "threshold": int(m.group(1))}),
    (re.compile(r"^If target was killed, activate the effect above once more$"),
     lambda m: {"kind": "noop", "note": "repeat on kill"}),
]


TAG_PATTERNS = [
    (re.compile(r"^targets? randomly$", re.I), lambda m: "targets_random"),
    (re.compile(r"^targets the unit with the most hp$", re.I), lambda m: "targets_most_hp"),
    (re.compile(rf"^prioritizes targets that have the most {ST}$", re.I),
     lambda m: f"targets_most_status:{m.group(1)}"),
]


def parse_triggered(trigger: str, text: str, raw: str) -> Optional[dict]:
    """Turn one triggered clause into an effect dict (or None).

    The wiki writes "the sum of X and both [Y]" when both components of a
    Potency/Count status are summed; that is normalised to a status list where
    the status appears twice (summing Potency + Count).
    """
    text = text.strip()
    # Parentheticals that carry a limit plus a remark: "(max 15%; once per
    # turn)", "(once per turn; rounded down)", "(once per Coin)".
    LIMIT_WORDS = r"(once|twice|\d+ times?(?: max)?) per (turn|Encounter|Coin|Skill)"
    note_match = re.search(rf"\((max [\d.]+%);\s*{LIMIT_WORDS}(?:;[^)]*)?\)", text)
    if note_match:
        text = text[: note_match.start()] + f"({note_match.group(1)})" + text[note_match.end():]
    note_match = re.search(rf"\({LIMIT_WORDS}(?:;[^)]*)?\)", text)
    if note_match and not re.fullmatch(rf"\({LIMIT_WORDS}\)", note_match.group(0)):
        text = text[: note_match.start()] + text[note_match.end():]
    text = re.sub(r"\((max [\d.]+);\s*rounded down\)", r"(\1)", text)
    # The limits themselves are read below, before any remark is removed, so a
    # "(4 times per Skill)" is not swallowed by the remark pass.
    # "(this effect does not reduce this unit's HP below 1)" and friends.
    text = re.sub(
        r"\((?:this effect does not reduce this unit's HP below \d+|does not get Staggered due to this effect|excluding the Suit already in this unit's Hand)\)",
        "",
        text,
    ).strip()
    text = re.sub(r"\s{2,}", " ", text)
    # Trailing per-turn / per-encounter limits: "(2 times per turn)".
    limits = None
    limit_match = re.search(rf"\({LIMIT_WORDS}\)", text)
    if limit_match:
        word = limit_match.group(1)
        count = {"once": 1, "twice": 2}.get(word, None)
        if count is None:
            count = int(re.sub(r"\D", "", word) or 1)
        unit = limit_match.group(2)
        limits = {
            "per_turn" if unit in ("turn", "Coin") else ("per_skill" if unit == "Skill" else "per_encounter"): count
        }
        text = (text[: limit_match.start()] + text[limit_match.end():]).strip()
        # Tidy a dangling separator left inside a parenthetical.
        text = re.sub(r"\(max (\d+)%;\s*\)", r"(max \1%)", text)
        text = re.sub(r"\(max (\d+)%;?\)", r"(max \1%)", text)
    else:
        # No bare limit: drop remark parentheticals that merely mention one,
        # e.g. "(once per turn; excluding the Suit already in this unit's Hand)".
        text = re.sub(
            r"\([^()]*?(?:once|twice|\d+ times?) per (?:turn|Encounter|Coin|Skill)[^()]*\)",
            "",
            text,
        ).strip()
    # "next turn" buffs are applied at the start of the following turn.
    next_turn = False
    if text.endswith(" next turn") or text.endswith(" next turn."):
        next_turn = True
        text = re.sub(r"\s+next turn\.?$", "", text).strip()
    normalized = text.replace("both [", "[")
    if __import__("os").environ.get("LCB_DBG2"):
        print("DBG2 limits", limits, "text", repr(text))
    for pattern, handler in PATTERNS:
        match = pattern.match(normalized)
        if not match:
            continue
        effect = handler(match)
        if __import__("os").environ.get("LCB_DBG"):
            print("DBG matched", pattern.pattern, "limits", limits, "text", repr(normalized))
        if effect.get("kind") == "reuse_coin" and limits:
            effect["reuse_coin"] = (
                limits.get("per_skill") or limits.get("per_turn") or effect.get("reuse_coin") or 1
            )
            effect.pop("per_skill", None)
        # "both [X]" means X contributes Potency **and** Count, which the
        # evaluator expresses by listing X twice.  Patterns that already emit the
        # pair are left alone.
        both = re.search(r"both \[([^\]]+)\]", text)
        if both:
            name = both.group(1)
            for key in ("condition",):
                cond = effect.get(key)
                if not isinstance(cond, dict):
                    continue
                if cond.get("statuses"):
                    statuses = list(cond["statuses"])
                    if statuses.count(name) < 2:
                        statuses.append(name)
                    cond["statuses"] = statuses
                elif cond.get("status") == name:
                    cond["statuses"] = [name, name]
                    cond.pop("status")
                elif cond.get("status"):
                    cond["statuses"] = [cond["status"], name, name]
                    cond.pop("status")
        if trigger == "coin_clash_lose":
            effect["only_after_clash_lose"] = True
            effect["trigger"] = "coin"
        if trigger == "coin_reuse":
            effect["reuse_only"] = True
            effect["trigger"] = "coin"
        effect = apply_stack_component(effect)
        if limits:
            if effect.get("kind") == "reuse_coin":
                # The count already carries the "(N times per Skill)" cap.
                limits = {k: v for k, v in limits.items() if k != "per_skill"}
            effect.update(limits)
        if next_turn:
            effect["next_turn"] = True
        effect["raw"] = raw
        effect["trigger"] = trigger
        return effect
    return None


# Standalone condition bullets, used both on their own line and inside an
# "If any of the following conditions are met" block.
CONDITION_PATTERNS = [
    (re.compile(rf"^If target has {ST}$"),
     lambda m: {"source": "target", "status": m.group(1), "gte": 1}),
    (re.compile(r"^If this unit's Speed is (\d+) or slower$"),
     lambda m: {"self_speed_at_most": int(m.group(1))}),
    (re.compile(r"^If target's Speed is faster than this unit's by (\d+) or more$"),
     lambda m: {"target_speed_advantage": int(m.group(1))}),
    (re.compile(r"^If this unit's SP is at (\d+) or higher$"),
     lambda m: {"self_sp_at_least": int(m.group(1))}),
    (re.compile(r"^If the target\(Core\) has (\d+)% or less HP$"),
     lambda m: {"source": "target", "hp_below_percent": int(m.group(1)), "hp_or_equal": True}),
    (re.compile(r"^If this unit has (\d+)% or less HP$"),
     lambda m: {"hp_below_percent": int(m.group(1)), "hp_or_equal": True}),
]

ANY_OF_RE = re.compile(r"^If any of the following conditions are met, (.+)$")
INLINE_IF_RE = re.compile(rf"^If (?:the target|target) has {ST}, (.+)$")
INLINE_IF_SELF_RE = re.compile(rf"^If this unit has {ST}, (.+)$")
INLINE_IF_SP_RE = re.compile(r"^If target[’']s SP is below (\d+), (.+)$")
INLINE_IF_SP2_RE = re.compile(r"^If target has less than (\d+) SP, (.+)$")


def strip_limits(text: str):
    """Remove a trailing "(once per turn)" style limit; return (text, limits)."""
    match = re.search(r"\((once|twice|\d+ times?) per (turn|Encounter|Coin|Skill)\)", text)
    if not match:
        return text, None
    word = match.group(1)
    count = {"once": 1, "twice": 2}.get(word)
    if count is None:
        count = int(re.sub(r"\D", "", word) or 1)
    unit = match.group(2)
    key = {
        "turn": "per_turn",
        "Coin": "per_turn",
        "Skill": "per_skill",
        "Encounter": "per_encounter",
    }[unit]
    return (text[: match.start()] + text[match.end():]).strip(), {key: count}


def parse_condition_line(line: str):
    for pattern, handler in CONDITION_PATTERNS:
        match = pattern.match(line)
        if match:
            return handler(match)
    return None


def parse_any_of_conditions(lines, start: int):
    """Collect the condition bullets that follow an "any of the following" line."""
    conditions = []
    index = start
    while index < len(lines):
        candidate = lines[index].lstrip("-\u2022\u00b7 ").strip()
        if not candidate.startswith("If "):
            break
        condition = parse_condition_line(candidate)
        if condition is None:
            break
        conditions.append(condition)
        index += 1
    return conditions, index


TAG_LINES = {
    "can clash with this skill regardless of speed": "clash_any_speed",
    "[unclashable]": "unclashable",
    "[target fixed]": "target_fixed",
}


def format_status_count_floor(status: str) -> str:
    return f"status_count_floor:{status}"


def load_stack_statuses() -> set:
    """Statuses the engine stores as Stack (from data/statuses/statuses.json)."""
    path = os.path.join(DATA, "statuses", "statuses.json")
    if not os.path.exists(path):
        return set()
    with open(path, encoding="utf-8") as fh:
        records = json.load(fh)
    return {
        (record.get("name_en") or "")
        for record in records
        if record.get("structure") == "stack"
    }


STACK_STATUSES = load_stack_statuses()


def apply_stack_component(effect: dict) -> dict:
    """Grants of Stack-based statuses go to Stack, not Potency/Count."""
    if effect.get("kind") not in ("inflict", "gain"):
        return effect
    for key in ("status", "status2"):
        if effect.get(key) in STACK_STATUSES:
            effect["component"] = "stack"
            break
    return effect


def clean_line(line: str) -> str:
    return re.sub(r"\s+", " ", line).strip()


def split_trigger(line: str) -> Tuple[str, str]:
    match = TRIGGER_RE.match(line)
    if match:
        trigger = match.group("trigger").strip().lower()
        if trigger in TRIGGERS:
            return TRIGGERS[trigger], match.group("rest").strip()
    return "on_use", line


def expand_coin_effects(effect: dict, coin_count: int) -> dict:
    """Turn `which: the final Coin` into explicit coin indexes."""
    if effect.get("kind") != "unbreakable_coin":
        return effect
    which = effect.pop("which", None)
    if which == "all Coins":
        coins = list(range(1, coin_count + 1))
    elif which == "the final Coin":
        coins = [coin_count]
    elif which == "the second Coin":
        coins = [2]
    else:
        coins = []
    effect["coins"] = coins
    return effect


def parse_text_block(text: str, coin_count: int) -> Tuple[Dict[str, List[dict]], List[str]]:
    """Split a block of effect text into buckets, plus the unmodeled lines."""
    buckets: Dict[str, List[dict]] = {
        "on_use": [],
        "clash_win": [],
        "clash_lose": [],
        "attack_end": [],
        "combat_start": [],
        "before_attack": [],
        "on_kill": [],
        "on_evade": [],
        "turn_start": [],
        "turn_end": [],
        "heads_hit": [],
        "coin": [],
    }
    unmodeled: List[str] = []
    pending_trigger = "on_use"
    pending_resonance = None
    all_lines = text.split("\n")
    line_index = 0
    while line_index < len(all_lines):
        raw_line = all_lines[line_index]
        line_index += 1
        line = clean_line(raw_line)
        if not line:
            continue
        if line in ("-", "•"):
            continue
        stripped = line.lstrip("-•· ").strip()
        if stripped:
            line = stripped
        if re.fullmatch(r"\[[^\]]+\]", line):
            # a bare trigger token with no clause carries no effect of its own
            continue
        trigger, rest = split_trigger(line)
        # "[BeforeAttack] Gain the following effects for every N highest Reson."
        # scales each of the bullets that follow it.
        header = re.match(r"^Gain the following effects for every (\d+) highest Reson\.$", rest)
        if header:
            pending_resonance = int(header.group(1))
            continue
        # "At 4+ highest Reson., heal 1 additional ally" widens the heal above it.
        extra_ally = re.match(r"^At (\d+)\+ highest Reson\., heal (\d+) additional all", rest)
        if extra_ally:
            for bucket in buckets.values():
                for existing in reversed(bucket):
                    if existing.get("kind") in ("sp_heal", "heal_percent_hp", "heal_hp"):
                        clone = dict(existing)
                        clone["ally_count"] = int(extra_ally.group(2))
                        clone["include_self"] = False
                        clone.setdefault("condition", {})["resonance_gte"] = int(extra_ally.group(1))
                        clone["raw"] = line
                        bucket.append(clone)
                        break
                else:
                    continue
                break
            continue
        # "If any of the following conditions are met, <effect>" followed by the
        # condition bullets: attach the whole list as an any-of condition.
        any_of = ANY_OF_RE.match(rest)
        if any_of:
            conditions, next_index = parse_any_of_conditions(all_lines, line_index)
            line_index = next_index
            consumed_all = True
            for part in re.split(r"\s+and\s+", any_of.group(1)):
                for pattern, handler in PATTERNS:
                    match = pattern.match(part.strip())
                    if not match:
                        continue
                    effect = handler(match)
                    effect["raw"] = line
                    effect["trigger"] = trigger
                    condition = {"any_of": conditions} if conditions else None
                    if condition:
                        effect["condition"] = condition
                    buckets.setdefault(trigger if trigger != "coin" else "coin", []).append(effect)
                    break
                else:
                    consumed_all = False
            if not consumed_all or not conditions:
                unmodeled.append(line)
            continue
        condition_only = parse_condition_line(rest)
        if condition_only is not None and line.startswith("If "):
            # A condition bullet that was not consumed by a parent line.
            unmodeled.append(line)
            continue
        inline_hp = re.match(r"^At less than (\d+)% HP, (.+)$", rest)
        if inline_hp:
            condition = {"hp_below_percent": int(inline_hp.group(1))}
            effect = None
            for pattern, handler in PATTERNS:
                match = pattern.match(inline_hp.group(2))
                if not match:
                    continue
                effect = handler(match)
                effect["raw"] = line
                effect["trigger"] = trigger
                effect["condition"] = condition
                break
            if effect is not None:
                if trigger == "coin_clash_lose":
                    effect["only_after_clash_lose"] = True
                    effect["trigger"] = "coin"
                buckets.setdefault(trigger if trigger != "coin_clash_lose" else "coin", []).append(effect)
                continue
            # Not an "At less than N% HP" clause after all (e.g. "convert all
            # Coins into Unbreakable Coins"): fall through to the normal patterns.
        inline_if_sp = INLINE_IF_SP_RE.match(rest) or INLINE_IF_SP2_RE.match(rest)
        if inline_if_sp:
            condition = {"target_sp_below": int(inline_if_sp.group(1))}
            effect = None
            body, body_limits = strip_limits(inline_if_sp.group(2))
            for pattern, handler in PATTERNS:
                match = pattern.match(body) or pattern.match(body[:1].upper() + body[1:])
                if not match:
                    continue
                effect = handler(match)
                effect["raw"] = line
                effect["trigger"] = trigger
                effect["condition"] = condition
                if body_limits:
                    effect.update(body_limits)
                break
            if effect is not None:
                buckets.setdefault(trigger if trigger != "coin" else "coin", []).append(effect)
                continue
            unmodeled.append(line)
            continue
        inline_if_self = INLINE_IF_SELF_RE.match(rest)
        if inline_if_self:
            condition = {"source": "self", "status": inline_if_self.group(1), "gte": 1}
            effect = None
            self_body, self_limits = strip_limits(inline_if_self.group(2))
            for pattern, handler in PATTERNS:
                match = pattern.match(self_body) or pattern.match(
                    self_body[:1].upper() + self_body[1:]
                )
                if not match:
                    continue
                effect = handler(match)
                effect["raw"] = line
                effect["trigger"] = trigger
                effect["condition"] = condition
                if self_limits:
                    effect.update(self_limits)
                break
            if effect is not None:
                buckets.setdefault(trigger if trigger != "coin" else "coin", []).append(effect)
                continue
            unmodeled.append(line)
            continue
        inline_if = INLINE_IF_RE.match(rest)
        if inline_if:
            condition = {"source": "target", "status": inline_if.group(1), "gte": 1}
            rest = inline_if.group(2)
            effect = None
            for pattern, handler in PATTERNS:
                match = pattern.match(rest)
                if not match:
                    continue
                effect = handler(match)
                effect["raw"] = line
                effect["trigger"] = trigger
                effect["condition"] = condition
                break
            if effect is not None:
                buckets.setdefault(trigger if trigger != "coin" else "coin", []).append(effect)
                continue
            unmodeled.append(line)
            continue
        for tag_re, tag_of in TAG_PATTERNS:
            tag_match = tag_re.match(rest)
            if tag_match:
                buckets.setdefault("tags", []).append(
                    {"kind": "tag", "tag": tag_of(tag_match), "raw": line}
                )
                break
        else:
            tag_match = None
        if tag_match:
            continue
        lower = rest.lower().rstrip(".")
        if lower in TAG_LINES:
            buckets.setdefault("tags", []).append({"kind": "tag", "tag": TAG_LINES[lower], "raw": line})
            continue
        if trigger == "on_use" and not TRIGGER_RE.match(line):
            rest = line
        effect = parse_triggered(trigger, rest, line)
        if effect is not None and pending_resonance:
            if effect.get("kind") == "damage_percent":
                effect = {
                    "kind": "damage_percent_per_resonance",
                    "step": effect.get("value", 0),
                    "max": effect.get("max", 0) or 0,
                    "per": pending_resonance,
                }
                effect["raw"] = line
                effect["trigger"] = trigger
            elif effect.get("kind") == "attack_weight":
                effect["per"] = pending_resonance
                effect["step"] = effect.pop("value", 1)
                effect["from_resonance"] = True
        if effect is None:
            # try splitting "X and Y" compounds
            parts = re.split(r"\s+and\s+", rest)
            parsed_any = False
            if len(parts) > 1:
                for part in parts:
                    sub = parse_triggered(trigger, part.strip(), line)
                    if sub is not None:
                        parsed_any = True
                        bucket = trigger if trigger != "coin" else "coin"
                        effect2 = expand_coin_effects(sub, coin_count) if bucket == "coin" else sub
                        buckets.setdefault(bucket, []).append(effect2)
            if not parsed_any:
                unmodeled.append(line)
            continue
        effect = expand_coin_effects(effect, coin_count)
        bucket = trigger if trigger != "coin" else "coin"
        buckets.setdefault(bucket, []).append(effect)
    return buckets, unmodeled


def build_skill_entry(skill: dict, tier: dict, enemies: bool) -> dict:
    coin_count = tier.get("coins") or 1
    entry: Dict[str, object] = {
        "note": skill.get("name_en") or skill.get("name") or skill.get("id"),
        "on_use": [],
        "clash_win": [],
        "clash_lose": [],
        "attack_end": [],
        "combat_start": [],
        "before_attack": [],
        "on_kill": [],
        "on_evade": [],
        "turn_start": [],
        "turn_end": [],
        "heads_hit": {},
        "coins": {},
        "tags": [],
        "unmodeled": [],
    }
    buckets, unmodeled = parse_text_block(tier.get("on_use_text") or "", coin_count)
    for key in (
        "on_use",
        "clash_win",
        "clash_lose",
        "attack_end",
        "combat_start",
        "before_attack",
        "on_kill",
        "on_evade",
        "turn_start",
        "turn_end",
    ):
        entry[key] = [strip_trigger(e) for e in buckets.get(key, [])]
    entry["tags"] = [e["tag"] for e in buckets.get("tags", [])]
    for index, coin_text in enumerate(tier.get("coin_texts") or [], start=1):
        coin_buckets, coin_unmodeled = parse_text_block(coin_text, coin_count)
        merged: List[dict] = []
        for key in ("on_use", "coin", "clash_win", "clash_lose", "attack_end", "combat_start"):
            merged.extend(coin_buckets.get(key, []))
        heads = coin_buckets.get("heads_hit", [])
        if heads:
            entry["heads_hit"][str(index)] = [strip_trigger(e) for e in heads]
        for extra in coin_buckets.get("clash_win", []):
            entry["clash_win"].append(strip_trigger(extra))
        for extra in coin_buckets.get("clash_lose", []):
            entry["clash_lose"].append(strip_trigger(extra))
        for extra in coin_buckets.get("attack_end", []):
            entry["attack_end"].append(strip_trigger(extra))
        own = [e for e in merged if e.get("trigger") in (None, "coin", "on_use")]
        for effect in own:
            # The Coin an effect belongs to, so "Reuse this Coin" knows which.
            effect["coin_index"] = index
        if own:
            entry["coins"][str(index)] = [strip_trigger(e) for e in own]
        unmodeled.extend(coin_unmodeled)
    # Tag-only clauses ("Target cannot be Staggered ...", "[Discard] ...") become
    # Skill tags wherever they were written.
    tags = set(entry["tags"])
    for key in (
        "on_use",
        "clash_win",
        "clash_lose",
        "attack_end",
        "combat_start",
        "before_attack",
        "on_kill",
        "on_evade",
        "turn_start",
        "turn_end",
    ):
        kept = []
        for effect in entry[key]:
            if effect.get("kind") == "tag" and effect.get("tag"):
                tags.add(effect["tag"])
                continue
            kept.append(effect)
        entry[key] = kept
    for coin_key, effects in list(entry["coins"].items()):
        kept = []
        for effect in effects:
            if effect.get("kind") == "tag" and effect.get("tag"):
                tags.add(effect["tag"])
                continue
            kept.append(effect)
        entry["coins"][coin_key] = kept
    for coin_key, effects in list(entry["heads_hit"].items()):
        kept = []
        for effect in effects:
            if effect.get("kind") == "tag" and effect.get("tag"):
                tags.add(effect["tag"])
                continue
            kept.append(effect)
        entry["heads_hit"][coin_key] = kept
    entry["tags"] = sorted(tags)
    entry["unmodeled"] = sorted(set(unmodeled))
    return entry


def strip_trigger(effect: dict) -> dict:
    effect = dict(effect)
    effect.pop("trigger", None)
    return effect


def main() -> int:
    out: Dict[str, dict] = {"version": 1, "skills": {}}
    total_lines = 0
    unmodeled_lines = 0
    for group in ("identities", "enemies", "ego"):
        directory = os.path.join(DATA, group)
        if not os.path.isdir(directory):
            continue
        for name in sorted(os.listdir(directory)):
            if not name.endswith(".json"):
                continue
            record = json.load(open(os.path.join(directory, name), encoding="utf-8"))
            if group == "ego":
                # E.G.O skills: one entry per skill type.
                for kind_key in ("awakening", "corrosion"):
                    skill = record.get(kind_key)
                    if not skill:
                        continue
                    skill = dict(skill)
                    skill["id"] = f"{record['id']}.{kind_key}"
                    skill["upties"] = {"1": skill}
                    entry = build_skill_entry(skill, skill, True)
                    total_lines += len(entry.get("unmodeled", [])) + sum(
                        len(v) for k, v in entry.items() if isinstance(v, list) and k != "unmodeled"
                    )
                    unmodeled_lines += len(entry.get("unmodeled", []))
                    out["skills"][f"{record['id']}.{kind_key}@1"] = entry
                continue
            skills = record.get("skills", [])
            for skill in skills:
                tiers = skill.get("upties") or {}
                if not tiers:
                    tiers = {str(t): skill for t in (1, 2, 3, 4)}
                for tier_key, tier in tiers.items():
                    if group == "enemies":
                        tier_key = "1"
                    entry = build_skill_entry(skill, tier, group == "enemies")
                    total_lines += len(entry.get("unmodeled", [])) + sum(
                        len(v) for k, v in entry.items() if isinstance(v, list) and k != "unmodeled"
                    )
                    unmodeled_lines += len(entry.get("unmodeled", []))
                    out["skills"][f"{skill['id']}@{tier_key}"] = entry
    os.makedirs(os.path.join(DATA, "mechanics"), exist_ok=True)
    path = os.path.join(DATA, "mechanics", "effects.json")
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(out, fh, ensure_ascii=False, indent=1, sort_keys=True)
    print(f"skills: {len(out['skills'])}")
    print(f"effects: {total_lines - unmodeled_lines}")
    print(f"unmodeled lines: {unmodeled_lines}")
    print(f"wrote {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
