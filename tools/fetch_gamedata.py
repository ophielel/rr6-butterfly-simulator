"""Download game localization JSON (official in-game text, keyed by internal IDs).

Sources
-------
* EN dump:  x1bViolet/Limbus-Localization-Files  (mirror of the client's Localize folder)
* zh-CN:    LocalizeLimbusCompany/LocalizeLimbusCompany :: LLC_zh-CN (community zh-CN pack)

Both packs use the game's own file names and internal IDs, which is what makes
them joinable.  Every download is pinned by commit SHA in data/sources/gamedata.json.
"""

from __future__ import annotations

import json
import os
import sys
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
RAW = os.path.join(ROOT, "data", "_raw", "gamedata")
os.makedirs(RAW, exist_ok=True)

EN_REPO = "x1bViolet/Limbus-Localization-Files"
EN_BRANCH = "English"
ZH_REPO = "LocalizeLimbusCompany/LocalizeLimbusCompany"
ZH_BRANCH = "main"
ZH_DIR = "LLC_zh-CN"

FILES = [
    "Personalities.json",
    "Egos.json",
    "Skills_personality.json",
    "Skills_Ego_Personality.json",
    "Skills_Abnormality_Refraction6.json",
    "Passives.json",
    "Passives_Abnormality_Refraction6.json",
    "Enemies_Refraction6.json",
    "Bufs.json",
    "Bufs_Refraction6.json",
    "BattleKeywords.json",
    "BattleKeywords_Refraction6.json",
    "UnitKeyword.json",
    "Passive_Ego.json",
    "Skills_Ego.json",
    "RailwayDungeon.json",
    "RailwayDungeonStationName.json",
]


def get(url: str) -> bytes:
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0 (research)"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return r.read()


def head_sha(repo: str, branch: str) -> str:
    data = json.loads(get(f"https://api.github.com/repos/{repo}/commits/{branch}"))
    return data["sha"]


def download(repo: str, branch: str, path: str, out: str) -> bool:
    url = f"https://raw.githubusercontent.com/{repo}/{branch}/{path}"
    try:
        raw = get(url)
    except Exception as exc:  # noqa: BLE001
        print(f"  FAIL {path}: {exc}")
        return False
    with open(out, "wb") as fh:
        fh.write(raw)
    print(f"  ok {len(raw):8d} {path}")
    return True


def main() -> int:
    meta_path = os.path.join(ROOT, "data", "sources", "gamedata.json")
    meta = {}
    if os.path.exists(meta_path):
        with open(meta_path, encoding="utf-8") as fh:
            meta = json.load(fh)
    en_sha = head_sha(EN_REPO, EN_BRANCH)
    zh_sha = head_sha(ZH_REPO, ZH_BRANCH)

    for name in FILES:
        out = os.path.join(RAW, name)
        if os.path.exists(out) and os.path.getsize(out) > 0:
            print(f"  cached {name}")
            continue
        print(f"doc {name}")
        download(EN_REPO, en_sha, name, out)
        time.sleep(0.5)

    zh_dir = os.path.join(RAW, "zh-CN")
    os.makedirs(zh_dir, exist_ok=True)
    for name in FILES:
        out = os.path.join(zh_dir, name)
        if os.path.exists(out) and os.path.getsize(out) > 0:
            continue
        download(ZH_REPO, zh_sha, f"{ZH_DIR}/{name}", out)
        time.sleep(0.5)

    meta["en"] = {
        "repo": EN_REPO,
        "commit": en_sha,
        "language": "en",
        "kind": "official_game_text",
        "note": "mirror of LimbusCompany_Data Localize folder",
    }
    meta["zh_cn"] = {
        "repo": ZH_REPO,
        "commit": zh_sha,
        "language": "zh-CN",
        "kind": "community_translation",
        "note": "都市零协会汉化组 language pack; auxiliary source only",
    }
    os.makedirs(os.path.dirname(meta_path), exist_ok=True)
    with open(meta_path, "w", encoding="utf-8") as fh:
        json.dump(meta, fh, ensure_ascii=False, indent=2)
    print("pinned:", en_sha[:10], zh_sha[:10])
    return 0


if __name__ == "__main__":
    sys.exit(main())
