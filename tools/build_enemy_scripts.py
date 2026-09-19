"""Build the enemy action-pattern scripts.

The Imago's rotation is documented in two places:

* wiki.gg `Butterfly of Entangled Lives 羅生蝶/Enemy/...::Imago` - the turns are
  written as "In the Past: Temper and Cast, Temper and Cast, Fluttering Havoc x4"
  per turn, with separate blocks for below 66% and below 33% HP.
* the Japanese wiki `幻想体/羅生蝶` - the same thing as a table with a slot count
  ("行動総数 6 / 6 / 4") and the small/mid/big skill per time state.

This script parses the wiki.gg text (which already lives in `data/_raw/pages/`)
and cross-checks the result against the Japanese table stored below verbatim.
The output is `data/mechanics/enemy_scripts.json`.
"""

from __future__ import annotations

import json
import os
import re
import sys
from typing import Dict, List

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
PAGES = os.path.join(ROOT, "data", "_raw", "pages")
DATA = os.path.join(ROOT, "data")

IMAGO = "9567"

# Verbatim copy of the Japanese wiki action-pattern table for station 5, used as
# a cross-check for the parsed wiki.gg rotation.
JA_TABLE = """
第5区間  混乱で行動がスキップされない。全てで1~3を繰り返す。
1 小技 ×2, 《乱撃》×4  6
2 中技 ×2, 《粉砕》×4  6
3 大技 , 《揺乱》×3  4
HP66%以下
1 小技 ×3, 《乱撃》×3  6
2 中技 ×3, 《粉砕》×3  6
3 大技 , 中技 , 《揺乱》×2  4
HP33%以下
1 小技 ×4, 《乱撃》×2  6
2 中技 ×4, 《粉砕》×2  6
3 大技 ×2, 中技 , 《揺乱》  4
※小技・中技・大技は時間状態によって変化
過去：《鍛冶》《燒燬》《劫火》
現在：《安寧》《天穿》《破邪》
未来：《腐蝕分解》《朽滅》《血花》
"""

# Station 1 (the Pupa).  Verbatim Japanese action-pattern table; the wiki.gg
# page lists the same six-slot lines with fewer slot details.
JA_PUPA = """
第1区間  羅生蝶::繭の行動  行動総数
1 《乱撃》×3, 《粉砕》×3  6
2 (1と同じ)  6
3 《乱撃》×3, 《粉砕》×3, 《胎動開始》  7
a 《羅生 - 回帰》, 《羅生 - 顕現》, 《羅生 - 到来》, 《胎動開始》  4
※ 1~3を行う。バリアを全て消耗すると次のターン、aを行う。
"""

PUPA_NAMES = {
    "乱撃": "Fluttering Havoc",
    "粉砕": "Pulverization",
    "胎動開始": "The Quickening",
    "羅生 - 回帰": "Entangled Life - Regression",
    "羅生 - 顕現": "Entangled Life - Manifestation",
    "羅生 - 到来": "Entangled Life - Advent",
}

# Japanese names -> what the wiki.gg rotation calls them, per time state.
JA_SHORT = {
    "past": {"small": "Temper and Cast", "mid": "Immolation", "big": "Kalpāgni"},
    "present": {"small": "Anitya", "mid": "Skypiercer", "big": "Smite the Wicked"},
    "future": {
        "small": "Corrosive Disintegration",
        "mid": "Rotting Annihilation",
        "big": "Bloodflower",
    },
}

COMMON = ["Fluttering Havoc", "Pulverization", "Chaotic Turmoil"]

STATE_HEADINGS = {
    "====In the Past====": "past",
    "====In the Present====": "present",
    "====In the Future====": "future",
}


def load_skills() -> Dict[str, str]:
    record = json.load(open(os.path.join(DATA, "enemies", f"{IMAGO}.json"), encoding="utf-8"))
    return {s["name"]: s["id"] for s in record["skills"]}


def parse_rotation(text: str) -> Dict[str, Dict[str, List[List[str]]]]:
    """-> {state: {"above_66": [[skill, ...], ...], "below_66": [...], "below_33": [...]}}"""
    out: Dict[str, Dict[str, List[List[str]]]] = {}
    state = None
    band = "above_66"
    for raw in text.split("\n"):
        line = raw.strip()
        if line in STATE_HEADINGS:
            state = STATE_HEADINGS[line]
            out[state] = {}
            band = "above_66"
            continue
        if state is None:
            continue
        if line.startswith("Below 66% HP"):
            band = "below_66"
            continue
        if line.startswith("Below 33% HP"):
            band = "below_33"
            continue
        if line.startswith("*"):
            skills = [s.strip() for s in line.lstrip("* ").split(",") if s.strip()]
            out[state].setdefault(band, []).append(skills)
        elif line.startswith("====") and line not in STATE_HEADINGS:
            state = None
    return out


