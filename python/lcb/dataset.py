"""Teacher datasets: storage and iteration (TRAINING_PLAN.md §4.4, §5).

The teacher writes `.npz` files with one row per **actor decision**:

| array | meaning |
|-------|---------|
| `state` | encoded observation of the turn |
| `cand` | candidate action features, concatenated |
| `offsets` | number of candidates per row |
| `label` | index of the teacher's action in the candidate list |
| `weight` | sample weight (§5) |
| `actor` | position of the actor inside the turn (0 = first to choose) |
| `seed` | episode seed; splits are **by seed**, never by neighbouring rows |
| `turn` | simulator turn at which the decision was made |
| `decision` | which turn-decision the row belongs to |

`iter_decisions` turns the flat arrays back into the nested structure the policy
network consumes.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any, Dict, Iterable, Iterator, List, Optional, Sequence, Tuple

import numpy as np

Decision = Tuple[np.ndarray, List[Tuple[np.ndarray, int, float]]]


def save_dataset(path: str | Path, arrays: Dict[str, np.ndarray]) -> None:
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    np.savez_compressed(path, **arrays)


def load_dataset(path: str | Path) -> Dict[str, np.ndarray]:
    with np.load(path) as data:
        return {name: data[name] for name in data.files}


def merge(datasets: Sequence[Dict[str, np.ndarray]]) -> Dict[str, np.ndarray]:
    if not datasets:
        raise ValueError("nothing to merge")
    if len(datasets) == 1:
        return datasets[0]
    out: Dict[str, np.ndarray] = {}
    for name in datasets[0]:
        if name == "decision":
            pieces = []
            offset = 0
            for data in datasets:
                pieces.append(data[name] + offset)
                offset += int(data[name].max()) + 1 if data[name].size else 0
            out[name] = np.concatenate(pieces)
            continue
        out[name] = np.concatenate([data[name] for data in datasets])
    return out


def subset(data: Dict[str, np.ndarray], seeds: Iterable[int] | None = None) -> Dict[str, np.ndarray]:
    """Keep only the rows whose episode seed is in `seeds` (split by seed).

    `cand` holds one row per *candidate* rather than per decision, so it is
    filtered through `offsets`; `decision` is re-indexed so the kept decisions
    stay contiguous.
    """
    if seeds is None:
        return data
    wanted = np.asarray(sorted(set(int(s) for s in seeds)), dtype=np.int32)
    row_mask = np.isin(data["seed"], wanted)
    out: Dict[str, np.ndarray] = {}
    offsets = data["offsets"]
    for name, value in data.items():
        if name == "cand":
            keep: List[np.ndarray] = []
            for row, count in enumerate(offsets):
                keep.append(np.full(int(count), bool(row_mask[row]), dtype=bool))
            out[name] = value[np.concatenate(keep)] if keep else value[:0]
        elif name == "decision":
            kept = data["decision"][row_mask]
            unique = {int(d): i for i, d in enumerate(np.unique(kept))}
            out[name] = np.asarray([unique[int(d)] for d in kept], dtype=np.int32)
        else:
            out[name] = value[row_mask]
    return out


def iter_decisions(data: Dict[str, np.ndarray]) -> Iterator[Decision]:
    """Yield `(state, [(candidate_matrix, label, weight), ...])` per decision."""
    states = data["state"]
    cands = data["cand"]
    offsets = data["offsets"]
    labels = data["label"]
    weights = data["weight"]
    decisions = data["decision"]
    cursor = 0
    order = np.argsort(decisions, kind="stable")
    position = 0
    while position < len(order):
        decision = decisions[order[position]]
        rows: List[int] = []
        while position < len(order) and decisions[order[position]] == decision:
            rows.append(int(order[position]))
            position += 1
        if not rows:
            continue
        actors: List[Tuple[np.ndarray, int, float]] = []
        for row in rows:
            count = int(offsets[row])
            actors.append(
                (
                    cands[cursor : cursor + count],
                    int(labels[row]),
                    float(weights[row]),
                )
            )
            cursor += count
        yield states[rows[0]], actors


def seed_split(
    data: Dict[str, np.ndarray], validation_fraction: float = 0.25, seed: int = 0
) -> Tuple[Dict[str, np.ndarray], Dict[str, np.ndarray]]:
    """Split by episode seed (never by row), as §5 requires."""
    seeds = np.unique(data["seed"])
    rng = np.random.default_rng(seed)
    shuffled = rng.permutation(seeds)
    cut = max(1, int(round(len(shuffled) * validation_fraction)))
    val_seeds = set(int(s) for s in shuffled[:cut])
    train_seeds = set(int(s) for s in shuffled[cut:])
    return subset(data, train_seeds), subset(data, val_seeds)


def describe(data: Dict[str, np.ndarray]) -> Dict[str, Any]:
    return {
        "rows": int(len(data.get("label", []))),
        "decisions": int(len(np.unique(data["decision"]))) if data.get("decision") is not None else 0,
        "episodes": int(len(np.unique(data["seed"]))) if data.get("seed") is not None else 0,
        "candidates": int(data["offsets"].sum()) if data.get("offsets") is not None else 0,
        "mean_weight": float(np.mean(data["weight"])) if data.get("weight") is not None else 0.0,
    }


__all__ = [
    "save_dataset",
    "load_dataset",
    "merge",
    "subset",
    "iter_decisions",
    "seed_split",
    "describe",
    "Decision",
]
