# Data library

The simulator never reads a wiki at runtime.  Everything lives in `data/` and is
produced by the scripts in `tools/`, so a given library state can be rebuilt and
diffed.

```
data/
  _raw/                 cached upstream responses (never edited by hand)
    pages/*.wikitext    wiki.gg pages, fetched through r.jina.ai
    gamedata/*.json     in-game localisation (en) + zh-CN community pack
    wiki_status_text.json
  identities/10110.json ...   one file per identity, keyed by official id
  ego/20109.json ...          one file per E.G.O
  enemies/9567.json ...       one file per enemy (Pupa, Imago, illusory butterflies)
  statuses/statuses.json      statuses referenced by the fixed content
  mechanics/effects.json      machine-readable skill mechanics (+ coverage.json)
  sources/_index.json         source registry
  sources/gamedata.json       pinned commits of the localisation dumps
```

## Sources

| Source | Role | Language | Verification |
|--------|------|----------|--------------|
| wiki.gg `limbuscompany.wiki.gg` (via `r.jina.ai`) | numbers: HP, speed, coin power, resistances, effect text | en | `single_source_verified` |
| `x1bViolet/Limbus-Localization-Files` | official in-game text keyed by internal id (identity/skill/ego/status names) | en | `official` |
| `LocalizeLimbusCompany` (`LLC_zh-CN`) | Simplified-Chinese names for display only | zh-CN | `auxiliary_translation` |
| Fandom `limbuscompany.fandom.com` | cross-check only (clash/sanity wording) | en | `auxiliary` |
| `wikiwiki.jp/lcbwiki` (`戦闘システム詳細`) | clash resolution, dashboard/panel rules, attack accumulation | ja | `multi_source_verified` |

Direct requests to wiki.gg are blocked by the host's WAF, so `tools/wikigg.py`
routes the MediaWiki API through `r.jina.ai` and caches every response in
`data/_raw/`.  `tools/fetch_gamedata.py` pins the exact commit SHA of both
localisation repositories in `data/sources/gamedata.json`.

## Pipeline

```bash
python3 tools/fetch_gamedata.py     # in-game text (pinned commits)
python3 tools/fetch_pages.py        # wiki.gg pages used by the fixed content
python3 tools/build_library.py      # -> data/identities, data/ego, data/enemies, data/statuses
python3 tools/extract_effects.py    # -> data/mechanics/effects.json
python3 tools/report.py             # -> docs/COVERAGE.md, docs/DATA_REPORT.md
```

Everything is idempotent and cached; re-running only fetches what is missing.

## How effect text becomes mechanics

`extract_effects.py` reads the effect text of each skill (already stored in the
library, with status names in `[brackets]`) and matches it against a fixed list
of patterns.  Each produced effect keeps the line it came from in `raw`:

```json
{
  "kind": "coin_power",
  "value": 1,
  "condition": {"source": "target", "statuses": ["Sinking", "Butterfly", "Butterfly"], "gte": 6},
  "raw": "If the sum of the target's [Sinking] and both [Butterfly] is 6 or higher, Coin Power +1"
}
```

A status listed twice means "both values" — the wiki writes that as
`both [Butterfly]` (Potency **and** Count), which the extractor normalises.

Lines that match no pattern go into `unmodeled` for that skill.  They are
reported by `Simulator::strict_blockers()` and refuse to run in strict mode, so
an unimplemented effect can never silently change a result.

## Adding a mechanic

1. Add the effect kind to `sim/crates/lcb-core/src/effects.rs` (data) and handle
   it in `battle::apply_effects` (behaviour).
2. Add a pattern to `tools/extract_effects.py`, or, if the text is irregular,
   author the entry directly in `data/mechanics/effects.json`.
3. Add a test in `sim/crates/lcb-core/tests/mechanics.rs` that names the source.
4. Re-run the pipeline and `cargo test`.

## Boss rotation

The Imago's rotations are documented on the wiki in a notation that this project
does not consider unambiguous, so the engine does not guess them.  The default
policy is "first listed skill" (`battle::choose_enemy_skill`) and the encounter
configuration is the place to plug in a real script once it is confirmed.  The
raw rotation text is preserved verbatim in
`data/_raw/pages/boss_imago.wikitext`.
