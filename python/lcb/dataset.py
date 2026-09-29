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


def adapt_action_dim(
    data: Dict[str, np.ndarray],
    target_dim: int,
    legacy_dims: Optional[Sequence[int]] = None,
    effect_dim: int = 0,
    target_slot_dim: int = 0,
) -> Dict[str, np.ndarray]:
    """Pad legacy candidate rows, or reject non-zero feature truncation.

    `legacy_dims` are the action widths an older dataset may have been written
    at, broadest first (e.g. `(44 + 22, 44)` for "effects only" and "nothing").
    A row that came from such a layout has its trailing `effect_dim` and
    `target_slot_dim` columns removed before the new columns are appended, so an
    older dataset keeps meaning what it meant instead of being reinterpreted.
    """
    if "cand" not in data or data["cand"].ndim != 2:
        raise ValueError("dataset is missing a 2-D cand array")
    target_dim = int(target_dim)
    current_dim = int(data["cand"].shape[1])
    if current_dim == target_dim:
        return data
    out = dict(data)
    if current_dim > target_dim:
        discarded = data["cand"][:, target_dim:]
        if discarded.size and not np.allclose(discarded, 0.0):
            raise ValueError(
                f"dataset action_dim={current_dim} has non-zero features; "
                f"cannot load it into action_dim={target_dim}"
            )
        out["cand"] = data["cand"][:, :target_dim]
        return out
    for legacy in legacy_dims or ():
        legacy = int(legacy)
        if current_dim != legacy or legacy >= target_dim:
            continue
        keep = legacy - effect_dim - target_slot_dim
        if keep < 0:
            raise ValueError(f"legacy action_dim={legacy} is smaller than its own blocks")
        rebuilt = np.zeros((data["cand"].shape[0], target_dim), dtype=data["cand"].dtype)
        rebuilt[:, :keep] = data["cand"][:, :keep]
        if effect_dim:
            rebuilt[:, keep : keep + effect_dim] = data["cand"][:, keep : keep + effect_dim]
        out["cand"] = rebuilt
        return out
    padded = np.zeros(
        (data["cand"].shape[0], target_dim), dtype=data["cand"].dtype
    )
    padded[:, :current_dim] = data["cand"]
    out["cand"] = padded
    return out


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
    """Yield `(state, [(candidate_matrix, label, weight), ...])` per decision.

    The candidate rows of a decision are the `offsets[row]` rows that belong to
    that decision's rows, so each row needs its own slice.  Taking them in the
    order the rows appear in the file (instead of walking a running cursor over
    the decision-sorted order) keeps the candidate matrix attached to its own
    label even if a dataset is ever merged or reordered; the current writers
    happen to store rows grouped by ascending decision, which is why the two
    agree today.
    """
    states = data["state"]
    cands = data["cand"]
    offsets = data["offsets"]
    labels = data["label"]
    weights = data["weight"]
    decisions = data["decision"]
    starts = np.concatenate([[0], np.cumsum(offsets)[:-1]]).astype(np.int64)
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
        rows.sort()
        actors: List[Tuple[np.ndarray, int, float]] = []
        for row in rows:
            start = int(starts[row])
            count = int(offsets[row])
            actors.append(
                (
                    cands[start : start + count],
                    int(labels[row]),
                    float(weights[row]),
                )
            )
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
    "adapt_action_dim",
    "subset",
    "iter_decisions",
    "seed_split",
    "describe",
    "Decision",
]
