# RR6 Butterfly of Entangled Lives — simulator

A small, source-driven combat simulator for the fixed content in
[`docs/archive/rr6_butterfly_simulator_plan_v2.md`](docs/archive/rr6_butterfly_simulator_plan_v2.md):
seven identities, seven E.G.O, and the
**Refraction Railway Line 6, Section 5 boss** *Butterfly of Entangled Lives
[羅生蝶]* (`9567`, the Imago).

Scope note: the deliverables are the seven identities, their E.G.O and the
Section 5 encounter, which is the Imago **together with its three Illusory
Butterfly allies** (`9572`-`9574`, "Wave 1" of `Station 8: Advent`) - hitting an
illusion takes Stacks off the Imago, which is how its state of time changes.  The
Pupa (`9563`) and the Stations 2-4 butterflies (`9564`-`9566`) are loaded because
their data documents the encounter, and `Simulator::section5` can carry a campaign
state between stations, but they are not part of the required deliverable.

The plan document and the five code-review documents it attracted are archived in
[`docs/archive/`](docs/archive/) - every review item they list is resolved; see
[`docs/archive/README.md`](docs/archive/README.md) for the per-document status.

Two rules shape the whole project:

> Unknown mechanics are marked `UNKNOWN` / `NOT_IMPLEMENTED`, never guessed.
> The simulator only simulates; search lives in Python.

## Layout

```
data/          generated data library (identities, ego, enemies, statuses, mechanics)
tools/         data pipeline (fetch -> parse -> extract -> report)
sim/           Rust workspace
  crates/lcb-core   rules: state, clash, damage, statuses, E.G.O, replay, hash
  crates/lcb-cli    inspect / run / JSON-stdio driver
  crates/lcb-py     PyO3 bindings (`lcb_sim`)
python/lcb/    environment wrapper + search (random, greedy, beam)
docs/          MECHANICS.md (rule -> source), STATUS.md, DATA.md, COVERAGE.md
```

The data pipeline is `tools/build_all.py`, which runs, in order:
`build_library.py` (identities / E.G.O / enemies / statuses), `extract_effects.py`
(skill clauses), `extract_passives.py` (identity and enemy Passives),
`extract_panic.py` (the wiki's Panic Types) and `extract_status_effects.py`
(the behaviour written in each status's own text), then `report.py`
(`docs/COVERAGE.md`, `docs/DATA_REPORT.md`).

## Rules layer

Beyond the damage / clash / deck core, the engine runs, all sourced in
`docs/MECHANICS.md`:

| layer | data | what the engine does |
|-------|------|----------------------|
| Skills | `data/mechanics/effects.json` | per-phase clauses (Combat Start, On Use, Before Attack, On Hit, Heads Hit, On Kill, On Evade, Attack End, Turn Start/End), Coin reuse, Attack Weight |
| Passives | `data/passives/passives.json` | Combat / Support Passives at their phases plus continuous damage modifiers |
| Statuses | `data/mechanics/status_effects.json` | each status's Turn Start / Turn End upkeep, continuous modifiers, Clash-end and on-hit riders |
| Sanity | `data/mechanics/panic_types.json` | SP in [-45, 45], Low Morale (-30), Panic / forced E.G.O Corrosion (-45) and each Panic Type's clauses |

Clauses that are not modelled are never dropped: they are listed by
`Simulator::strict_blockers()` (Skills) and `Simulator::unknown_rules_owned()`
/ `passive_gaps()` / `status_gaps()` (everything else), and tabulated in
`docs/COVERAGE.md`.  Skills are strict-gated; Passives and statuses are
reported until their remaining clauses are modelled.

## Build and test

```bash
# 1. data library (cached; only fetches what is missing)
python3 tools/build_all.py --skip-fetch   # or drop the flag to refresh caches

# 2. Rust core + tests (Windows toolchain via WSL, or native cargo)
cd sim && cargo test && cargo run -q -p lcb-cli -- inspect

# 3. Python bindings
cd sim && PYO3_PYTHON=$(python -c "import sys;print(sys.executable)") cargo build -p lcb-py --release
cp target/release/lcb_sim.dll ../python/lcb/lcb_sim.pyd   # .so on Linux/macOS

# 4. play a turn / run the Python smoke tests
python python/demo.py --turns 3 --policy greedy --seed 1
python python/tests/test_env.py
```

`sim/crates/lcb-cli serve` speaks a JSON line protocol (`reset`,
`legal_actions`, `step`, `clone`, `state_hash`, `unknown_rules`,
`strict_blockers`) if the extension cannot be built.

## What the Python side sees

```python
from lcb import LimbusEnv, greedy_turn

env = LimbusEnv()                 # data/ is found automatically
env.reset(seed=1)                 # 7 fixed identities vs the Section 5 Imago
for action in greedy_turn(env):   # each candidate is scored by simulating it
    env.step(action)
env.commit()                      # resolve the turn
env.state_hash()                  # every state is hashable
clone = env.clone_state()         # deep copy, safe to mutate
```

Action space, as in the plan: pick unit → pick skill → pick target → commit.
`legal_actions()` returns the full product, including E.G.O usages the team can
currently afford.

## Sources and honesty

| Source | Use | Status |
|--------|-----|--------|
| wiki.gg (via `r.jina.ai`) | all numbers and effect text | `single_source_verified` |
| in-game localisation dump | official names, ids, status text | `official` |
| zh-CN community pack | Chinese display names only | `auxiliary_translation` |

* `docs/MECHANICS.md` — each implemented rule with its source and status, plus
  the list of rules that are deliberately `UNKNOWN`.
* `docs/STATUS.md` — phase-by-phase status and the verification gaps (no golden
  test from gameplay footage yet).
* `docs/COVERAGE.md` — per-skill counts of modelled vs unmodelled effect lines.

Strict mode (`strict=true` / `--strict`) refuses to run content whose effect
text contains lines the engine does not model, so unimplemented mechanics can
never quietly change a result.

## Licence and attribution

Code is MIT (see `LICENSE`).  Limbus Company and its data are (c) Project Moon;
this project is unaffiliated.  The generated library only exists so the
simulator can run, and each record keeps its provenance in `data/sources/`.
wiki.gg text is CC BY-SA 4.0.  The raw upstream cache (`data/_raw/`, mirrored
game localisation files and wiki pages) is deliberately **not** committed:

```bash
python3 tools/fetch_gamedata.py && python3 tools/fetch_pages.py
```
