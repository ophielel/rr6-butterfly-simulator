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
    "before attack": "on_use",
    "on hit": "coin",
    "on succeed attack": "coin",
    "onhit": "coin",
    "on hit without cracking": "coin",
    "heads hit": "coin",
    "hit after clash lose": "coin",
    "on kill": "on_kill",
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
                "condition": {"source": "target" if m.group(3) in ("target", "the main target") else "self",
                              "status": m.group(3 + 1), "component": "potency"}}),
    (re.compile(rf"^Final Power \+{N} for every {N} {ST} on (self|target|the main target) \(max {N}[^)]*\)$"),
     lambda m: {"kind": "base_power", "value": 0, "step": int(m.group(1)), "per": int(m.group(2)),
                "max": int(m.group(5)),
                "condition": {"source": "target" if m.group(3) in ("target", "the main target") else "self",
                              "status": m.group(4), "component": "potency"}}),
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
                "condition": {"source": m.group(3), "status": m.group(4), "component": "potency"}}),
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
    (re.compile(r"^This Attack Skill deals 0 damage$"),
     lambda m: {"kind": "zero_damage"}),
    (re.compile(r"^Does not take damage for this turn$"),
     lambda m: {"kind": "no_damage_taken"}),
    # Trigger [Tremor Burst]; then, reduce target's [Tremor] Count by 1
    (re.compile(r"^Trigger \[([^\]]+)\]; then, reduce target's \[([^\]]+)\] Count by (\d+)$"),
     lambda m: {"kind": "tremor_burst" if m.group(1) == "Tremor Burst" else "activate_status",
                "status": m.group(1), "consume_count": int(m.group(3))}),
    (re.compile(r"^Activate \[([^\]]+)\] on target (once|twice|\d+ times?)\. Target loses (\d+) \[([^\]]+)\] Count$"),
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
]


def parse_triggered(trigger: str, text: str, raw: str) -> Optional[dict]:
    """Turn one triggered clause into an effect dict (or None).

    The wiki writes "the sum of X and both [Y]" when both components of a
    Potency/Count status are summed; that is normalised to a status list where
    the status appears twice (summing Potency + Count).
    """
    text = text.strip()
    # Trailing per-turn / per-encounter limits: "(2 times per turn)".
    limits = None
    limit_match = re.search(r"\((once|twice|\d+ times?) per (turn|Encounter)\)", text)
    if limit_match:
        word = limit_match.group(1)
        count = {"once": 1, "twice": 2}.get(word, None)
        if count is None:
            count = int(re.sub(r"\D", "", word) or 1)
        limits = {
            "per_turn" if limit_match.group(2) == "turn" else "per_encounter": count
        }
        text = (text[: limit_match.start()] + text[limit_match.end():]).strip()
        # Tidy a dangling separator left inside a parenthetical.
        text = re.sub(r"\(max (\d+)%;\s*\)", r"(max \1%)", text)
        text = re.sub(r"\(max (\d+)%;?\)", r"(max \1%)", text)
    # "next turn" buffs are applied at the start of the following turn.
    next_turn = False
    if text.endswith(" next turn"):
        next_turn = True
        text = text[: -len(" next turn")].strip()
    normalized = text.replace("both [", "[")
    for pattern, handler in PATTERNS:
        match = pattern.match(normalized)
        if not match:
            continue
        effect = handler(match)
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
        if limits:
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
INLINE_IF_SP_RE = re.compile(r"^If target's SP is below (\d+), (.+)$")


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
    buckets: Dict[str, List[dict]] = {"on_use": [], "clash_win": [], "clash_lose": [], "attack_end": [], "combat_start": [], "coin": []}
    unmodeled: List[str] = []
    pending_trigger = "on_use"
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
        inline_if_sp = INLINE_IF_SP_RE.match(rest)
        if inline_if_sp:
            condition = {"target_sp_below": int(inline_if_sp.group(1))}
            effect = None
            for pattern, handler in PATTERNS:
                match = pattern.match(inline_if_sp.group(2))
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
        inline_if_self = INLINE_IF_SELF_RE.match(rest)
        if inline_if_self:
            condition = {"source": "self", "status": inline_if_self.group(1), "gte": 1}
            effect = None
            for pattern, handler in PATTERNS:
                match = pattern.match(inline_if_self.group(2))
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
        lower = rest.lower().rstrip(".")
        if lower in TAG_LINES:
            buckets.setdefault("tags", []).append({"kind": "tag", "tag": TAG_LINES[lower], "raw": line})
            continue
        if trigger == "on_use" and not TRIGGER_RE.match(line):
            rest = line
        effect = parse_triggered(trigger, rest, line)
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
        "coins": {},
        "tags": [],
        "unmodeled": [],
    }
    buckets, unmodeled = parse_text_block(tier.get("on_use_text") or "", coin_count)
    for key in ("on_use", "clash_win", "clash_lose", "attack_end", "combat_start"):
        entry[key] = [strip_trigger(e) for e in buckets.get(key, [])]
    entry["tags"] = [e["tag"] for e in buckets.get("tags", [])]
    for index, coin_text in enumerate(tier.get("coin_texts") or [], start=1):
        coin_buckets, coin_unmodeled = parse_text_block(coin_text, coin_count)
        merged: List[dict] = []
        for key in ("on_use", "coin", "clash_win", "clash_lose", "attack_end", "combat_start"):
            merged.extend(coin_buckets.get(key, []))
        for extra in coin_buckets.get("clash_win", []):
            entry["clash_win"].append(strip_trigger(extra))
        for extra in coin_buckets.get("clash_lose", []):
            entry["clash_lose"].append(strip_trigger(extra))
        for extra in coin_buckets.get("attack_end", []):
            entry["attack_end"].append(strip_trigger(extra))
        own = [e for e in merged if e.get("trigger") in (None, "coin", "on_use")]
        if own:
            entry["coins"][str(index)] = [strip_trigger(e) for e in own]
        unmodeled.extend(coin_unmodeled)
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
