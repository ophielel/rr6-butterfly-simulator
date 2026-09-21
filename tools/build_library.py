"""Build the structured data library from wiki.gg wikitext + in-game localisation JSON.

Design notes
------------
* Two independent sources are joined on the *official internal ID*:
  - wiki.gg page (numbers: HP, speed, coin power, resistances)  -> `wiki`
  - game localisation JSON (official names/descriptions/IDs)     -> `game`
* Nothing is invented: a field that cannot be parsed stays absent and is listed
  in the record's `unparsed` array so the simulator can refuse it in strict mode.
* Uptie resolution follows the wiki convention: a key prefixed with `N` holds the
  value *from uptie N upwards*; the unprefixed key is the uptie-4 value.
  `value(t) = first defined key with tier >= t` (plain == tier 4).
"""

from __future__ import annotations

import json
import os
import re
import sys
from typing import Any, Dict, List, Optional

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)

import wikitext as wt  # noqa: E402

PAGES = os.path.join(ROOT, "data", "_raw", "pages")
GAMEDATA = os.path.join(ROOT, "data", "_raw", "gamedata")
OUT = os.path.join(ROOT, "data")

WIKI = "https://limbuscompany.wiki.gg/wiki/"

# --------------------------------------------------------------------------- #
# identity / E.G.O / enemy page -> official game id
# --------------------------------------------------------------------------- #

IDENTITIES = {
    "10110": ("Lobotomy E.G.O::Solemn Lament Yi Sang", "Lobotomy_E.G.O::Solemn_Lament_Yi_Sang.wikitext", 1),
    "10414": ("Lobotomy E.G.O::Faint Aroma & Solitude Ryōshū", "Lobotomy_EGO_Faint_Aroma_and_Solitude_Ryoshu.wikitext", 4),
    "10813": ("Jeong's Office Rep Ishmael", "Jeong's_Office_Rep_Ishmael.wikitext", 8),
    "10913": ("Lobotomy E.G.O::The Sword Sharpened with Tears Rodion", "Lobotomy_E.G.O::The_Sword_Sharpened_with_Tears_Rodion.wikitext", 9),
    "11004": ("Los Mariachis Jefe Sinclair", "Los_Mariachis_Jefe_Sinclair.wikitext", 10),
    "11114": ("LCA Udjat Vanguard Team 3 Leader Outis", "LCA_Udjat_Vanguard_Team_3_Leader_Outis.wikitext", 11),
    "11214": ("Lobotomy E.G.O::Lamp Gregor", "Lobotomy_E.G.O::Lamp_Gregor.wikitext", 12),
}

EGOS = {
    "20109": ("Solemn Lament Yi Sang", "Solemn_Lament_Yi_Sang.wikitext", 1),
    "20106": ("Bygone Days Yi Sang", "Bygone_Days_Yi_Sang.wikitext", 1),
    "21207": ("Solemn Lament Gregor", "Solemn_Lament_Gregor.wikitext", 12),
    "21206": ("Bygone Days Gregor", "Bygone_Days_Gregor.wikitext", 12),
    "20807": ("Bygone Days Ishmael", "Bygone_Days_Ishmael.wikitext", 8),
    "20810": ("Tidal Elegy Ishmael", "Tidal_Elegy_Ishmael.wikitext", 8),
    "21009": ("Harmony Sinclair", "Harmony_Sinclair.wikitext", 10),
    "20903": ("Rime Shank Rodion", "Rime_Shank_Rodion.wikitext", 9),
}

ENEMIES = {
    "9563": ("Butterfly of Entangled Lives::The Pupa", "boss_pupa_enemy.wikitext"),
    "9567": ("Butterfly of Entangled Lives::Imago", "boss_imago.wikitext"),
    "9564": ("Illusory Butterfly of Entangled Lives::The Past", "illusory_past.wikitext"),
    "9565": ("Illusory Butterfly of Entangled Lives::The Present", "illusory_present.wikitext"),
    "9566": ("Illusory Butterfly of Entangled Lives::The Future", "illusory_future.wikitext"),
}

RESIST_WORDS = {
    "Fatal": 2.0,
    "Weak": 1.5,
    "Normal": 1.0,
    "Endure": 0.75,
    "Ineff": 0.5,
    "Ineffective": 0.5,
    "Immune": 0.0,
}

SIN_COLORS = ["Wrath", "Lust", "Sloth", "Gluttony", "Gloom", "Pride", "Envy"]


# --------------------------------------------------------------------------- #
# helpers
# --------------------------------------------------------------------------- #

def load_json(path: str) -> Any:
    with open(path, encoding="utf-8") as fh:
        return json.load(fh)


