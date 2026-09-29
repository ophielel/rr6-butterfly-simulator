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
        #: Action width the parameters were trained at (= `action_dim` for a new
        #: network, possibly narrower after `load()` zero-padded a checkpoint).
        self.saved_action_dim = self.action_dim

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

    def values(self, states: np.ndarray) -> np.ndarray:
        """Batched `value` over a `(n, state_dim)` matrix of observations.

        `embed` is a plain matmul, so a batch is the same computation without
        the per-row Python overhead; PPO needs one value per turn of every
        rollout and was spending most of its update time in those calls.
        """
        p = self.params
        states = np.atleast_2d(np.asarray(states, dtype=np.float64))
        h = np.tanh(states @ p["W1"] + p["b1"])
        hv = np.tanh(h @ p["Wv"] + p["bv"])
        return hv @ p["wv"]

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
        decisions: Sequence[Tuple[np.ndarray, List[Tuple], float]],
        clip: float = 0.2,
        entropy_coef: float = 0.0,
    ) -> Dict[str, float]:
        """One clipped-surrogate step.

        Each decision is `(state, [(cand, action, old_logprob[, credit])], advantage)`;
        the joint log-probability is the sum over the actors of the turn, so the
        importance ratio is per turn. Optional actor credits only scale that
        actor's policy gradient; the value head is trained separately.

        `entropy_coef` adds the usual entropy bonus on each actor's categorical
        distribution over the legal candidates (`coef * dH/dlogits`), which is
        the only exploration term here: it never reads the state's contents, so
        it cannot favour a particular skill or route.
        """
        grads = self._zero_grads()
        stats = {"loss": 0.0, "ratio": 0.0, "clip_frac": 0.0, "count": 0.0, "entropy": 0.0}
        for state, actors, advantage in decisions:
            fresh = self._zero_grads()
            h = self.embed(state)
            dh = np.zeros_like(h)
            old_total = sum(float(actor[2]) for actor in actors)
            new_total = 0.0
            pieces: List[Tuple[np.ndarray, np.ndarray, np.ndarray, int, float]] = []
            for actor in actors:
                cand, action, _old_logprob = actor[:3]
                credit = float(actor[3]) if len(actor) > 3 else 1.0
                if cand.shape[0] == 0 or action < 0:
                    continue
                joined = np.concatenate(
                    [np.repeat(h[None, :], cand.shape[0], axis=0), cand], axis=1
                )
                z = np.tanh(joined @ self.params["W2"] + self.params["b2"])
                logits = z @ self.params["W3"] + self.params["b3"]
                probs = self._softmax(logits)
                new_total += float(np.log(max(probs[action], 1e-12)))
                stats["entropy"] += float(-np.sum(probs * np.log(np.maximum(probs, 1e-12))))
                pieces.append((joined, z, probs, action, credit))
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
                for joined, z, probs, action, credit in pieces:
                    actor_dlogp = -advantage * credit
                    dlogits = probs.copy()
                    dlogits[action] -= 1.0
                    dlogits *= -actor_dlogp
                    fresh["W3"] += z.T @ dlogits
                    fresh["b3"] += np.array([dlogits.sum()])
                    dz = np.outer(dlogits, self.params["W3"]) * (1.0 - z * z)
                    fresh["W2"] += joined.T @ dz
                    fresh["b2"] += dz.sum(axis=0)
                    djoined = dz @ self.params["W2"].T
                    dh += djoined[:, : self.hidden].sum(axis=0)
            if entropy_coef:
                # dH/dlogits = -p * (log p + H), so the *ascent* direction on the
                # entropy is +coef * p * (log p + H); this is added to the loss
                # gradient that the surrogate already put in `fresh`.
                for joined, z, probs, _action, _credit in pieces:
                    logp = np.log(np.maximum(probs, 1e-12))
                    entropy = float(-np.sum(probs * logp))
                    dlogits = entropy_coef * probs * (logp + entropy)
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
        stats["entropy"] /= stats["count"]
        return stats

    def value_update(
        self,
        decisions: Sequence[Tuple],
        value_coef: float = 1.0,
        clip: float = 0.0,
    ) -> float:
        """Mean-squared-error step for the value head.

        Each entry is `(state, target)` or, when `clip > 0`, `(state, target,
        baseline)` where `baseline` is the prediction captured before the
        update.  With `clip > 0` the target is clipped to `baseline +/- clip`,
        which is the PPO-style trust region for the critic; the gradient also
        vanishes for a sample whose error already exceeds the clip, so the
        returned loss still measures the true error.
        """
        grads = self._zero_grads()
        loss = 0.0
        if all(len(decision) <= 2 for decision in decisions):
            states = np.stack([np.asarray(decision[0], dtype=np.float64) for decision in decisions])
            targets = np.asarray([float(decision[1]) for decision in decisions], dtype=np.float64)
            h = self.embed(states)
            hv = np.tanh(h @ self.params["Wv"] + self.params["bv"])
            predictions = hv @ self.params["wv"]
            errors = predictions - targets
            loss = float(np.sum(errors * errors))
            dhv = (
                np.outer(errors, self.params["wv"]) * (1.0 - hv * hv)
            )
            grads["wv"] += hv.T @ errors
            grads["Wv"] += h.T @ dhv
            grads["bv"] += dhv.sum(axis=0)
            dh = dhv @ self.params["Wv"].T
            grads["W1"] += states.T @ (dh * (1.0 - h * h))
            grads["b1"] += (dh * (1.0 - h * h)).sum(axis=0)
            count = max(1, len(decisions))
            for name in grads:
                grads[name] = value_coef * grads[name] / count
            self.value_adam.step(self.params, grads, self.value_lr, clip=1.0)
            self.updates += 1
            return loss / count
        for decision in decisions:
            state, target = decision[0], float(decision[1])
            baseline = float(decision[2]) if len(decision) > 2 else None
            h = self.embed(state)
            hv = np.tanh(h @ self.params["Wv"] + self.params["bv"])
            prediction = float(hv @ self.params["wv"])
            error = prediction - target
            loss += error * error
            if baseline is not None and clip > 0.0:
                clipped_target = float(
                    np.clip(target, baseline - clip, baseline + clip)
                )
                error = prediction - clipped_target
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
    def load(
        cls, path: str | Path, action_dim: Optional[int] = None
    ) -> "PolicyValueNet":
        with np.load(path) as data:
            params = {name: data[name] for name in data.files}
        state_dim = params["W1"].shape[0]
        hidden = params["W1"].shape[1]
        saved_action_dim = params["W2"].shape[0] - hidden
        target_action_dim = saved_action_dim if action_dim is None else int(action_dim)
        if target_action_dim < saved_action_dim:
            raise ValueError(
                f"checkpoint expects action_dim={saved_action_dim}, got {target_action_dim}"
            )
        value_hidden = params["Wv"].shape[1]
        net = cls(state_dim, target_action_dim, hidden=hidden, value_hidden=value_hidden)
        #: Width the checkpoint was actually trained at, before any zero padding.
        net.saved_action_dim = saved_action_dim
        copied = {k: v.astype(np.float64) for k, v in params.items()}
        if target_action_dim != saved_action_dim:
            old_w2 = copied["W2"]
            new_w2 = np.zeros((hidden + target_action_dim, hidden), dtype=np.float64)
            width = min(saved_action_dim, target_action_dim)
            new_w2[: hidden + width] = old_w2[: hidden + width]
            copied["W2"] = new_w2
        net.params.update(copied)
        return net

    def num_params(self) -> int:
        return int(sum(value.size for value in self.params.values()))

    #: Number of trailing action features that carry the probed effect
    #: fingerprint (`lcb.features.ACTION_EFFECT_DIM`).
    EFFECT_DIM = 22

    def uses_action_effects(self) -> bool:
        """Whether probing candidate effects can change this network's decision.

        The effect features are the last `EFFECT_DIM` columns of the candidate
        matrix, so they reach the logits only through the last `EFFECT_DIM` rows
        of `W2`.  A checkpoint that was trained without them has those rows
        created by `load()`'s zero padding and never updated, so the probes that
        fill the features cannot change a single decision.
        """
        block = self.params["W2"][-self.EFFECT_DIM :, :]
        return bool(np.any(block != 0.0))


__all__ = ["PolicyValueNet", "AdamState", "VALUE_SCALE"]