def cross_check(parsed: Dict[str, Dict[str, List[List[str]]]]) -> List[str]:
    """Verify the parsed rotation against the Japanese table (slot counts)."""
    problems: List[str] = []
    expected_counts = {
        "above_66": [6, 6, 4],
        "below_66": [6, 6, 4],
        "below_33": [6, 6, 4],
    }
    for state, bands in parsed.items():
        for band, turns in bands.items():
            counts = [len(t) for t in turns]
            if counts != expected_counts[band]:
                problems.append(f"{state}/{band}: slot counts {counts} != {expected_counts[band]}")
            # turn 3 always ends with Chaotic Turmoil (《揺乱》)
            if not turns[-1][-1].startswith("Chaotic"):
                problems.append(f"{state}/{band}: last skill is {turns[-1][-1]}, expected Chaotic Turmoil")
            # small/mid/big names must match the JA table
            small = turns[0][0]
            mid = turns[1][0]
            big = turns[2][0]
            want = JA_SHORT[state]
            if (small, mid, big) != (want["small"], want["mid"], want["big"]):
                problems.append(
                    f"{state}: small/mid/big {(small, mid, big)} != {(want['small'], want['mid'], want['big'])}"
                )
    return problems


def build_pupa() -> dict:
    record = json.load(open(os.path.join(DATA, "enemies", "9563.json"), encoding="utf-8"))
    ids = {s["name"]: s["id"] for s in record["skills"]}
    havoc = ids["Fluttering Havoc"]
    pulv = ids["Pulverization"]
    quick = ids["The Quickening"]
    regression = ids["Entangled Life - Regression"]
    manifestation = ids["Entangled Life - Manifestation"]
    advent = ids["Entangled Life - Advent"]
    return {
        "enemy_id": "9563",
        "slots": 6,
        "cycle_turns": 3,
        "station": 1,
        "acts_while_staggered": False,
        "turns": [
            [havoc, havoc, havoc, pulv, pulv, pulv],
            [havoc, havoc, havoc, pulv, pulv, pulv],
            [havoc, havoc, havoc, pulv, pulv, pulv, quick],
        ],
        "branch": {
            "when": "barrier_broken",
            "turns": [[regression, manifestation, advent, quick]],
        },
        "shield_percent": 1.3,
        "hp_floor_percent": 90,
        "ends_encounter_on": [quick],
        "notes": [
            "Station 1: at encounter start the Pupa gains 1.3% of its max HP as Shield and its HP does not fall below 90%.",
            "If the Shield is fully consumed before the end of turn 3, the next turn uses pattern a and the encounter ends after The Quickening.",
            "The Quickening is Unclashable, Target Fixed and deals 0 damage; its Attack End ends the Encounter.",
        ],
        "source": [
            "https://limbuscompany.wiki.gg/wiki/Butterfly_of_Entangled_Lives_%E7%BE%85%E7%94%9F%E8%9D%B6/Enemy/Butterfly_of_Entangled_Lives::The_Pupa",
            "https://wikiwiki.jp/lcbwiki/幻想体/羅生蝶 (行動パターン)",
        ],
        "ja_table": JA_PUPA.strip(),
    }


def main() -> int:
    text = open(os.path.join(PAGES, "boss_imago.wikitext"), encoding="utf-8").read()
    parsed = parse_rotation(text)
    problems = cross_check(parsed)
    for problem in problems:
        print("WARN", problem)

    skills = load_skills()
    missing = set()
    for state, bands in parsed.items():
        for turns in bands.values():
            for turn in turns:
                for name in turn:
                    if name not in skills:
                        missing.add(name)
    if missing:
        print("ERROR unknown skill names:", sorted(missing))
        return 1

    script = {
        "imago": {
            "enemy_id": IMAGO,
            "slots": 6,
            "cycle_turns": 3,
            "station": 5,
            "source": [
                "https://limbuscompany.wiki.gg/wiki/Butterfly_of_Entangled_Lives_%E7%BE%85%E7%94%9F%E8%9D%B6/Enemy/Butterfly_of_Entangled_Lives::Imago",
                "https://wikiwiki.jp/lcbwiki/幻想体/羅生蝶 (行動パターン)",
            ],
            "acts_while_staggered": True,
            "clash_count_swing": {"threshold": 10, "divisor": 10},
            "notes": [
                "Station 5: the Imago acts even while Staggered (JA table: 混乱で行動がスキップされない).",
                "Turn 3 uses fewer slots than turns 1-2; the remaining slots stay idle.",
                "The small/mid/big skill of turns 1-2 depends on the active time state.",
            ],
            "ja_table": JA_TABLE.strip(),
            "shared": {name: skills[name] for name in COMMON},
            "states": {},
        }
    }

    for state, bands in parsed.items():
        entry = {"bands": []}
        for band, turns in bands.items():
            entry["bands"].append(
                {
                    "band": band,
                    "turns": [[skills[name] for name in turn] for turn in turns],
                }
            )
        entry["small"] = skills[JA_SHORT[state]["small"]]
        entry["mid"] = skills[JA_SHORT[state]["mid"]]
        entry["big"] = skills[JA_SHORT[state]["big"]]
        script["imago"]["states"][state] = entry

    out = {
        "version": 1,
        "skills": {"imago": script["imago"], "pupa": build_pupa()},
    }
    path = os.path.join(DATA, "mechanics", "enemy_scripts.json")
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(out, fh, ensure_ascii=False, indent=1, sort_keys=True)
    print(f"wrote {path}")
    for state, bands in parsed.items():
        for band, turns in bands.items():
            print(f"  {state:8s} {band:9s} " + " | ".join(str(len(t)) for t in turns))
    return 0


if __name__ == "__main__":
    sys.exit(main())
