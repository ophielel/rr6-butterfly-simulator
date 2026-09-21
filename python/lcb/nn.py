"""A small policy/value network with hand-written gradients (NumPy only).

The project has no PyTorch dependency, so the model is deliberately small and
explicit:

```text
h        = tanh(s W1 + b1)                      shared state embedding
z_i      = tanh([h, x_i] W2 + b2)               per candidate action
logits_i = z_i . W3 + b3                        score of candidate i
v        = wv . tanh(h Wv + bv)                 state value
```

`x_i` already carries which actor is choosing and what the earlier actors of the
turn picked (`lcb.features`), so one shared scorer produces the plan
autoregressively over the fixed actor order - exactly the "shared state encoder
plus per-actor head" the plan allows, without one head per Sinner.

Gradients are the textbook ones (softmax cross-entropy for BC, the clipped
surrogate for PPO, mean-squared error for the value head); Adam is used for the
update.  Everything is float64 in the optimiser to keep the small model stable.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Dict, List, Optional, Sequence, Tuple

import numpy as np

#: Returns are divided by this before the value head sees them.
VALUE_SCALE = 1000.0


def _xavier(rng: np.random.Generator, fan_in: int, fan_out: int) -> np.ndarray:
    limit = np.sqrt(6.0 / (fan_in + fan_out))
    return rng.uniform(-limit, limit, size=(fan_in, fan_out))


@dataclass
class AdamState:
    m: Dict[str, np.ndarray] = field(default_factory=dict)
    v: Dict[str, np.ndarray] = field(default_factory=dict)
    t: int = 0

    def step(
        self,
        params: Dict[str, np.ndarray],
        grads: Dict[str, np.ndarray],
        lr: float,
        clip: float = 0.0,
    ) -> None:
        self.t += 1
        beta1, beta2, eps = 0.9, 0.999, 1e-8
        if clip > 0.0:
            # Global-norm clipping: the value targets are in reward units
            # (±1000 for a win), so an unclipped step can blow the head up.
            total = float(np.sqrt(sum(float((g * g).sum()) for g in grads.values())))
            if total > clip:
                scale = clip / total
                grads = {name: grad * scale for name, grad in grads.items()}
        for name, grad in grads.items():
            if name not in params:
                continue
            m = self.m.setdefault(name, np.zeros_like(params[name]))
            v = self.v.setdefault(name, np.zeros_like(params[name]))
            m *= beta1
            m += (1 - beta1) * grad
            v *= beta2
            v += (1 - beta2) * (grad * grad)
            m_hat = m / (1 - beta1**self.t)
            v_hat = v / (1 - beta2**self.t)
            params[name] -= lr * m_hat / (np.sqrt(v_hat) + eps)


class PolicyValueNet:
    """Candidate-scoring policy with a value head (see the module docstring)."""

    def __init__(
        self,
        state_dim: int,
        action_dim: int,
        hidden: int = 64,
        value_hidden: int = 32,
        seed: int = 0,
        lr: float = 3e-3,
    ) -> None:
        self.state_dim = int(state_dim)
        self.action_dim = int(action_dim)
        self.hidden = int(hidden)
        self.value_hidden = int(value_hidden)
        self.lr = float(lr)
        rng = np.random.default_rng(seed)
        self.params: Dict[str, np.ndarray] = {
            "W1": _xavier(rng, self.state_dim, hidden),
            "b1": np.zeros(hidden),
            "W2": _xavier(rng, hidden + self.action_dim, hidden),
            "b2": np.zeros(hidden),
            "W3": _xavier(rng, hidden, 1).ravel(),
            "b3": np.zeros(1),
            "Wv": _xavier(rng, hidden, value_hidden),
            "bv": np.zeros(value_hidden),
            "wv": _xavier(rng, value_hidden, 1).ravel(),
        }
        self.params = {k: v.astype(np.float64) for k, v in self.params.items()}
        self.adam = AdamState()
        self.value_adam = AdamState()
        #: The value head is trained on **normalised** returns (see
        #: `lcb.ppo.VALUE_SCALE`); raw rewards span ±1000 and would need ~1000x
        #: the step size to be fitted by a small Adam model.
        self.value_lr = lr
        self.updates = 0

    # -- forward -----------------------------------------------------------
    def embed(self, state: np.ndarray) -> np.ndarray:
        p = self.params
        return np.tanh(state @ p["W1"] + p["b1"])

    def logits(self, h: np.ndarray, cand: np.ndarray) -> np.ndarray:
        p = self.params
        if cand.shape[0] == 0:
            return np.zeros(0)
        joined = np.concatenate([np.repeat(h[None, :], cand.shape[0], axis=0), cand], axis=1)
        z = np.tanh(joined @ p["W2"] + p["b2"])
        return z @ p["W3"] + p["b3"]

    def value(self, state: np.ndarray) -> float:
        p = self.params
        h = self.embed(state)
        hv = np.tanh(h @ p["Wv"] + p["bv"])
        return float(hv @ p["wv"])

    @staticmethod
    def _softmax(logits: np.ndarray) -> np.ndarray:
        if logits.size == 0:
            return logits
        shifted = logits - logits.max()
        exp = np.exp(shifted)
        return exp / exp.sum()

    def probs(self, state: np.ndarray, cand: np.ndarray) -> np.ndarray:
        return self._softmax(self.logits(self.embed(state), cand))

    def argmax(self, state: np.ndarray, cand: np.ndarray) -> int:
        logits = self.logits(self.embed(state), cand)
        return int(np.argmax(logits)) if logits.size else -1

    def sample(
        self, state: np.ndarray, cand: np.ndarray, rng: np.random.Generator
    ) -> Tuple[int, float]:
        probs = self.probs(state, cand)
        if probs.size == 0:
            return -1, 0.0
        index = int(rng.choice(len(probs), p=probs))
        return index, float(np.log(max(probs[index], 1e-12)))

    # -- training ----------------------------------------------------------
    def _zero_grads(self) -> Dict[str, np.ndarray]:
        return {name: np.zeros_like(value) for name, value in self.params.items()}

    def bc_update(
        self, decisions: Sequence[Tuple[np.ndarray, List[Tuple[np.ndarray, int, float]]]]
    ) -> float:
        """One cross-entropy step.  Each decision is `(state, [(cand, label, weight), ...])`."""
        grads = self._zero_grads()
        total_loss = 0.0
        total_weight = 0.0
        for state, actors in decisions:
            h = self.embed(state)
            dh = np.zeros_like(h)
            for cand, label, weight in actors:
                if cand.shape[0] == 0 or label < 0:
                    continue
                joined = np.concatenate(
                    [np.repeat(h[None, :], cand.shape[0], axis=0), cand], axis=1
                )
                z = np.tanh(joined @ self.params["W2"] + self.params["b2"])
                logits = z @ self.params["W3"] + self.params["b3"]
                probs = self._softmax(logits)
                total_loss += -float(np.log(max(probs[label], 1e-12))) * weight
                total_weight += weight
                dlogits = probs.copy()
                dlogits[label] -= 1.0
                dlogits *= weight
                grads["W3"] += z.T @ dlogits
                grads["b3"] += np.array([dlogits.sum()])
                dz = np.outer(dlogits, self.params["W3"]) * (1.0 - z * z)
                grads["W2"] += joined.T @ dz
                grads["b2"] += dz.sum(axis=0)
                djoined = dz @ self.params["W2"].T
                dh += djoined[:, : self.hidden].sum(axis=0)
            grads["W1"] += np.outer(state, dh * (1.0 - h * h))
            grads["b1"] += dh * (1.0 - h * h)
        if total_weight == 0:
            return 0.0
        for name in grads:
            grads[name] /= total_weight
        self.adam.step(self.params, grads, self.lr)
        self.updates += 1
        return total_loss / total_weight

    def ppo_update(
        self,
        decisions: Sequence[Tuple[np.ndarray, List[Tuple[np.ndarray, int, float]], float]],
        clip: float = 0.2,
    ) -> Dict[str, float]:
        """One clipped-surrogate step.

        Each decision is `(state, [(cand, action, old_logprob), ...], advantage)`;
        the joint log-probability is the sum over the actors of the turn, so the
        importance ratio is per *turn*, as the plan requires.  The value head is
        trained separately (`value_update`) so that the two objectives cannot
        fight over the shared state embedding.
        """
        grads = self._zero_grads()
        stats = {"loss": 0.0, "ratio": 0.0, "clip_frac": 0.0, "count": 0.0}
        for state, actors, advantage in decisions:
            fresh = self._zero_grads()
            h = self.embed(state)
            dh = np.zeros_like(h)
            old_total = sum(old for _, _, old in actors)
            new_total = 0.0
            pieces: List[Tuple[np.ndarray, np.ndarray, np.ndarray, int]] = []
            for cand, action, _ in actors:
                if cand.shape[0] == 0 or action < 0:
                    continue
                joined = np.concatenate(
                    [np.repeat(h[None, :], cand.shape[0], axis=0), cand], axis=1
                )
                z = np.tanh(joined @ self.params["W2"] + self.params["b2"])
                logits = z @ self.params["W3"] + self.params["b3"]
                probs = self._softmax(logits)
                new_total += float(np.log(max(probs[action], 1e-12)))
                pieces.append((joined, z, probs, action))
            if not pieces:
                continue
            ratio = float(np.exp(min(max(new_total - old_total, -20.0), 20.0)))
            unclipped = ratio * advantage
            clipped = float(np.clip(ratio, 1 - clip, 1 + clip)) * advantage
            # The surrogate is -min(unclipped, clipped); the gradient only flows
            # when the unclipped term is the active one.
            active = unclipped <= clipped
            stats["ratio"] += ratio
            stats["clip_frac"] += 0.0 if active else 1.0
            stats["count"] += 1.0
            stats["loss"] += -min(unclipped, clipped)
            if active and abs(advantage) > 1e-9:
                dlogp = -advantage
                for joined, z, probs, action in pieces:
                    dlogits = probs.copy()
                    dlogits[action] -= 1.0
                    dlogits *= -dlogp
                    fresh["W3"] += z.T @ dlogits
                    fresh["b3"] += np.array([dlogits.sum()])
                    dz = np.outer(dlogits, self.params["W3"]) * (1.0 - z * z)
                    fresh["W2"] += joined.T @ dz
                    fresh["b2"] += dz.sum(axis=0)
                    djoined = dz @ self.params["W2"].T
                    dh += djoined[:, : self.hidden].sum(axis=0)
            fresh["W1"] += np.outer(state, dh * (1.0 - h * h))
            fresh["b1"] += dh * (1.0 - h * h)
            for name in fresh:
                grads[name] += fresh[name]
        if stats["count"] == 0:
            return stats
        for name in grads:
            grads[name] /= stats["count"]
        self.adam.step(self.params, grads, self.lr)
        self.updates += 1
        stats["ratio"] /= stats["count"]
        stats["clip_frac"] /= stats["count"]
        stats["loss"] /= stats["count"]
        return stats

    def value_update(
        self,
        decisions: Sequence[Tuple[np.ndarray, float]],
        value_coef: float = 1.0,
    ) -> float:
        """Mean-squared-error step for the value head."""
        grads = self._zero_grads()
        loss = 0.0
        for state, target in decisions:
            h = self.embed(state)
            hv = np.tanh(h @ self.params["Wv"] + self.params["bv"])
            prediction = float(hv @ self.params["wv"])
            error = prediction - target
            loss += error * error
            grads["wv"] += error * hv
            dhv = np.outer(np.array([error]), self.params["wv"]) * (1.0 - hv * hv)
            grads["Wv"] += np.outer(h, dhv.ravel())
            grads["bv"] += dhv.ravel()
            dh = dhv @ self.params["Wv"].T
            grads["W1"] += np.outer(state, dh.ravel() * (1.0 - h * h))
            grads["b1"] += dh.ravel() * (1.0 - h * h)
        count = max(1, len(decisions))
        for name in grads:
            grads[name] = value_coef * grads[name] / count
        self.value_adam.step(self.params, grads, self.value_lr, clip=1.0)
        self.updates += 1
        return loss / count

    # -- persistence -------------------------------------------------------
    def save(self, path: str | Path) -> None:
        path = Path(path)
        path.parent.mkdir(parents=True, exist_ok=True)
        np.savez(path, **self.params)

    @classmethod
    def load(cls, path: str | Path) -> "PolicyValueNet":
        with np.load(path) as data:
            params = {name: data[name] for name in data.files}
        state_dim = params["W1"].shape[0]
        hidden = params["W1"].shape[1]
        action_dim = params["W2"].shape[0] - hidden
        value_hidden = params["Wv"].shape[1]
        net = cls(state_dim, action_dim, hidden=hidden, value_hidden=value_hidden)
        net.params.update({k: v.astype(np.float64) for k, v in params.items()})
        return net

    def num_params(self) -> int:
        return int(sum(value.size for value in self.params.values()))


__all__ = ["PolicyValueNet", "AdamState", "VALUE_SCALE"]
