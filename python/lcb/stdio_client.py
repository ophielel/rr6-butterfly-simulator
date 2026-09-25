"""Fallback transport: drive `lcb-cli serve` over JSON lines.

Used automatically when the PyO3 extension (`lcb_sim`) is not available, for
example when the Rust toolchain cannot produce a `.pyd` for the running
interpreter.  The API mirrors :class:`lcb.env.LimbusEnv`.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path
from typing import Any, Dict, List, Optional


def find_cli() -> str:
    env = os.environ.get("LCB_CLI")
    if env and os.path.exists(env):
        return env
    root = Path(__file__).resolve().parents[2]
    for name in ("lcb-cli.exe", "lcb-cli"):
        for profile in ("release", "debug"):
            candidate = root / "sim" / "target" / profile / name
            if candidate.exists():
                return str(candidate)
    found = shutil.which("lcb-cli")
    if found:
        return found
    raise FileNotFoundError(
        "lcb-cli not found; build it with `cd sim && cargo build -p lcb-cli` "
        "or set LCB_CLI"
    )


def _native(path: str, exe: str) -> str:
    """Translate a WSL path when driving a Windows binary from Linux."""
    if os.name == "nt" or not exe.lower().endswith(".exe"):
        return path
    if not path.startswith("/mnt/"):
        return path
    try:
        out = subprocess.run(
            ["wslpath", "-w", path], capture_output=True, text=True, check=True
        )
        return out.stdout.strip()
    except Exception:  # noqa: BLE001
        return path


class StdioSimulator:
    """Minimal stand-in for `lcb_sim.PySimulator`."""

    def __init__(self, data_dir: Optional[str] = None, exe: Optional[str] = None) -> None:
        self._exe = exe or find_cli()
        args = [self._exe]
        if data_dir:
            args += ["--data", _native(str(data_dir), self._exe)]
        args.append("serve")
        self._proc = subprocess.Popen(
            args,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            errors="replace",
            bufsize=1,
        )

    def _call(self, request: Dict[str, Any]) -> Dict[str, Any]:
        assert self._proc.stdin and self._proc.stdout
        self._proc.stdin.write(json.dumps(request) + "\n")
        self._proc.stdin.flush()
        line = self._proc.stdout.readline()
        if not line:
            raise RuntimeError("lcb-cli terminated")
        response = json.loads(line)
        if response.get("error") is not None:
            raise RuntimeError(response["error"])
        return response

    def reset(
        self,
        seed: int,
        team: Optional[List[str]] = None,
        enemies: Optional[List[str]] = None,
        strict: bool = False,
        max_turns: Optional[int] = None,
        enemy_hp_scale: Optional[float] = None,
        infinite_ego_resources: bool = False,
    ) -> str:
        request: Dict[str, Any] = {
            "cmd": "reset", "seed": seed, "strict": strict,
            "infinite_ego_resources": infinite_ego_resources,
        }
        if max_turns is not None:
            request["max_turns"] = int(max_turns)
        if enemy_hp_scale is not None:
            request["enemy_hp_scale"] = float(enemy_hp_scale)
        if team:
            request["team"] = team
        if enemies:
            request["enemies"] = enemies
        return self._call(request)["state_hash"]

    def legal_actions(self) -> str:
        return json.dumps(self._call({"cmd": "legal_actions"})["actions"])

    def submit(self, action_json: str) -> str:
        request: Dict[str, Any] = {"cmd": "submit", "action": json.loads(action_json)}
        return json.dumps(self._call(request))

    def step(self, action_json: Optional[str]) -> str:
        request: Dict[str, Any] = {"cmd": "step"}
        if action_json is not None and action_json.strip() != "null":
            request["action"] = json.loads(action_json)
        return json.dumps(self._call(request))

    def step_turn(self, plan_json: str) -> str:
        return json.dumps(self._call({"cmd": "step_turn", "plan": json.loads(plan_json)}))

    def state_json(self) -> str:
        return json.dumps(self._call({"cmd": "state"})["state"])

    def state_hash(self) -> str:
        return self._call({"cmd": "state_hash"})["state_hash"]

    def clone_state(self) -> "StdioSimulator":
        snapshot = self._call({"cmd": "state"})["state"]
        clone = StdioSimulator(exe=self._exe)
        clone._call({"cmd": "load_state", "state": snapshot})
        return clone

    def unknown_rules(self) -> List[str]:
        return list(self._call({"cmd": "unknown_rules"})["rules"])

    def strict_blockers(self) -> List[str]:
        return list(self._call({"cmd": "strict_blockers"})["blockers"])

    def close(self) -> None:
        if self._proc.poll() is None:
            self._proc.terminate()


__all__ = ["StdioSimulator", "find_cli"]
