"""Fetch the Japanese wiki battle-system page used as the second source for the
clash / dashboard / attack rules.

The JA wiki has no MediaWiki API we can use, so the page is downloaded as HTML
and flattened to text.  The result is cached next to the other raw sources.
"""

from __future__ import annotations

import html
import os
import re
import subprocess
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
OUT = os.path.join(ROOT, "data", "_raw", "pages")

PAGES = {
    "/lcbwiki/Wiki管理/戦闘指南/戦闘システム詳細": "ja_wiki_battle_system_details.txt",
    "/lcbwiki/Wiki管理/戦闘指南/守備スキル": "ja_wiki_defense_skills.txt",
    "/lcbwiki/Wiki管理/戦闘指南/威力": "ja_wiki_power.txt",
    "/lcbwiki/Wiki管理/戦闘指南/破壊不能コイン": "ja_wiki_unbreakable_coin.txt",
    "/lcbwiki/Wiki管理/戦闘指南/ダメージ計算": "ja_wiki_damage.txt",
}

UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/126 Safari/537.36"


def flatten(raw: str) -> str:
    body = re.sub(r"<script.*?</script>", "", raw, flags=re.S)
    body = re.sub(r"<style.*?</style>", "", body, flags=re.S)
    body = re.sub(r"<(br|/tr|/p|/div|/li|/h\d)[^>]*>", "\n", body)
    text = re.sub(r"<[^>]+>", " ", body)
    text = html.unescape(text)
    text = re.sub(r"[ \t\u3000]+", " ", text)
    return re.sub(r"\n\s*\n+", "\n", text)


def main() -> int:
    os.makedirs(OUT, exist_ok=True)
    for page, name in PAGES.items():
        path = os.path.join(OUT, name)
        if os.path.exists(path) and os.path.getsize(path) > 0:
            print(f"cached {name}")
            continue
        url = "https://wikiwiki.jp" + urllib.parse.quote(page)
        raw = subprocess.run(
            ["curl", "-s", "-A", UA, url], capture_output=True, check=True
        ).stdout.decode("utf-8", "replace")
        text = flatten(raw)
        if len(text) < 2000:
            print(f"FAIL {name} (unexpectedly short: {len(text)})")
            continue
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(text)
        print(f"ok {len(text):7d} {name}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