def game_index(name: str, lang: str = "") -> Dict[str, dict]:
    base = os.path.join(GAMEDATA, "zh-CN") if lang == "zh" else GAMEDATA
    path = os.path.join(base, name)
    if not os.path.exists(path):
        return {}
    data = load_json(path)
    out = {}
    for entry in data.get("dataList", []):
        out[str(entry.get("id"))] = entry
    return out


def clean(text: str) -> str:
    return wt.strip_markup(text).strip()


def num(text: str, default: Optional[int] = None) -> Optional[int]:
    if text is None:
        return default
    m = re.search(r"[+-]?\d+", text.replace(" ", ""))
    if not m:
        return default
    return int(m.group(0))


def parse_resist(value: str) -> Optional[float]:
    value = value.strip().rstrip(".")
    if value in RESIST_WORDS:
        return RESIST_WORDS[value]
    try:
        return float(value)
    except ValueError:
        return None


def uptie_value(tpl: wt.Template, key: str, tier: int) -> Optional[str]:
    """wiki convention: `Nkey` is the value from uptie N upwards, plain == tier 4."""
    if tier >= 4:
        return tpl.named.get(key)
    for t in range(tier, 4):
        if f"{t}{key}" in tpl.named:
            return tpl.named[f"{t}{key}"]
    return tpl.named.get(key)


def parse_coin_texts(tpl: wt.Template, prefix: str, tier: int) -> List[str]:
    """Collect `ce1..ceN` / `Nce1..` entries for one uptie tier."""
    out: List[str] = []
    idx = 1
    while True:
        key = f"{prefix}{idx}"
        value = None
        if tier >= 4:
            value = tpl.named.get(key)
        else:
            for t in range(tier, 4):
                if f"{t}{key}" in tpl.named:
                    value = tpl.named[f"{t}{key}"]
                    break
            if value is None:
                value = tpl.named.get(key)
        if value is None:
            break
        out.append(clean(value))
        idx += 1
        if idx > 12:
            break
    return out


# --------------------------------------------------------------------------- #
# skills
# --------------------------------------------------------------------------- #

def parse_skill(tpl: wt.Template, skill_id: str, slot: str) -> dict:
    tiers: Dict[str, dict] = {}
    for tier in (1, 2, 3, 4):
        base = num(uptie_value(tpl, "spower", tier))
        coin_power = num(uptie_value(tpl, "cpower", tier))
        entry = {
            "base_power": base,
            "coin_power": coin_power,
            "coins": num(uptie_value(tpl, "coin", tier)),
            "offense_level_mod": num(uptie_value(tpl, "atkmod", tier)),
            "attack_weight": num(uptie_value(tpl, "atkweight", tier)),
            "defense_level_mod": num(uptie_value(tpl, "defmod", tier)),
            "sin_affinity": uptie_value(tpl, "sin", tier),
            "skill_amount": num(uptie_value(tpl, "amt", tier)),
            "on_use_text": clean(uptie_value(tpl, "se", tier) or ""),
            "coin_texts": parse_coin_texts(tpl, "ce", tier),
            "tags": [clean(x) for x in tpl.named.get("tag", "").split(",") if x.strip()],
        }
        tiers[str(tier)] = entry
    return {
        "id": skill_id,
        "slot": slot,
        "name": tpl.named.get("name", ""),
        "type": tpl.named.get("type", ""),
        "rank": num(tpl.named.get("slevel")),
        "upties": tiers,
    }


def parse_skills_from_page(page: wt.Template, page_id: str, extra_slots: bool = False) -> List[dict]:
    """The Skills of an identity.

    `skill1-2` / `skill3-2` (and `-3`) are the alternate Skills a passive can
    trigger ("use 'Stories that Never Cease' as an Unopposed Attack"); they carry
    the same shape as a slot Skill but no Dashboard slot.
    """
    out: List[dict] = []
    for slot in ("skill1", "skill2", "skill3", "defense", "skill4"):
        variants = [slot]
        if extra_slots:
            variants += [f"{slot}-{n}" for n in (2, 3)]
        for name in variants:
            raw = page.named.get(name)
            if not raw:
                continue
            tpl = wt.find_template(raw, "UptieSkills")
            if tpl is None:
                continue
            suffix = {"skill1": "01", "skill2": "02", "skill3": "03", "defense": "04", "skill4": "05"}[slot]
            index = name.split("-")[1] if "-" in name else ""
            skill = parse_skill(
                tpl,
                page_id + suffix + index,
                "defense" if slot == "defense" else slot,
            )
            skill["variant"] = bool(index)
            out.append(skill)
    return out


