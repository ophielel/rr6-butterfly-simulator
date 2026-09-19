"""One-shot build/verify helper.

    python3 tools/build_all.py [--skip-fetch] [--release]

Runs the data pipeline, the Rust test suite, builds the PyO3 extension and
finally plays a demo turn, printing a compact status report.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SIM = os.path.join(ROOT, "sim")


def native_path(path: str, for_windows: bool) -> str:
    """Translate a WSL path when the target interpreter is a Windows binary."""
    if not for_windows or not path.startswith("/mnt/"):
        return path
    try:
        out = subprocess.run(
            ["wslpath", "-w", path], capture_output=True, text=True, check=True
        )
        return out.stdout.strip()
    except Exception:  # noqa: BLE001
        return path


def cargo() -> str:
    exe = shutil.which("cargo")
    if exe:
        return exe
    candidate = os.path.expanduser("~/.cargo/bin/cargo.exe")
    if os.path.exists(candidate):
        return candidate
    candidate = "/mnt/c/Users/24790/.cargo/bin/cargo.exe"
    if os.path.exists(candidate):
        return candidate
    raise SystemExit("cargo not found; install Rust or pass a path")


def run(cmd, cwd=None, env=None, check=True):
    print(f"$ {' '.join(cmd)}")
    result = subprocess.run(cmd, cwd=cwd, env=env)
    if check and result.returncode != 0:
        raise SystemExit(f"command failed: {' '.join(cmd)}")
    return result.returncode


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--skip-fetch", action="store_true")
    parser.add_argument("--release", action="store_true")
    parser.add_argument(
        "--python",
        default=None,
        help="interpreter used for the pipeline and the demo (defaults to sys.executable)",
    )
    args = parser.parse_args()

    py = args.python or sys.executable
    if not args.skip_fetch:
        run([py, os.path.join(HERE, "fetch_gamedata.py")])
        run([py, os.path.join(HERE, "fetch_pages.py")])
    run([py, os.path.join(HERE, "build_library.py")])
    run([py, os.path.join(HERE, "extract_effects.py")])
    run([py, os.path.join(HERE, "extract_passives.py")])
    run([py, os.path.join(HERE, "extract_panic.py")])
    run([py, os.path.join(HERE, "extract_status_effects.py")])
    run([py, os.path.join(HERE, "report.py")])

    c = cargo()
    run([c, "test"], cwd=SIM)
    profile = "--release" if args.release else None
    build = [c, "build", "-p", "lcb-py"]
    if profile:
        build.append(profile)
    env = dict(os.environ)
    env.setdefault("PYO3_PYTHON", py)
    run(build, cwd=SIM, env=env)

    target = "release" if args.release else "debug"
    copied = None
    for name in ("lcb_sim.dll", "liblcb_sim.so", "liblcb_sim.dylib"):
        src = os.path.join(SIM, "target", target, name)
        if os.path.exists(src):
            dst = os.path.join(
                ROOT, "python", "lcb", "lcb_sim.pyd" if name.endswith(".dll") else name
            )
            shutil.copyfile(src, dst)
            copied = dst
            print(f"copied {name} -> {dst}")
            break

    demo_python = py
    if copied and copied.endswith(".pyd") and not py.lower().endswith(".exe"):
        for candidate in ("/mnt/c/Python314/python.exe", r"C:/Python314/python.exe"):
            if os.path.exists(candidate):
                demo_python = candidate
                break
    demo_script = native_path(
        os.path.join(ROOT, "python", "demo.py"), demo_python.lower().endswith(".exe")
    )
    run([demo_python, demo_script, "--turns", "2", "--policy", "greedy"])
    print("\nbuild_all: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
