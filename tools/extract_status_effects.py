"""Extract status behaviour into data/mechanics/status_effects.json.

A status is not just a number: its own text says what it does ("Turn Start: gain
1 [Defense Level Up] for every Stack (max 5)", "Base Attack Skills deal
+(Stack x 10)% damage (max 30%)").  This tool parses those clauses out of
`data/statuses/statuses.json` (in-game text) and `data/_raw/wiki_status_text.json`
(wiki text for statuses the localisation dump does not carry, e.g. Rodion's
Blessing / Despair / Protecting Sword / Tear-sharpened) into per-phase effects
the engine applies:

  * `turn_start` / `turn_end` - resolved while the unit holds the status,
  * `passive` - continuous modifiers (damage dealt / taken),
  * `unmodeled` - every clause this project does not model yet.

Sources are recorded per status, with the language and the verification level.
"""

from __future__ import annotations

import json
import os
import re
import sys
from typing import Dict, List

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import extract_effects as E  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DATA = os.path.join(ROOT, "data")

N = E.N
ST = E.ST
NUM = r"(\d+(?:\.\d+)?)"  # an integer or decimal percentage

# Statuses whose text is a full rules block worth parsing.  Everything else is
# left in `statuses.json` as documentation.
STATUS_PATTERNS = [
    # Gains scaled by the unit's own stacks / SP.
    (re.compile(rf"^Turn Start: gain {N} {ST} for every Potency$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "per": 1, "step": int(m.group(1)),
                "condition": {"source": "self", "status": "self", "component": "potency"}}),
    (re.compile(rf"^Turn Start: lose {N} Count$"),
     lambda m: {"kind": "lose_status_count", "status": "self", "count": int(m.group(1))}),
    # "Potency: Base 0, Max 5" / "Count: Base 3, Max 3" - the caps of the status.
    (re.compile(rf"^Potency: Base {N}, Max {N}$"),
     lambda m: {"kind": "cap_potency", "value": int(m.group(2))}),
    (re.compile(rf"^Count: Base {N}, Max {N}$"),
     lambda m: {"kind": "cap_count", "value": int(m.group(2))}),
    (re.compile(rf"^Turn Start: gain {N} {ST} for every Stack \(max {N}\)$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "per": 1, "step": int(m.group(1)), "max": int(m.group(3)),
                "condition": {"source": "self", "status": "self", "component": "stack"}}),
    (re.compile(rf"^Turn Start: gain {N} {ST} for every {N} SP \(max {N}\)$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "per_sp": int(m.group(3)), "max": int(m.group(4)),
                "condition": {"source": "self"}}),
    (re.compile(rf"^Turn Start: gain {N} {ST}$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1))}),
    (re.compile(rf"^At {N} Stack, gain additional {N} {ST}$"),
     lambda m: {"kind": "gain", "status": m.group(3), "potency": int(m.group(2)),
                "condition": {"source": "self", "status": "self", "component": "stack",
                              "gte": int(m.group(1))}}),
    (re.compile(rf"^Turn End: lose \(Stack x {N}\) SP$"),
     lambda m: {"kind": "sp_damage_self_per_stack", "status": "self", "step": int(m.group(1))}),
    (re.compile(rf"^Turn End: Gain {N} {ST} next turn for every {N} Stack \(max {N}\)$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "per": int(m.group(3)), "step": int(m.group(1)), "max": int(m.group(4)),
                "next_turn": True,
                "condition": {"source": "self", "status": "self", "component": "stack"}}),
    (re.compile(rf"^Turn End: Lose {N} Stack$"),
     lambda m: {"kind": "gain", "status": "self", "component": "stack", "potency": -int(m.group(1))}),
    # Continuous damage modifiers.
    (re.compile(rf"^Base Attack Skills deal \+\(Stack x {NUM}\)% damage \(max {N}%\)$"),
     lambda m: {"kind": "damage_percent", "value": 0, "step_f": float(m.group(1)), "per": 1,
                "max": int(m.group(2)),
                "condition": {"source": "self", "status": "self", "component": "stack"}}),
    (re.compile(rf"^Deal \+{N}% damage for every Stack \(max {N}%\)$"),
     lambda m: {"kind": "damage_percent", "value": 0, "step": int(m.group(1)), "per": 1,
                "max": int(m.group(2)),
                "condition": {"source": "self", "status": "self", "component": "stack"}}),
    (re.compile(rf"^For every Stack, take \+{NUM}% more damage from Sloth and Gloom Skills$"),
     lambda m: {"kind": "damage_taken_percent", "value": 0, "step_f": float(m.group(1)), "per": 1,
                "condition": {"source": "self", "status": "self", "component": "stack"}}),
    (re.compile(rf"^For every {N} \({ST} \+ {ST}\), take \+{NUM}% damage from Base Attack Skills \(max {N}%\)$"),
     lambda m: {"kind": "damage_taken_percent", "value": 0, "step_f": float(m.group(4)), "per": 1,
                "max": int(m.group(5)),
                "condition": {"source": "self", "statuses": [m.group(2), m.group(3)],
                              "component": "potency"}}),
    (re.compile(rf"^Take \+{NUM}% damage from Base Attack Skills \(max {N}%\)$"),
     lambda m: {"kind": "damage_taken_percent", "value": 0, "step_f": float(m.group(1)), "per": 1,
                "max": int(m.group(2)),
                "condition": {"source": "self", "status": "self", "component": "stack"}}),
    (re.compile(rf"^Take \+{NUM}% damage \(max {N}%\)$"),
     lambda m: {"kind": "damage_taken_percent", "value": 0, "step_f": float(m.group(1)), "per": 1,
                "max": int(m.group(2)),
                "condition": {"source": "self", "status": "self", "component": "stack"}}),
    (re.compile(rf"^Offense Level \+{N} for every Stack \(max {N}\)$"),
     lambda m: {"kind": "offense_level_per_stack", "step": int(m.group(1)), "max": int(m.group(2))}),
    # Aggro / Defense Level upkeep.
    (re.compile(rf"^Turn Start: \+\(Stack x {N}\) {ST} to the leftmost Skill Slot on the Dashboard \(max {N}\)$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "per": 1, "step": int(m.group(1)), "max": int(m.group(3)),
                "component": "stack",
                "condition": {"source": "self", "status": "self", "component": "stack"}}),
    (re.compile(r"^Expires at Turn End$"),
     lambda m: {"kind": "remove_at_turn_end"}),
    (re.compile(rf"^Max Stack: {N}$"),
     lambda m: {"kind": "max_stack", "value": int(m.group(1))}),
    (re.compile(r"^Turn End: Lose 1 Stack$"),
     lambda m: {"kind": "gain", "status": "self", "potency": -1, "component": "stack"}),
    (re.compile(r"^Reduced by 1 at Turn End$"),
     lambda m: {"kind": "gain", "status": "self", "potency": -1, "component": "stack"}),
    (re.compile(r"^Turn End: reduce the Count by 1$", re.I),
     lambda m: {"kind": "gain", "status": "self", "count": -1}),
    (re.compile(rf"^Turn End: Lose {N} Stack$"),
     lambda m: {"kind": "gain", "status": "self", "potency": -int(m.group(1)),
                "component": "stack"}),
    (re.compile(rf"^Turn End: Reduce {N} Stack$"),
     lambda m: {"kind": "gain", "status": "self", "potency": -int(m.group(1)),
                "component": "stack"}),
    (re.compile(rf"^Turn Start: Gain {N} {ST}$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1))}),
    (re.compile(rf"^Turn Start: {N} {ST}$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1))}),
    (re.compile(rf"^Gain {N} {ST} at Turn Start$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1))}),
    (re.compile(rf"^Turn End: Gain {N} {ST}$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1))}),
    (re.compile(r"^Unique \[Ammo\]$"),
     lambda m: {"kind": "noop", "note": "ammo status"}),
    # Prefix-free forms: the trigger was already stripped, so the phase decides
    # when these resolve.
    (re.compile(rf"^inflict {N} {ST}$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1))}),
    (re.compile(rf"^inflict \+{N} {ST} Count$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "count": int(m.group(1))}),
    (re.compile(rf"^inflict {N} {ST} on the attacker$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1)),
                "on_attacker": True}),
    (re.compile(rf"^inflict {N} {ST} against the attacker$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1)),
                "on_attacker": True}),
    (re.compile(rf"^inflict {N} {ST} on the attacker(?: and lose {N} Stack)?$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1)),
                "on_attacker": True}),
    # Clash-end / on-hit riders.
    (re.compile(rf"^When Clash ends, inflict {N} {ST}$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1))}),
    (re.compile(rf"^When Clash ends, inflict \+{N} {ST} Count$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "count": int(m.group(1))}),
    (re.compile(rf"^When hit, inflict {N} {ST} on the attacker$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1)),
                "on_attacker": True}),
    (re.compile(rf"^When hit, inflict {N} {ST} against the attacker$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1)),
                "on_attacker": True}),
    (re.compile(rf"^On Hit with the first Coin of a Base Attack Skill, inflict \+{N} {ST} Count$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "count": int(m.group(1)),
                "first_coin_only": True}),
    (re.compile(rf"^On Hit, inflict {N} {ST}$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1))}),
    (re.compile(rf"^When hit, inflict {N} {ST}$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1)),
                "on_attacker": True}),
    (re.compile(rf"^When hit, .+$"),
     lambda m: {"kind": "noop", "note": "on-hit rider"}),
    (re.compile(r"^On Hit with .+$"),
     lambda m: {"kind": "noop", "note": "on-hit rider"}),
]


def split_clauses(text: str) -> List[str]:
    text = re.sub(r"&nbsp;", " ", text)
    text = re.sub(r"<[^>]*>", "", text)
    lines: List[str] = []
    for raw in re.split(r"\n|•", text):
        line = raw.strip().lstrip("-· ").strip()
        if not line:
            continue
        line = re.sub(r"\s+", " ", line)
        lines.append(line)
    return lines


def parse_status(text: str) -> Dict[str, object]:
    out: Dict[str, object] = {
        "turn_start": [],
        "turn_end": [],
        "clash_end": [],
        "on_hit": [],
        "passive": [],
        "unmodeled": [],
    }
    for clause in split_clauses(text):
        trigger = None
        body = clause
        where = None
        match = re.match(
            r"^(Turn Start|Turn End|On Hit|When hit|When Hit|When Clash ends|Always Active):?\s*(.+)$",
            clause,
        )
        if match and match.group(1) in (
            "Turn Start",
            "Turn End",
            "On Hit",
            "When hit",
            "When Hit",
            "When Clash ends",
            "Always Active",
        ):
            where = match.group(1)
            body = match.group(2).strip().lstrip(",: ").strip()
            trigger = {
                "Turn Start": "turn_start",
                "Turn End": "turn_end",
                "On Hit": "on_hit",
                "When hit": "on_hit",
                "When Hit": "on_hit",
                "When Clash ends": "clash_end",
            }.get(where)
        # Everything else is a continuous clause unless a pattern says otherwise.
        limits = None
        body, limits = E.strip_limits(body)
        effect = None
        candidates = [body, clause]
        if where:
            candidates.append(f"{where}: {body}")
        for candidate in candidates:
            for pattern, handler in STATUS_PATTERNS:
                found = pattern.match(candidate)
                if found:
                    effect = handler(found)
                    break
            if effect is not None:
                break
        if effect is None:
            # Continuous damage / level clauses have no trigger.
            out["unmodeled"].append(clause)
            continue
        if effect.get("kind") == "noop":
            out["unmodeled"].append(clause)
            continue
        if effect.get("kind") == "max_stack":
            out["max_stack"] = int(effect.get("value") or 0)
            continue
        if effect.get("kind") == "cap_potency":
            out["max_potency"] = int(effect.get("value") or 0)
            continue
        if effect.get("kind") == "cap_count":
            out["max_count"] = int(effect.get("value") or 0)
            continue
        effect["raw"] = clause
        if limits:
            effect.update(limits)
        if trigger:
            out[trigger].append(effect)
        else:
            out["passive"].append(effect)
    out["unmodeled"] = sorted(set(out["unmodeled"]))
    return out


FIXED = ("10110", "10414", "10813", "10913", "11004", "11114", "11214", "9567")


def used_statuses() -> set:
    """Status names the fixed content's Skills / E.G.O reference."""
    path = os.path.join(DATA, "mechanics", "effects.json")
    if not os.path.exists(path):
        return set()
    book = load(path)["skills"]
    names: set = set()

    def walk(effect: dict) -> None:
        for key in ("status", "status2", "heal_from_status"):
            value = effect.get(key)
            if value and value != "self":
                names.add(value)
        condition = effect.get("condition") or {}
        if condition.get("status"):
            names.add(condition["status"])
        for entry in condition.get("statuses") or []:
            names.add(entry)
        for sub in effect.get("sub_effects") or []:
            walk(sub)

    identities = tuple(x for x in FIXED if len(x) == 5)
    enemies = tuple(x for x in FIXED if len(x) == 4)
    for key, entry in book.items():
        skill_id = key.split("@")[0]
        if not (skill_id.startswith(identities) or skill_id[:4] in enemies):
            continue
        for phase in (
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
            "passive",
        ):
            for effect in entry.get(phase) or []:
                walk(effect)
        for group in ("coins", "heads_hit"):
            for effects in (entry.get(group) or {}).values():
                for effect in effects:
                    walk(effect)
    return names


# Clauses a status page restates that the Skill granting the status already
# carries.  Keeping both would apply them twice: Rodion's The Knight's Faith has
# "[Turn End] At less than 3 [Tear-sharpened], lose 15 SP to gain 1
# [Tear-sharpened] / - If this unit has [Tear-sharpened], lose ([Tear-sharpened]
# Stack x 15) more SP" as its own Turn End upkeep, which is exactly what the
# status page lists again.
STATUS_CLAUSES_OWNED_BY_A_SKILL = {
    "Tear-sharpened": {"sp_damage_self_per_stack"},
}


# Statuses whose own text defines their lifecycle, so the general "a value
# reached 0, remove it" rule must not apply.  `Bright -光-` (HanafudaCombo) is
# read at "0 Count" by Koi-Koi's Kozan trigger, so its Count running out is a
# state the Skill has to see, not a deletion.
NO_VALUE_EXPIRY = {"Bright -光-"}


def infer_structure(text: str, primary: str) -> str:
    """Which values a status carries, for statuses the localisation dump lacks.

    A wiki-only status such as Poise mentions both halves ("Critical hit chance
    +Potency%; critical hits consume 1 Count"), so it is a Potency/Count status
    and is removed once either reaches 0 (wiki.gg `Status Effects`).
    """
    lowered = text.lower()
    if primary == "stack" or "stack" in lowered:
        return "stack"
    if "count" in lowered and "potency" in lowered:
        return "potency_count"
    return "single"


def load(path: str):
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def main() -> int:
    statuses = load(os.path.join(DATA, "statuses", "statuses.json"))
    primary: Dict[str, str] = {}
    expires: Dict[str, bool] = {}
    expiry: Dict[str, str] = {}
    structure: Dict[str, str] = {}
    for record in statuses:
        name = record.get("name_en") or record.get("wiki_name")
        if name:
            primary.setdefault(name, record.get("primary") or "potency")
            expiry.setdefault(name, record.get("expiry") or "either_zero")
            value = record.get("structure") or "single"
            if (record.get("primary") or "") == "stack":
                # The status' own text measures it by Stack ("0.1 per Stack"),
                # so it carries a single Stack value even when the table calls it
                # Potency/Count.
                value = "stack"
            structure.setdefault(name, value)
            if record.get("expires_at_turn_end"):
                expires[name] = True
    wiki = load(os.path.join(DATA, "_raw", "wiki_status_text.json"))
    used = used_statuses()
    out: Dict[str, dict] = {"version": 1, "statuses": {}, "used_by_fixed_content": sorted(used)}
    modelled = 0
    unmodelled = 0
    entries = []
    for record in statuses:
        name = record.get("name_en") or record.get("wiki_name")
        text = record.get("text_en") or ""
        if name and name in wiki and len(wiki[name]) > len(text):
            text = wiki[name]
            source_kind = "wiki"
        else:
            source_kind = "game_text"
        if text:
            entries.append((name or record["key"], record["key"], text, source_kind))
    known = {entry[0] for entry in entries}
    # Statuses the wiki documents but the localisation dump does not carry
    # (Rodion's Blessing / Despair / Protecting Sword / Tear-sharpened, ...).
    for name in sorted(wiki):
        if name in known or not name:
            continue
        entries.append((name, f"wiki:{name}", wiki[name], "wiki"))
    for name, key, text, source_kind in entries:
        parsed = parse_status(text)
        owned = STATUS_CLAUSES_OWNED_BY_A_SKILL.get(name)
        if owned:
            for phase, value in parsed.items():
                if not isinstance(value, list):
                    continue
                parsed[phase] = [
                    effect
                    for effect in value
                    if not isinstance(effect, dict) or effect.get("kind") not in owned
                ]
        modelled += sum(
            len(v) for k, v in parsed.items() if isinstance(v, list) and k != "unmodeled"
        )
        unmodelled += len(parsed["unmodeled"])
        out["statuses"][name] = {
            "key": key,
            "used_by_fixed_content": name in used,
            "primary": primary.get(name, "potency"),
            "max_potency": parsed.get("max_potency"),
            "max_count": parsed.get("max_count"),
            # `single` statuses (Charge-like resources, "Potency Fixed") have no
            # second value, so "either reaches 0" must not fire on the value they
            # never use.
            "structure": structure.get(name) or infer_structure(text, primary.get(name, "potency")),
            "expiry": "none" if name in NO_VALUE_EXPIRY else expiry.get(name, "either_zero"),
            "expires_at_turn_end": bool(expires.get(name, False)),
            "name": name,
            "text": text,
            "effects": parsed,
            "sources": [
                {
                    "kind": source_kind,
                    "language": "en",
                    "title": name,
                    "verification": "official" if source_kind == "game_text" else "wiki_verified",
                }
            ],
        }
    path = os.path.join(DATA, "mechanics", "status_effects.json")
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(out, fh, ensure_ascii=False, indent=1, sort_keys=True)
    print(f"statuses: {len(out['statuses'])}")
    print(f"clauses modelled: {modelled}, unmodelled: {unmodelled}")
    print(f"statuses used by the fixed content: {len(used)}")
    print(f"wrote {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