def parse_passives(page: wt.Template) -> List[dict]:
    out: List[dict] = []
    for key, raw in page.named.items():
        m = re.fullmatch(r"(?:(\d))?passive(\d)", key)
        if not m or not raw:
            continue
        uptie, idx = m.groups()
        text = clean(raw)
        if not text:
            continue
        kind = {"0": "combat_passive", "1": "support_passive_1", "2": "support_passive_2"}.get(idx or "", f"passive{idx}")
        out.append({"kind": kind, "uptie_from": int(uptie) if uptie else 1, "text": text, "raw_key": key})
    out.sort(key=lambda x: (x["kind"], x["uptie_from"]))
    return out


# --------------------------------------------------------------------------- #
# identities
# --------------------------------------------------------------------------- #

def build_identity(game_id: str, meta, en_pers, zh_pers, en_skills, zh_skills) -> dict:
    wiki_title, filename, sinner_no = meta
    path = os.path.join(PAGES, filename)
    text = open(path, encoding="utf-8").read()
    page = wt.find_template(text, "IDPage")
    if page is None:
        raise RuntimeError(f"no IDPage in {filename}")

    unparsed: List[str] = []
    resist = {}
    for key in ("slash", "pierce", "blunt"):
        value = parse_resist(page.get(key))
        if value is None:
            unparsed.append(f"resist.{key}")
        else:
            resist[key] = value

    speed = {}
    for key, tier in (("speed", 4), ("2speed", 3), ("1speed", 2)):
        raw = page.get(key)
        m = re.match(r"(\d+)\s*~\s*(\d+)", raw or "")
        if m:
            speed[str(tier)] = [int(m.group(1)), int(m.group(2))]

    hp = num(page.get("hp"))
    hp_growth = page.get("hpgrowth")
    try:
        hp_growth = float(hp_growth) if hp_growth else None
    except ValueError:
        hp_growth = None
        unparsed.append("hpgrowth")

    stagger = []
    for i in (1, 2, 3, 4):
        raw_stagger = page.get(f"stagger{i}")
        if raw_stagger in (None, ""):
            continue
        value = num(raw_stagger)
        if value is None:
            unparsed.append(f"stagger{i}")
        else:
            stagger.append(value)

    keywords = [clean(m.group(1)) for m in re.finditer(r"\{\{Keyword\|([^}]*)\}\}", page.get("keyword", ""))]

    en_skills_idx = en_skills
    zh_skills_idx = zh_skills
    skills = parse_skills_from_page(page, game_id, extra_slots=True)
    # An alternate Skill (`skill1-2`) has no id of its own on the page; match it to
    # the game record by name.
    by_name = {}
    for gid, record in en_skills_idx.items():
        levels = record.get("levelList") or []
        if levels:
            by_name.setdefault(levels[-1].get("name"), int(gid))
    for skill in skills:
        gid = skill["id"]
        if skill.get("variant") and gid not in en_skills_idx:
            match = by_name.get(skill.get("name"))
            if match is not None:
                skill["id"] = str(match)
                skill["variant_of"] = gid
                gid = skill["id"]
        en = en_skills_idx.get(gid)
        zh = zh_skills_idx.get(gid)
        if en:
            levels = en.get("levelList", [])
            if levels:
                skill["name_en"] = levels[-1].get("name")
        if zh:
            levels = zh.get("levelList", [])
            if levels:
                skill["name_zh"] = levels[-1].get("name")
        if not en:
            unparsed.append(f"game_text:{gid}")

    en_p = en_pers.get(game_id, {})
    zh_p = zh_pers.get(game_id, {})

    return {
        "id": game_id,
        "kind": "identity",
        "sinner": en_p.get("name"),
        "sinner_zh": zh_p.get("name"),
        "title_en": (en_p.get("title") or wiki_title).replace("\n", " "),
        "title_zh": (zh_p.get("title") or "").replace("\n", " "),
        "wiki_title": wiki_title,
        "rarity": num(page.get("rarity")),
        "season": clean(page.get("season")),
        "release_date": page.get("releasedate"),
        "world": page.get("world"),
        "keywords": keywords,
        "stats": {
            "hp": hp,
            "hp_growth": hp_growth,
            "defense_level_mod": num(page.get("defmod")),
            "speed": speed,
            "resist": resist,
            "stagger_thresholds": stagger,
        },
        "skills": skills,
        "passives": parse_passives(page),
        "unparsed": unparsed,
        "sources": [
            {"kind": "wiki", "url": WIKI + wiki_title.replace(" ", "_"), "language": "en",
             "verification": "single_source_verified"},
            {"kind": "game_text", "language": "en", "id": game_id, "verification": "official"},
            {"kind": "game_text_zh_cn", "language": "zh-CN", "id": game_id,
             "verification": "synthetic_translation"},
        ],
    }


