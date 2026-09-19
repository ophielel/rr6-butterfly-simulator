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
    (re.compile(rf"^Coin Power \+{N} for every {N} value of the sum of the target's {ST} and both {ST} \(max {N}\)$"),
     lambda m: {"kind": "coin_power", "value": int(m.group(1)), "per": int(m.group(2)),
                "max": int(m.group(5)),
                "condition": {"source": "target",
                              "statuses": [m.group(3), m.group(4), m.group(4)],
                              "component": "potency"}}),
    (re.compile(rf"^If the sum of the target's {ST} and both {ST} is {N} or higher, Coin Power \+{N}$"),
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
    (re.compile(r"^Activate \[([^\]]+)\] on target (once|twice|\d+ times?)\. Target loses (\d+) \[([^\]]+)\] Count$"),
     lambda m: {"kind": "activate_status", "status": m.group(1),
                "times": {"once": 1, "twice": 2}.get(m.group(2), None) or int(re.sub(r"\D", "", m.group(2)) or 1),
                "consume_count": int(m.group(3))}),
    (re.compile(rf"^Gain Shield equal to \(the sum of both {ST} on the selected target\)% HP \(max {N}% per turn\)$"),
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
    normalized = text.replace("both [", "[")
    for pattern, handler in PATTERNS:
        match = pattern.match(normalized)
        if not match:
            continue
        effect = handler(match)
        if "both [" in text:
            for key in ("condition",):
                cond = effect.get(key)
                if not isinstance(cond, dict):
                    continue
                if cond.get("statuses"):
                    statuses = list(cond["statuses"])
                    statuses = statuses[:-1] + [statuses[-1], statuses[-1]]
                    cond["statuses"] = statuses
                elif cond.get("status"):
                    status = cond.pop("status")
                    cond["statuses"] = [status, status]
        effect["raw"] = raw
        effect["trigger"] = trigger
        return effect
    return None


TAG_LINES = {
    "can clash with this skill regardless of speed": "clash_any_speed",
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
    for raw_line in text.split("\n"):
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
        lower = line.lower().rstrip(".")
        if lower in TAG_LINES:
            buckets.setdefault("tags", []).append({"kind": "tag", "tag": TAG_LINES[lower], "raw": line})
            continue
        trigger, rest = split_trigger(line)
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
