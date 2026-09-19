"""Extract Panic Types into data/mechanics/panic_types.json.

Source: the wiki.gg `Sanity` page ("List of Panic Types"), which tabulates each
type's Low Morale and Panic effects, plus the in-game `BattleKeywords*.json`
entries that describe the same rows (LowMorale / Panic families).  The effects
are parsed with the same patterns the skills use, so anything that cannot be
translated is recorded instead of guessed.

Sanity rules for reference (wiki.gg `Sanity`, `Clash`):
  * SP lives in [-45, 45]; Heads chance = 50 + SP.
  * SP <= -30: Low Morale (chance-based on the wiki, chance not documented).
  * SP == -45: Panic; Sinners are fixed at -45 until the next Turn Start and
    then either Corrode (E.G.O Corrosion) or Panic; after that turn their SP
    resets to 0.
  * Low Morale / Panic effects only apply on the earliest Turn Start.
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
PAGES = os.path.join(DATA, "_raw", "pages")
RAW = os.path.join(DATA, "_raw", "gamedata")

N = E.N
ST = E.ST
# Panic-table cells are written without the [brackets] the skills use.
SO = r"([A-Z][A-Za-z '&.\-]*?)"

PANIC_PATTERNS = [
    # "Clash Power -1, Min & Max Speed -1" (Blue Sand)
    (re.compile(rf"^Clash Power -{N}, Min & Max Speed -{N}$"),
     lambda m: {"kind": "compound", "sub_effects": [
         {"kind": "clash_power", "value": -int(m.group(1))},
         {"kind": "gain", "status": "Bind", "count": int(m.group(2)), "component": "stack"}]}),
    (re.compile(rf"^Clash Power -{N}; Defense Level -{N}$"),
     lambda m: {"kind": "compound", "sub_effects": [
         {"kind": "clash_power", "value": -int(m.group(1))},
         {"kind": "gain", "status": "Defense Level Down", "count": int(m.group(2))}]}),
    (re.compile(rf"^[Gg]ain {N} Sinking and \+{N} Sinking Count at every Turn End; Clash Power -{N}\.$"),
     lambda m: {"kind": "compound", "sub_effects": [
         {"kind": "gain", "status": "Sinking", "potency": int(m.group(1))},
         {"kind": "gain", "status": "Sinking", "count": int(m.group(2)), "status2": "Sinking"},
         {"kind": "clash_power", "value": -int(m.group(3))}]}),
    (re.compile(rf"^Turn End: Gain \+{N} Sinking Count$"),
     lambda m: {"kind": "gain", "status": "Sinking", "count": int(m.group(1)),
                "trigger_turn_end": True}),
    (re.compile(rf"^Turn End: Gain {N} {ST} and {N} {ST} next turn$"),
     lambda m: {"kind": "compound", "sub_effects": [
         {"kind": "gain", "status": m.group(2), "count": int(m.group(1)), "next_turn": True},
         {"kind": "gain", "status": m.group(4), "count": int(m.group(3)), "next_turn": True}]}),
    # Bracket-less forms used by the Panic table.
    (re.compile(rf"^Turn End: Gain {N} {SO} and {N} {SO} next turn$"),
     lambda m: {"kind": "compound", "sub_effects": [
         {"kind": "gain", "status": m.group(2), "count": int(m.group(1)), "next_turn": True},
         {"kind": "gain", "status": m.group(4), "count": int(m.group(3)), "next_turn": True}]}),
    (re.compile(rf"^Turn Start: gain {N} {SO} and {N} {SO}$"),
     lambda m: {"kind": "compound", "sub_effects": [
         {"kind": "gain", "status": m.group(2), "count": int(m.group(1))},
         {"kind": "gain", "status": m.group(4), "count": int(m.group(3))}]}),
    (re.compile(rf"^Turn Start: Gain {N} {SO} and {N} {SO}$"),
     lambda m: {"kind": "compound", "sub_effects": [
         {"kind": "gain", "status": m.group(2), "count": int(m.group(1))},
         {"kind": "gain", "status": m.group(4), "count": int(m.group(3))}]}),
    (re.compile(rf"^Turn Start: (?:gain|Gain) {N} {SO}$"),
     lambda m: {"kind": "gain", "status": m.group(2), "count": int(m.group(1))}),
    (re.compile(rf"^Turn Start: Lose {N} {SO} Potency$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": -int(m.group(1))}),
    (re.compile(rf"^[Gg]ain {N} {SO} at every Turn End$"),
     lambda m: {"kind": "gain", "status": m.group(2), "potency": int(m.group(1)),
                "trigger_turn_end": True}),
    (re.compile(r"^Turn End: Lose .+$"),
     lambda m: {"kind": "noop", "note": "Turn End loss"}),
    (re.compile(r"^Does not act for this turn\.$"),
     lambda m: {"kind": "cannot_act"}),
]


def strip_markup(text: str) -> str:
    text = re.sub(r"<[^>]*>", "", text)
    text = re.sub(r"\{\{StatusEffect\|([^|}]+)[^}]*\}\}", r"\1", text)
    text = re.sub(r"\[\[File:[^\]]*\]\]", "", text)
    text = re.sub(r"\{\{IDIcon\|([^|}]+)[^}]*\}\}", r"\1", text)
    text = re.sub(r"\{\{EnBox\|(\d+)\}\}", r"enemy \1", text)
    text = re.sub(r"\{\{[^}]*\}\}", "", text)
    return text.strip()


def parse_effects(text: str) -> Dict[str, list]:
    """Turn one cell ("Turn End: Gain 1 Bind and 2 Offense Level Down") into
    effects, split on sentence boundaries."""
    effects: List[dict] = []
    unmodeled: List[str] = []
    if not text or text.strip() in ("-", "—"):
        return {"effects": effects, "unmodeled": unmodeled}
    parts = [p.strip() for p in re.split(r"(?<=[.。])\s+|;\s+", text) if p.strip()]
    saved = E.PATTERNS
    E.PATTERNS = PANIC_PATTERNS + saved
    try:
        for part in parts:
            effect = None
            for pattern, handler in E.PATTERNS:
                match = pattern.match(part)
                if match:
                    effect = handler(match)
                    break
            if effect is None:
                unmodeled.append(part)
            else:
                effect["raw"] = part
                effects.append(effect)
    finally:
        E.PATTERNS = saved
    return {"effects": effects, "unmodeled": unmodeled}


def parse_sanity_page() -> Dict[str, dict]:
    """Panic Type -> {low_morale, panic, sources} from the wiki table."""
    path = os.path.join(PAGES, "sanity.wikitext")
    if not os.path.exists(path):
        return {}
    with open(path, encoding="utf-8") as fh:
        text = fh.read()
    start = text.find("==List of Panic Types==")
    if start < 0:
        return {}
    rows = re.split(r"\n\|-", text[start:])
    out: Dict[str, dict] = {}
    for row in rows:
        cells = [strip_markup(c) for c in re.split(r"\n\|", row)]
        cells = [c for c in cells if c]
        if len(cells) < 4:
            continue
        name, low, panic, source = cells[0], cells[1], cells[2], cells[3]
        if not name or name.startswith("!"):
            continue
        out[name] = {
            "type": name,
            "low_morale_text": low,
            "panic_text": panic,
            "sources": source,
        }
    return out


def load(path: str) -> dict:
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def identity_titles() -> Dict[str, str]:
    """identity id -> wiki title (the Panic table references the wiki title)."""
    out: Dict[str, str] = {}
    for name in sorted(os.listdir(os.path.join(DATA, "identities"))):
        if not name.endswith(".json"):
            continue
        record = load(os.path.join(DATA, "identities", name))
        out[record["id"]] = record.get("wiki_title") or record.get("title_en") or ""
    return out


def main() -> int:
    table = parse_sanity_page()
    titles = identity_titles()
    out: Dict[str, dict] = {"version": 1, "types": {}, "identities": {}}
    modelled = 0
    unmodelled = 0
    for name, entry in table.items():
        low = parse_effects(entry["low_morale_text"])
        panic = parse_effects(entry["panic_text"])
        modelled += len(low["effects"]) + len(panic["effects"])
        unmodelled += len(low["unmodeled"]) + len(panic["unmodeled"])
        out["types"][name] = {
            "type": name,
            "sources_text": entry["sources"],
            "low_morale": low["effects"],
            "panic": panic["effects"],
            "low_morale_text": entry["low_morale_text"],
            "panic_text": entry["panic_text"],
            "unmodeled": low["unmodeled"] + panic["unmodeled"],
            "sources": [
                {"kind": "wiki", "language": "en", "title": "Sanity", "verification": "wiki_verified"}
            ],
        }
    # Map every identity to its Panic Type; the default is the plain "Panic"
    # type (no Low Morale effect; Panic: does not act for this turn).
    for identity_id, title in titles.items():
        found = None
        for name, entry in out["types"].items():
            if title and title in (entry.get("sources_text") or ""):
                found = name
                break
        out["identities"][identity_id] = found or "Panic"
    if "Panic" not in out["types"]:
        out["types"]["Panic"] = {
            "type": "Panic",
            "low_morale": [],
            "panic": [{"kind": "cannot_act", "raw": "Does not act for this turn."}],
            "low_morale_text": "",
            "panic_text": "Does not act for this turn.",
            "unmodeled": [],
            "sources": [
                {"kind": "wiki", "language": "en", "title": "Sanity", "verification": "wiki_verified"}
            ],
        }
    os.makedirs(os.path.join(DATA, "mechanics"), exist_ok=True)
    path = os.path.join(DATA, "mechanics", "panic_types.json")
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(out, fh, ensure_ascii=False, indent=1, sort_keys=True)
    print(f"panic types: {len(out['types'])}")
    print(f"identities mapped: {len(out['identities'])}")
    print(f"effects modelled: {modelled}, unmodelled: {unmodelled}")
    print(f"wrote {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