# --------------------------------------------------------------------------- #
# E.G.O
# --------------------------------------------------------------------------- #

def build_ego(game_id: str, meta, en_egos, zh_egos) -> dict:
    wiki_title, filename, sinner_no = meta
    path = os.path.join(PAGES, filename)
    if not os.path.exists(path):
        return {"id": game_id, "kind": "ego", "wiki_title": wiki_title, "unparsed": ["page_missing"],
                "sources": [{"kind": "wiki", "url": WIKI + wiki_title.replace(" ", "_"), "language": "en",
                             "verification": "single_source_verified"}]}
    text = open(path, encoding="utf-8").read()
    page = wt.find_template(text, "EGPage")
    if page is None:
        page = wt.find_template(text, "IDPage")

    unparsed: List[str] = []
    costs = {}
    for sin in SIN_COLORS:
        value = num(page.get(f"{sin.lower()}cost"))
        if value:
            # The battle state tracks resources by the lowercase sin key
            # (`setup::sin_key`), so E.G.O costs use the same spelling.
            costs[sin.lower()] = value
    def pick_skill(prefix: str) -> Optional[wt.Template]:
        """Highest threadspin variant of an E.G.O skill (`askill4` > `askill3` > ...)."""
        for suffix in ("4", "3", "2", ""):
            raw = page.get(f"{prefix}{suffix}")
            if raw:
                found = wt.find_template(raw, "Skill")
                if found is not None:
                    return found
        return None

    awak = pick_skill("askill")
    corr = pick_skill("cskill")

    def parse_ego_skill(tpl: Optional[wt.Template]) -> Optional[dict]:
        if tpl is None:
            return None
        coins = []
        for i in range(1, 9):
            raw = tpl.named.get(f"ce{i}")
            coins.append(clean(raw) if raw else "")
        return {
            "name": tpl.named.get("name", ""),
            "sin": tpl.named.get("sin", ""),
            "type": tpl.named.get("type", ""),
            "base_power": num(tpl.named.get("spower")),
            "coin_power": num(tpl.named.get("cpower")),
            "coins": num(tpl.named.get("coin")),
            "attack_weight": num(tpl.named.get("atkweight")),
            "offense_level_mod": num(tpl.named.get("atkmod")),
            "on_use_text": clean(tpl.named.get("se", "")),
            "coin_texts": coins,
            "reuse": "Reuse" in (tpl.named.get("se", "") + " ".join(coins)),
        }

    en = en_egos.get(game_id, {})
    zh = zh_egos.get(game_id, {})

    return {
        "id": game_id,
        "kind": "ego",
        "name_en": en.get("name"),
        "name_zh": zh.get("name"),
        "wiki_title": wiki_title,
        "risk": page.get("risk"),
        "sin_affinity": page.get("affinity"),
        "awakening_sp": num(page.get("asanity")),
        "corrosion_sp": num(page.get("csanity")),
        "resource_cost": costs,
        "awakening": parse_ego_skill(awak),
        "corrosion": parse_ego_skill(corr),
        "passives": parse_passives(page),
        "unparsed": unparsed,
        "sources": [
            {"kind": "wiki", "url": WIKI + wiki_title.replace(" ", "_"), "language": "en",
             "verification": "single_source_verified"},
            {"kind": "game_text", "language": "en", "id": game_id, "verification": "official"},
        ],
    }


# --------------------------------------------------------------------------- #
# enemies
# --------------------------------------------------------------------------- #

def parse_enemy_skills(page: wt.Template) -> List[dict]:
    out: List[dict] = []
    seen = set()
    for key, raw in list(page.named.items()) + [(f"__pos{i}", v) for i, v in enumerate(page.positional)]:
        if "{{Skill" not in (raw or ""):
            continue
        for tpl in wt.find_templates(raw, "Skill"):
            name = tpl.named.get("name", "")
            if (name, tpl.named.get("slevel")) in seen:
                continue
            seen.add((name, tpl.named.get("slevel")))
            coins = []
            for i in range(1, 9):
                raw_coin = tpl.named.get(f"ce{i}")
                coins.append(clean(raw_coin) if raw_coin else "")
            out.append({
                "key": key,
                "name": name,
                "hint": num(tpl.named.get("hint")),
                "level": num(tpl.named.get("level")),
                "sin": tpl.named.get("sin"),
                "rank": num(tpl.named.get("slevel")),
                "type": tpl.named.get("type"),
                "base_power": num(tpl.named.get("spower")),
                "coin_power": num(tpl.named.get("cpower")),
                "coins": num(tpl.named.get("coin")),
                "attack_weight": num(tpl.named.get("atkweight")),
                "offense_level_mod": num(tpl.named.get("atkmod")),
                "on_use_text": clean(tpl.named.get("se", "")),
                "coin_texts": coins,
            })
    return out


