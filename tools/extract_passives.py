"""Extract identity / enemy Passives into data/passives/passives.json.

Passives are the other half of a unit's behaviour: they grant statuses at
Combat Start and Turn Start, keep up continuous modifiers and trigger extra
attacks.  Their text is written in its own style ("Turn Start: if this unit
does not have [X], gain Y"), so this tool normalises each line into the
`[Trigger] clause` form the skill parser understands, adds the passive-only
patterns below, and records every clause that is still not modelled.

Sources: in-game `Passives.json` (official ids and text) plus the wiki.gg page
of the identity / enemy that owns them.
"""

from __future__ import annotations

import json
import os
import re
import sys
from typing import Dict, List, Optional

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import extract_effects as E  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DATA = os.path.join(ROOT, "data")
RAW = os.path.join(DATA, "_raw", "gamedata")

N = E.N
ST = E.ST

# Trigger prefixes a passive line can start with.
TRIGGER_PREFIX = {
    "combat start": "combat_start",
    "turn start": "turn_start",
    "turn end": "turn_end",
    "attack end": "attack_end",
    "attack skill end": "attack_end",
    "always active": "passive",
    "on tails hit": "heads_hit",
    "in an encounter": "passive",
}

PASSIVE_PATTERNS = [
    # "Deal +5% damage for every [Protection] on self (max 15%)"
    (re.compile(rf"^Deal \+{N}% damage for every {ST} on self \(max {N}%\)$"),
     lambda m: {"kind": "damage_percent", "value": 0, "step": int(m.group(1)), "per": 1,
                "max": int(m.group(3)),
                "condition": {"source": "self", "status": m.group(2), "component": "count"}}),
    (re.compile(rf"^Deal \+{N}% damage against targets in either Low Morale or Panic states$"),
     lambda m: {"kind": "damage_percent", "value": int(m.group(1))}),
    (re.compile(rf"^Take -\(SP / {N}\)% HP damage from attacks \(max {N}%\)$"),
     lambda m: {"kind": "damage_percent", "value": -int(m.group(2))}),
    (re.compile(rf"^Deal \+\(-SP/{N}\)% damage with Base Skills \(max {N}%\)$"),
     lambda m: {"kind": "damage_percent", "value": int(m.group(2))}),
    # "Turn Start: Gain 1 [LanternGregBigBird]"
    (re.compile(rf"^[Gg]ain {N} {ST}$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1))}),
    (re.compile(rf"^[Gg]ain {N} {ST} for every {N} Gloom Reson\. \(max {N}\)$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "per": int(m.group(3)), "step": int(m.group(1)), "max": int(m.group(4)),
                "multiplier": 1, "from_resonance": True, "resonance_of": "Gloom"}),
    (re.compile(rf"^If this unit does not have {ST}, gain {N} {ST} Potency and {N} {ST} Count$"),
     lambda m: {"kind": "gain", "status": m.group(4), "potency": int(m.group(3)),
                "count": int(m.group(5)), "status2": m.group(6),
                "condition": {"source": "self", "lacks_status": [m.group(1)]}}),
    (re.compile(rf"^If an enemy has {ST}, heal {N} SP$"),
     lambda m: {"kind": "sp_heal", "value": int(m.group(2)),
                "condition": {"source": "target", "status": m.group(1), "gte": 1}}),
    (re.compile(rf"^At {N}\+ {ST}, gain {N} {ST}$"),
     lambda m: {"kind": "gain", "status": m.group(3), "potency": int(m.group(2)),
                "condition": {"source": "self", "status": m.group(2), "gte": int(m.group(1))}}),
    (re.compile(rf"^At {N} {ST}, gain {N} {ST} and {N} {ST} instead$"),
     lambda m: {"kind": "compound", "condition": {"source": "self", "status": m.group(2),
                                                  "gte": int(m.group(1))},
                "sub_effects": [
                    {"kind": "gain", "status": m.group(4), "potency": int(m.group(3))},
                    {"kind": "gain", "status": m.group(6), "potency": int(m.group(5))}]}),
    (re.compile(rf"^inflict {N} {ST} On Hit with a Base Attack Skill$"),
     lambda m: {"kind": "inflict", "status": m.group(2), "potency": int(m.group(1)),
                "on_base_attack_hit": True}),
    # A passive that only widens the caps of another effect: recorded, no value.
    (re.compile(rf"^{ST} Stack gained from this unit's Base Skills and {ST} effect is capped at {N} per turn$"),
     lambda m: {"kind": "noop", "note": "cap on Protection gains"}),
    (re.compile(r"^Begin [Ee]ncounters with .+$"),
     lambda m: {"kind": "noop", "note": "encounter start resource"}),
    (re.compile(r"^Uses (Plus|Minus) Coin Skills as Base Skills$"),
     lambda m: {"kind": "noop", "note": "coin type switch"}),
    (re.compile(r"^External effects cannot change this unit's Min\. and Max\. Speeds$"),
     lambda m: {"kind": "noop", "note": "speed immunity"}),
]


def normalise(desc: str) -> List[str]:
    """Turn a passive description into `[Trigger] clause` lines."""
    lines: List[str] = []
    for raw_line in desc.split("\n"):
        line = raw_line.strip()
        if not line:
            continue
        if line.startswith("<style") or line.startswith("</style"):
            continue
        line = re.sub(r"</?style[^>]*>", "", line).strip()
        line = re.sub(r"<style=\"highlight\">", "", line)
        line = re.sub(r"\s+", " ", line)
        if not line:
            continue
        match = re.match(r"^([A-Za-z][A-Za-z ']{2,30}):\s*(.+)$", line)
        if match and match.group(1).strip().lower() in TRIGGER_PREFIX:
            line = f"[{match.group(1).strip()}] {match.group(2).strip()}"
        # Bullet points under a heading inherit the heading's trigger later; keep
        # them as plain clauses for now.
        lines.append(line.lstrip("-•· ").strip())
    return lines


def parse_passive(desc: str) -> Dict[str, object]:
    """Parse a passive: phase -> effects, plus the clauses that are unmodelled."""
    lines = normalise(desc)
    text = "\n".join(lines)
    saved = E.PATTERNS
    E.PATTERNS = PASSIVE_PATTERNS + saved
    try:
        buckets, unmodeled = E.parse_text_block(text, 1)
    finally:
        E.PATTERNS = saved
    out: Dict[str, object] = {
        "combat_start": [],
        "turn_start": [],
        "turn_end": [],
        "attack_end": [],
        "passive": [],
        "unmodeled": sorted(set(unmodeled)),
    }
    for key in ("combat_start", "turn_start", "turn_end", "attack_end", "passive"):
        out[key] = [E.strip_trigger(e) for e in buckets.get(key, [])]
    return out


def load(path: str) -> dict:
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


# Passives whose behaviour lives in engine code (documented by a test) rather
# than in the generic effect list.
HAND_MODELLED = {
    "956704": "battle::resolve_clash (Clash Count swing, scripts::ClashCountSwing)",
    "956705": "battle::update_time_state + scripts::TimeState (state of time)",
    "957201": "battle::apply_hit (Segmentation: Stack loss per hit, attacker SP heal)",
    "957202": "battle::apply_hit (Origination: damage transfer, HP floor)",
    "957301": "battle::apply_hit (Segmentation)",
    "957302": "battle::apply_hit (Origination)",
    "957401": "battle::apply_hit (Segmentation)",
    "957402": "battle::apply_hit (Origination)",
}


# Clauses modelled by hand from the passive's own (official) text.  Each entry
# records the exact clause it covers, so the tool can subtract it from the
# unmodelled list instead of quietly dropping it.
HAND_EFFECTS: Dict[str, dict] = {
    # Outis - Vanguard Team
    "1111401": {
        "covered": [
            "Deal +5% damage for every [Protection] on self (max 15%)",
            "Combat Start: gain 2 [TheUdjatOutis]",
        ],
        "passive": [
            {"kind": "damage_percent", "value": 0, "step": 5, "per": 1, "max": 15,
             "condition": {"source": "self", "status": "Protection", "component": "count"}},
        ],
        "combat_start": [
            {"kind": "gain", "status": "The Udjat -Vanguard-", "potency": 2, "component": "stack"},
        ],
    },
    # Outis - The Scepter of Horus - Replica (Uptie 4 variant)
    "1111412": {
        "covered": [
            "If this unit has [TheUdjatOutis], inflict 1 [SheutFracture] On Hit with a Base Attack Skill (15 per turn)",
        ],
        "passive": [
            {"kind": "inflict", "status": "Sheut Fracture", "potency": 1, "per_turn": 15,
             "on_base_attack_hit": True,
             "condition": {"source": "self", "status": "The Udjat -Vanguard-", "gte": 1}},
        ],
    },
    # Outis - Eagle Eye (support passive)
    "1111421": {
        "covered": ["Combat Start: apply 1 [Protection] next turn to 1 ally with the lowest HP percentage"],
        "combat_start": [
            {"kind": "gain", "status": "Protection", "count": 1, "next_turn": True,
             "ally": "lowest_hp", "ally_count": 1},
        ],
    },
    # Gregor - Eyes Bright Like Lamps (Combat Start)
    "1121401": {
        "covered": [
            "Combat Start: Gain 1 [LanternGregBigBird]",
            "Combat Start: If an enemy has [DelusionGregBigBird], heal 3 SP",
        ],
        "combat_start": [
            {"kind": "gain", "status": "Lamp", "potency": 1, "component": "stack"},
            {"kind": "sp_heal", "value": 3,
             "condition": {"source": "target", "status": "Dazzle", "gte": 1}},
        ],
    },
    "1121411": {
        "covered": [
            "Combat Start: Gain 1 [LanternGregBigBird] for every 2 Gloom Reson. (max 3)",
            "Combat Start: If an enemy has [DelusionGregBigBird], heal 5 SP",
        ],
        "combat_start": [
            {"kind": "gain", "status": "Lamp", "potency": 1, "per": 2, "step": 1, "max": 3,
             "from_resonance": True, "resonance_of": "Gloom", "component": "stack"},
            {"kind": "sp_heal", "value": 5,
             "condition": {"source": "target", "status": "Dazzle", "gte": 1}},
        ],
    },
    # Gregor - Dazzling Lamp
    "1121402": {
        "covered": ["Turn Start: Gain 1 [LanternGregBigBird]"],
        "turn_start": [
            {"kind": "gain", "status": "Lamp", "potency": 1, "component": "stack"},
        ],
    },
    # Jeong's Office Rep - Stacking the Deck
    "1081301": {
        "covered": ["On Tails Hit, heal 5 SP (once per turn)"],
        "tails_hit": [
            {"kind": "sp_heal", "value": 5, "per_turn": 1},
        ],
    },
    # Jeong's Office Rep - Koi-Koi
    "1081302": {
        "covered": [
            "Turn Start: if this unit does not have [HanafudaCombo], gain 0 [HanafudaCombo] Potency and 3 [HanafudaCombo] Count",
            "Turn Start: if this unit does not have a Suit in Hand, gain a random Suit([HanafudaOne], [HanafudaTwo], or [HanafudaThree]) that corresponds to a Base Attack Skill on this unit's leftmost Skill Slot",
        ],
        "turn_start": [
            {"kind": "gain", "status": "Bright -\u5149-", "potency": 0, "count": 3,
             "component": "stack",
             "condition": {"source": "self", "lacks_status": ["Bright -\u5149-"]}},
            {"kind": "suit_convert",
             "condition": {"source": "self", "lacks_status": ["HanafudaOne", "HanafudaTwo", "HanafudaThree"]}},
        ],
    },
    # Rodion - The Sword Sharpened with Tears (Blessing / Despair states)
    "1091302": {
        "covered": [
            "Turn Start: at 0 or higher SP, gain [BlessingAlly]",
            "Turn Start: at less than 0 SP, gain [DespairAlly]",
        ],
        "turn_start": [
            {"kind": "gain", "status": "Blessing", "potency": 1, "component": "stack",
             "condition": {"self_sp_at_least": 0}},
            {"kind": "gain", "status": "Despair", "potency": 1, "component": "stack",
             "condition": {"self_sp_below": 0}},
        ],
    },
    "1091311": {
        "covered": [
            "Take -(SP / 2)% HP damage from attacks (max 20%)",
            "Deal +(-SP/2)% damage with Base Skills (max 20%)",
            "Combat Start: at 3+ [ProtectiveSword], gain 1 [Protection]",
            "Combat Start: at 3+ [PenetratingSword] gain 1 [AttackDmgUp]",
        ],
        "passive": [
            {"kind": "damage_taken_percent", "per": 2, "step": 1, "max": 20,
             "condition": {"source": "self", "status": "Blessing", "gte": 1}},
            {"kind": "damage_percent_from_negative_sp", "per": 2, "step": 1, "max": 20,
             "condition": {"source": "self", "status": "Despair", "gte": 1}},
        ],
        "combat_start": [
            {"kind": "gain", "status": "Protection", "count": 1,
             "condition": {"source": "self", "status": "Protecting Sword", "gte": 3}},
            {"kind": "gain", "status": "Attack Power Up", "count": 1,
             "condition": {"source": "self", "status": "Tear-sharpened", "gte": 3}},
        ],
    },
}


def localisation(group: str) -> Dict[str, dict]:
    """id -> {name, desc} from the cached zh table when it is present."""
    out: Dict[str, dict] = {}
    for name in ("Passives-a1c10p1.json", "Passives.json"):
        path = os.path.join(RAW, name)
        if os.path.exists(path):
            for item in load(path).get("dataList", []):
                out[str(item["id"])] = item
    return out


def main() -> int:
    entries = []
    for name in sorted(os.listdir(RAW)):
        if name.startswith("Passives") and name.endswith(".json"):
            entries.extend(load(os.path.join(RAW, name)).get("dataList", []))
    zh = localisation("zh")
    identities = [
        name[:-5]
        for name in sorted(os.listdir(os.path.join(DATA, "identities")))
        if name.endswith(".json")
    ]
    enemies = [
        name[:-5]
        for name in sorted(os.listdir(os.path.join(DATA, "enemies")))
        if name.endswith(".json")
    ]
    out: Dict[str, dict] = {"version": 1, "passives": {}}
    modelled = 0
    unmodelled = 0
    for item in entries:
        pid = str(item.get("id", ""))
        if len(pid) != 7:
            continue
        owner = pid[:5] if pid[:5] in identities else pid[:4]
        if owner not in identities and owner not in enemies:
            continue
        # Last two digits: kind (0/1 = combat passive, 2/3 = support passive).
        kind_digit = pid[5]
        kind = {
            "0": "combat",
            "1": "combat",
            "2": "support",
            "3": "support",
        }.get(kind_digit, "other")
        parsed = parse_passive(item.get("desc") or "")
        count = sum(
            len(v) for k, v in parsed.items() if isinstance(v, list) and k != "unmodeled"
        )
        modelled += count
        unmodelled += len(parsed["unmodeled"])
        zh_item = zh.get(pid) or {}
        hand = HAND_MODELLED.get(pid)
        if hand:
            parsed["unmodeled"] = []
            parsed["hand_modelled"] = hand
        curated = HAND_EFFECTS.get(pid)
        if curated:
            for clause in curated.get("covered", []):
                parsed["unmodeled"] = [u for u in parsed["unmodeled"] if u != clause]
            for key in ("passive", "combat_start", "turn_start", "turn_end",
                        "attack_end", "tails_hit"):
                if key in curated:
                    parsed[key] = curated[key]
            parsed["hand_modelled"] = "extract_passives.HAND_EFFECTS"
        out["passives"][pid] = {
            "id": pid,
            "hand_modelled": hand,
            "owner": owner,
            "kind": kind,
            "name": item.get("name"),
            "name_zh": zh_item.get("name"),
            "desc": (item.get("desc") or "").strip(),
            "desc_zh": (zh_item.get("desc") or "").strip(),
            "effects": parsed,
            "sources": [
                {"kind": "game_text", "language": "en", "id": pid, "verification": "official"}
            ],
        }
    os.makedirs(os.path.join(DATA, "passives"), exist_ok=True)
    path = os.path.join(DATA, "passives", "passives.json")
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(out, fh, ensure_ascii=False, indent=1, sort_keys=True)
    print(f"passives: {len(out['passives'])}")
    print(f"clauses modelled: {modelled}")
    print(f"clauses unmodelled: {unmodelled}")
    print(f"wrote {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
