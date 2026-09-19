"""wiki.gg fetcher routed through r.jina.ai (direct access is WAF-blocked).

Caches every response under data/_raw/ so that data collection is reproducible
and does not hammer the upstream wiki.

Usage:
    python3 tools/wikigg.py page "Solemn Lament Yi Sang" out.wikitext
    python3 tools/wikigg.py search "Ryoshu Lobotomy"
"""

from __future__ import annotations

import hashlib
import json
import os
import sys
import time
import urllib.parse
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
RAW = os.path.join(ROOT, "data", "_raw")
BASE = "https://limbuscompany.wiki.gg/api.php"
JINA = "https://r.jina.ai/"

os.makedirs(RAW, exist_ok=True)


def _cache_path(key: str) -> str:
    return os.path.join(RAW, hashlib.sha256(key.encode()).hexdigest()[:16] + ".json")


def _fetch(url: str, tries: int = 4) -> str:
    last = None
    for i in range(tries):
        req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0 (research)"})
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                return r.read().decode("utf-8", "replace")
        except Exception as exc:  # noqa: BLE001
            last = exc
            time.sleep(3 * (i + 1))
    raise RuntimeError(f"fetch failed: {url}: {last}")


def _strip_jina(text: str) -> str:
    marker = "Markdown Content:"
    idx = text.find(marker)
    return text[idx + len(marker):].lstrip("\n") if idx >= 0 else text


def api(params: dict, use_cache: bool = True) -> dict:
    url = BASE + "?" + urllib.parse.urlencode(params)
    cache = _cache_path(url)
    if use_cache and os.path.exists(cache):
        with open(cache, encoding="utf-8") as fh:
            return json.load(fh)
    text = _strip_jina(_fetch(JINA + url))
    data = json.loads(text)
    with open(cache, "w", encoding="utf-8") as fh:
        json.dump(data, fh, ensure_ascii=False, indent=1)
    time.sleep(4)  # be polite to the jina.ai free tier
    return data


def page(title: str) -> str | None:
    data = api(
        {
            "action": "query",
            "prop": "revisions",
            "rvprop": "content",
            "rvslots": "main",
            "titles": title,
            "format": "json",
            "formatversion": "2",
            "redirects": "1",
        }
    )
    p = data["query"]["pages"][0]
    if "missing" in p or "revisions" not in p:
        return None
    return p["revisions"][0]["slots"]["main"]["content"]


def search(query: str, limit: int = 10) -> list[str]:
    data = api(
        {
            "action": "query",
            "list": "search",
            "srsearch": query,
            "srlimit": str(limit),
            "format": "json",
            "formatversion": "2",
        },
        use_cache=False,
    )
    return [r["title"] for r in data["query"]["search"]]


def main() -> int:
    cmd = sys.argv[1]
    if cmd == "page":
        text = page(sys.argv[2])
        if text is None:
            print("MISSING")
            return 1
        out = sys.argv[3] if len(sys.argv) > 3 else None
        if out:
            with open(out, "w", encoding="utf-8") as fh:
                fh.write(text)
            print(f"wrote {len(text)} chars -> {out}")
        else:
            print(text)
    elif cmd == "search":
        for t in search(sys.argv[2]):
            print(t)
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