def enemy_skill_ids() -> Dict[str, str]:
    """Official skill ids for the enemy skills of the fixed encounter."""
    data = game_index("Skills_Abnormality_Refraction6.json")
    names: Dict[str, List[str]] = {}
    for sid, entry in data.items():
        levels = entry.get("levelList") or []
        if not levels:
            continue
        name = levels[-1].get("name")
        if name:
            names.setdefault(name, []).append(str(sid))
    return names


def with_skill_ids(skills: List[dict], skill_ids: Dict[str, List[str]], enemy_id: str) -> List[dict]:
    prefix = enemy_id[:4]
    for skill in skills:
        name = skill.get("name") or ""
        candidates = [sid for sid in skill_ids.get(name, []) if sid.startswith(prefix)]
        if not candidates:
            candidates = skill_ids.get(name, [])
        skill["id"] = sorted(candidates)[0] if candidates else f"name:{name}"
    return skills


def build_enemy(game_id: str, meta, en_enemies, skill_ids: Dict[str, str]) -> dict:
    wiki_title, filename = meta
    path = os.path.join(PAGES, filename)
    text = open(path, encoding="utf-8").read()
    pages = wt.find_templates(text, "ABPage")
    page = pages[0] if pages else None
    if page is None:
        raise RuntimeError(f"no ABPage in {filename}")

    parts_tpl = wt.find_template(page.get("abnoparts1", ""), "ABPage/Parts")
    parts = []
    if parts_tpl is not None:
        parts.append({
            "name": parts_tpl.get("partsname"),
            "hp": num(parts_tpl.get("hp")),
            "hp_growth": float(parts_tpl.get("hpgrowth") or 0) or None,
            "level": num(parts_tpl.get("level")),
            "speed": parts_tpl.get("speed"),
            "defense_level_mod": num(parts_tpl.get("defmod")),
            "stagger_thresholds": [num(parts_tpl.get(f"stagger{i}")) for i in (1, 2, 3, 4)],
            "resist_physical": {k: parse_resist(parts_tpl.get(k)) for k in ("slash", "pierce", "blunt")},
            "resist_sin": {k.lower(): parse_resist(parts_tpl.get(k.lower())) for k in SIN_COLORS},
        })

    passives = []
    for key, raw in page.named.items():
        if not re.fullmatch(r"passive\d", key) or not raw:
            continue
        tpl = wt.find_template(raw, "Passive")
        if tpl is None:
            continue
        passives.append({
            "name": tpl.get("0"),
            "hint": num(tpl.named.get("hint")),
            "text": clean(tpl.named.get("1") or (tpl.positional[1] if len(tpl.positional) > 1 else "")),
        })

    en = en_enemies.get(game_id, {})

    return {
        "id": game_id,
        "kind": "enemy",
        "name_en": en.get("name") or page.get("name"),
        "wiki_title": wiki_title,
        "abno_code": page.get("abnocode"),
        "risk": page.get("abnorisk"),
        "sin_affinity": page.get("sin"),
        "faction": page.get("faction"),
        "location": page.get("location"),
        "hp": num(page.get("hp")),
        "hp_growth": float(page.get("hpgrowth") or 0) or None,
        "level": num(page.get("level")),
        "parts": parts,
        "skills": with_skill_ids(parse_enemy_skills(page), skill_ids, game_id),
        "passives": passives,
        "unparsed": [],
        "sources": [
            {"kind": "wiki", "url": WIKI + wiki_title.replace(" ", "_"), "language": "en",
             "verification": "single_source_verified"},
            {"kind": "game_text", "language": "en", "id": game_id, "verification": "official"},
        ],
    }


# --------------------------------------------------------------------------- #
# statuses
# --------------------------------------------------------------------------- #

