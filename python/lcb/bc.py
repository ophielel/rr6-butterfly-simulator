"""Stage B: behaviour cloning from the teacher (TRAINING_PLAN.md §5).

The teacher's plan is a label for *every* actor of the turn (never a macro
action), the loss is only computed over the legal candidates, and the sample
weight comes from the teacher (short wins and burst windows count more, plain and
failed episodes are kept so the policy does not collapse).

Splitting is **by episode seed**: validation seeds are never trained on.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple

import numpy as np

from . import dataset as ds
from .features import Encoder
from .nn import PolicyValueNet


@dataclass
class BCConfig:
    epochs: int = 6
    batch_decisions: int = 32
    learning_rate: float = 3e-3
    hidden: int = 64
    value_hidden: int = 32
    seed: int = 0
    validation_fraction: float = 0.25


@dataclass
class BCHistory:
    train_loss: List[float] = field(default_factory=list)
    val_loss: List[float] = field(default_factory=list)
    val_accuracy: List[float] = field(default_factory=list)
    val_accuracy_weighted: List[float] = field(default_factory=list)


def _accuracy(
    net: PolicyValueNet, decisions: Sequence[ds.Decision], weighted: bool = False
) -> float:
    correct = 0.0
    total = 0.0
    for state, actors in decisions:
        h = net.embed(state)
        for cand, label, weight in actors:
            if cand.shape[0] == 0 or label < 0:
                continue
            logits = net.logits(h, cand)
            pick = int(np.argmax(logits))
            amount = weight if weighted else 1.0
            correct += amount if pick == label else 0.0
            total += amount
    return correct / total if total else 0.0


def train_bc(
    data: Dict[str, np.ndarray],
    encoder: Encoder,
    config: Optional[BCConfig] = None,
    log: Optional[List[str]] = None,
) -> Tuple[PolicyValueNet, BCHistory, Dict[str, Any]]:
    """Train the policy on teacher labels; returns (net, history, info)."""
    config = config or BCConfig()
    train_data, val_data = ds.seed_split(data, config.validation_fraction, config.seed)
    train_decisions = list(ds.iter_decisions(train_data))
    val_decisions = list(ds.iter_decisions(val_data))
    net = PolicyValueNet(
        encoder.state_dim,
        encoder.action_dim,
        hidden=config.hidden,
        value_hidden=config.value_hidden,
        seed=config.seed,
        lr=config.learning_rate,
    )
    history = BCHistory()
    rng = np.random.default_rng(config.seed)
    for epoch in range(config.epochs):
        order = rng.permutation(len(train_decisions))
        losses: List[float] = []
        for start in range(0, len(order), config.batch_decisions):
            batch = [train_decisions[i] for i in order[start : start + config.batch_decisions]]
            losses.append(net.bc_update(batch))
        history.train_loss.append(float(np.mean(losses)) if losses else 0.0)
        history.val_loss.append(_loss(net, val_decisions) if val_decisions else 0.0)
        history.val_accuracy.append(_accuracy(net, val_decisions))
        history.val_accuracy_weighted.append(_accuracy(net, val_decisions, weighted=True))
        if log is not None:
            log.append(
                f"epoch {epoch + 1}/{config.epochs}: train_loss={history.train_loss[-1]:.4f} "
                f"val_loss={history.val_loss[-1]:.4f} acc={history.val_accuracy[-1]:.3f}"
            )
    info = {
        "train": ds.describe(train_data),
        "validation": ds.describe(val_data),
        "val_accuracy": history.val_accuracy[-1] if history.val_accuracy else 0.0,
        "params": net.num_params(),
    }
    return net, history, info


def _loss(net: PolicyValueNet, decisions: Sequence[ds.Decision]) -> float:
    total = 0.0
    count = 0
    for state, actors in decisions:
        h = net.embed(state)
        for cand, label, weight in actors:
            if cand.shape[0] == 0 or label < 0:
                continue
            probs = net._softmax(net.logits(h, cand))
            total += -float(np.log(max(probs[label], 1e-12))) * weight
            count += 1
    return total / count if count else 0.0


__all__ = ["BCConfig", "BCHistory", "train_bc"]