# Status keys the simulator needs, mapping the in-game key to the wiki display name.
STATUS_KEYS = [
    ("Burn", "Burn"), ("Combustion", "Burn"), ("Bleed", "Bleed"), ("Laceration", "Bleed"),
    ("Sinking", "Sinking"), ("SinkingWhite", "Butterfly"), ("BulletLament", "The Living & The Departed"),
    ("ReloadLament", "Reload (Solemn Lament)"), ("Poise", "Poise"), ("Fragile", "Fragile"),
    ("Unbreakable", "Unbreakable Coin"), ("Paralyze", "Paralyze"), ("Vulnerable", "Fragile"),
    ("StackPast", "In the Past (Deactivated)"), ("StackPresent", "In the Present (Deactivated)"),
    ("StackFuture", "In the Future (Deactivated)"), ("StackPastActivate", "In the Past"),
    ("StackPresentActivate", "In the Present"), ("StackFutureActivate", "In the Future"),
    ("TimeGap", "Temporal Disjunction"), ("HeatedWingScales", "Incandescent Scale Dust"),
    ("TransparentWingScales", "Acuate Scale Dust"), ("RustedWingScales", "Rusted Scale Dust"),
    ("DamageDown", "Damage Down"), ("PowerDown", "Power Down"), ("Haste", "Haste"),
    ("OffenseLevelUp", "Offense Level Up"), ("DefenseLevelUp", "Defense Level Up"),
    ("WrathFragility", "Wrath Fragility"), ("HPHealingDown", "HP Healing Down"),
]


# Statuses the fixed content uses beyond the hand-listed core ones; each is
# resolved by display name in the in-game data so the official key is used.
EXTRA_STATUS_NAMES = [
    "Tremor", "Tremor Burst", "Amplitude Conversion", "Charge", "Rupture", "Protection",
    "Damage Up", "Damage Down", "Power Up", "Power Down", "Attack Power Up", "Attack Power Down",
    "Offense Level Up", "Offense Level Down", "Defense Level Up", "Defense Level Down",
    "Bind", "Haste", "Paralyze", "Plus Coin Boost", "Minus Coin Drop", "Clash Power Up",
    "HP Healing Down", "Wrath Fragility", "Gloom Fragility", "Gloom Resist Down",
    "Slash Resist Down", "Pierce Resist Down", "Blunt Resist Down", "Unbreakable Coin",
    "No Damage Taken", "Discard", "Aggro", "Dazzle", "Deep Tears", "Tear-sharpened",
    "Protecting Sword", "Faint Aroma", "Blue Sand", "Bright -光-", "Suit", "Lamp",
    "Incandescent Scale Dust", "Acuate Scale Dust", "Rusted Scale Dust",
    "The Udjat -Vanguard-", "LCA Fracture Round", "Bullet - Solitude", "Tremor - Decay",
]


# Names the wiki/game data treat as Stack-based but whose text is ambiguous.
STACK_STATUS_NAMES = {
    "In the Past", "In the Present", "In the Future", "Temporal Disjunction",
    "Dazzle", "Faint Aroma", "Lamp", "Deep Tears", "Tear-sharpened",
    "Protecting Sword", "Bright -光-", "Suit", "Aggro", "Blue Sand",
    "LCA Fracture Round", "Bullet - Solitude", "The Udjat -Vanguard-",
}


# Statuses whose plain "Gain/Inflict N [X]" form fills **Count**, because their
# own text measures the effect by Count ("Take less damage from skills based on
# the effect's Count for one turn").  Everything else fills Potency
# (JA-wiki 戦闘システム詳細: 「火傷を１付与」= +1 Burn Potency; the in-game
# `Bufs` templates fill {0} with Potency for the damaging statuses).
COUNT_PRIMARY = {
    "Protection",
    "Fragile",
    "Haste",
    "Bind",
    "Charge",
    # Their own in-game text measures the effect by Count ("Deal less damage with
    # skills based on the effect's Count", "Raise the Power of Plus Coins by the
    # effect's Count"), so a bare "Gain N [X]" fills Count.
    "Damage Down",
    "Plus Coin Boost",
    "Minus Coin Drop",
    "Gloom Resist Down",
}


def status_primary(text: str, name: str, stack_based: bool) -> str:
    """Which component a plain "Gain/Inflict N [X]" fills.

    Read from the status's own text when it says how it is measured ("based on
    Stack", "based on the effect's Count"), with the hand table above and
    Potency as the fallback (JA-wiki 戦闘システム詳細: 「火傷を１付与」= +1 Burn
    Potency).
    """
    lowered = text.lower()
    if stack_based or "based on stack" in lowered or "per stack" in lowered:
        return "stack"
    if name in COUNT_PRIMARY or "the effect's count" in lowered:
        return "count"
    return "potency"


def status_expiry(text: str, name: str, stack_based: bool, primary: str) -> str:
    """When a status stops existing.

    Source: wiki.gg `Status Effects` (Overview) - "In single-value or
    double-value modes, if one or more of the values reach 0, the status effect
    is removed from the unit", plus each status's own text for the exceptions
    (Butterfly: "expires when both The Living and The Departed reach 0"; the
    Unique Ammo statuses have no expiry rule).
    """
    lowered = text.lower()
    if stack_based:
        # Stack statuses are removed by their own rules ("Max Stack", "Turn End:
        # Lose 1 Stack", "Expires at Turn End").
        return "none"
    if "expires when both" in lowered:
        return "both_zero"
    if "ammo" in lowered:
        return "none"
    if primary == "count":
        # Protection / Fragile / Haste / Bind / Charge only ever use Count.
        return "count_zero"
    if "count" not in lowered:
        return "potency_zero"
    return "either_zero"


def status_expires_at_turn_end(text: str) -> bool:
    """A status that only lasts the turn it was applied in ("... for one turn",
    "for this turn") is removed at Turn End."""
    lowered = text.lower()
    if "expires at turn end" in lowered or "reduced by 1 at turn end" in lowered:
        return True
    return any(
        marker in lowered
        for marker in ("for one turn", "for this turn", "this turn)", "for the turn")
    )


def status_is_stack(text: str, name: str = "") -> bool:
    """A status is Stack-based when its text is about Stack and not about
    Potency/Count (the engine applies the two differently)."""
    if name in STACK_STATUS_NAMES:
        return True
    if name.endswith("Resist Down") or "Fragility" in name or name in (
        "Fragile",
        "Protection",
    ):
        return False
    lowered = text.lower()
    if "potency and the count" in lowered:
        return False
    if "count" in lowered and "stack" not in lowered:
        return False
    if "per count" in lowered or "count)" in lowered.replace("stack)", ""):
        return False
    return any(marker in text for marker in ("Max Stack", "per Stack", "Stack)", "Stack:", "Stack "))


def build_extra_statuses(tables_en, tables_zh, existing: List[dict]) -> List[dict]:
    by_name = {}
    for table in tables_en:
        for key, entry in table.items():
            name = (entry.get("name") or "").strip()
            if name and name not in by_name:
                by_name[name] = (key, entry)
    zh_by_key = {}
    for table in tables_zh:
        zh_by_key.update(table)
    known = {record["wiki_name"] or "" for record in existing}
    out = []
    for name in EXTRA_STATUS_NAMES:
        found = by_name.get(name)
        if not found:
            continue
        key, entry = found
        if key in {record["key"] for record in existing}:
            continue
        text = entry.get("desc", "")
        stack_based = status_is_stack(text, entry.get("name") or "")
        zh = zh_by_key.get(key) or {}
        out.append({
            "key": key,
            "structure": "stack" if stack_based else "potency_count",
            "primary": status_primary(text, entry.get("name") or "", stack_based),
            "expiry": status_expiry(
                text,
                entry.get("name") or "",
                stack_based,
                "count" if (entry.get("name") or "") in COUNT_PRIMARY else "potency",
            ),
            "expires_at_turn_end": status_expires_at_turn_end(text),
            "name_en": entry.get("name"),
            "name_zh": zh.get("name"),
            "wiki_name": name,
            "text_en": text.strip(),
            "text_zh": (zh.get("desc") or "").strip(),
            "source_kind": "buff",
            "sources": [
                {"kind": "game_text", "language": "en", "id": key, "verification": "official"}
            ],
        })
    _ = known
    return out


def build_statuses(kw_en, kw_zh, bufs_en, bufs_zh) -> List[dict]:
    out = []
    tables_en = [(kw_en, "keyword"), (bufs_en, "buff")]
    lookup_zh = {}
    lookup_zh.update(kw_zh)
    lookup_zh.update(bufs_zh)
    for key, wiki_name in STATUS_KEYS:
        entry = None
        source = None
        for table, kind in tables_en:
            if key in table:
                entry = table[key]
                source = kind
                break
        if entry is None:
            continue
        zh = lookup_zh.get(key)
        text = entry.get("desc", "")
        # Stack-based statuses ("Max Stack: N", "per Stack", "Lose 1 Stack")
        # behave differently from Potency/Count statuses.
        stack_based = status_is_stack(text, entry.get("name") or "")
        out.append({
            "key": key,
            "structure": "stack" if stack_based else "potency_count",
            "primary": status_primary(text, entry.get("name") or "", stack_based),
            "expiry": status_expiry(
                text,
                entry.get("name") or "",
                stack_based,
                "count" if (entry.get("name") or "") in COUNT_PRIMARY else "potency",
            ),
            "expires_at_turn_end": status_expires_at_turn_end(text),
            "name_en": entry.get("name"),
            "name_zh": (zh or {}).get("name"),
            "wiki_name": wiki_name,
            "text_en": entry.get("desc", "").strip(),
            "text_zh": (zh or {}).get("desc", "").strip(),
            "source_kind": source,
            "sources": [{"kind": "game_text", "language": "en", "id": key, "verification": "official"}],
        })
    return out


# --------------------------------------------------------------------------- #
# main
# --------------------------------------------------------------------------- #

def merge(dst: Dict[str, dict], src: Dict[str, dict]) -> Dict[str, dict]:
    dst.update(src)
    return dst


def main() -> int:
    en_pers = game_index("Personalities.json")
    zh_pers = game_index("Personalities.json", "zh")
    en_egos = game_index("Egos.json")
    zh_egos = game_index("Egos.json", "zh")
    en_enemies = game_index("Enemies_Refraction6.json")

    personality_skills: Dict[str, dict] = {}
    zh_personality_skills: Dict[str, dict] = {}
    for i in range(1, 13):
        merge(personality_skills, game_index(f"Skills_personality-{i:02d}.json"))
        merge(zh_personality_skills, game_index(f"Skills_personality-{i:02d}.json", "zh"))
    merge(personality_skills, game_index("Skills_personality-x1p1c1.json"))
    merge(personality_skills, game_index("Skills.json"))
    merge(zh_personality_skills, game_index("Skills.json", "zh"))

    def dump(sub: str, name: str, payload: Any) -> None:
        d = os.path.join(OUT, sub)
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, name), "w", encoding="utf-8") as fh:
            json.dump(payload, fh, ensure_ascii=False, indent=2, sort_keys=False)

    for gid, meta in IDENTITIES.items():
        rec = build_identity(gid, meta, en_pers, zh_pers, personality_skills, zh_personality_skills)
        dump("identities", f"{gid}.json", rec)
        print(f"identity {gid:>6} {rec['title_en'][:40]:42s} skills={len(rec['skills'])} unparsed={len(rec['unparsed'])}")

    for gid, meta in EGOS.items():
        rec = build_ego(gid, meta, en_egos, zh_egos)
        dump("ego", f"{gid}.json", rec)
        print(f"ego      {gid:>6} {rec.get('name_en') or '?':42s} cost={rec.get('resource_cost')}")

    for gid, meta in ENEMIES.items():
        rec = build_enemy(gid, meta, en_enemies, enemy_skill_ids())
        dump("enemies", f"{gid}.json", rec)
        print(f"enemy    {gid:>6} {rec['name_en'][:40]:42s} skills={len(rec['skills'])} passives={len(rec['passives'])}")

    kw_en = game_index("BattleKeywords.json")
    merge(kw_en, game_index("BattleKeywords-walpu4.json"))
    kw_zh = game_index("BattleKeywords.json", "zh")
    merge(kw_zh, game_index("BattleKeywords-walpu4.json", "zh"))
    bufs_en = game_index("Bufs.json")
    merge(bufs_en, game_index("Bufs-walpu4.json"))
    merge(bufs_en, game_index("Bufs_Refraction6.json"))
    merge(bufs_en, game_index("BattleKeywords_Refraction6.json"))
    merge(bufs_en, game_index("Bufs-a1c10p1.json"))
    bufs_zh = game_index("Bufs.json", "zh")
    merge(bufs_zh, game_index("Bufs-walpu4.json", "zh"))
    merge(bufs_zh, game_index("Bufs_Refraction6.json", "zh"))
    merge(bufs_zh, game_index("BattleKeywords_Refraction6.json", "zh"))
    statuses = build_statuses(kw_en, kw_zh, bufs_en, bufs_zh)
    statuses.extend(
        build_extra_statuses(
            [kw_en, bufs_en, game_index("Bufs-a1c10p1.json"), game_index("BattleKeywords-a1c10p1.json")],
            [kw_zh, bufs_zh, game_index("Bufs-a1c10p1.json", "zh"), game_index("BattleKeywords-a1c10p1.json", "zh")],
            statuses,
        )
    )
    dump("statuses", "statuses.json", statuses)
    print(f"statuses {len(statuses)}")

    sources = {
        "wiki": {
            "site": "https://limbuscompany.wiki.gg/",
            "access": "via r.jina.ai (direct access is WAF-blocked)",
            "language": "en",
            "verification": "single_source_verified",
            "fetched_pages": sorted(os.listdir(PAGES)),
        },
        "game_text": load_json(os.path.join(OUT, "sources", "gamedata.json")),
    }
    dump("sources", "_index.json", sources)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
