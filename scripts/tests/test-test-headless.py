#!/usr/bin/env python3
"""Self-test for scripts/test-headless.sh's own compositors (#2248).

A storyboard or determinism runner gets a compositor of its own, named for
its pid. Those outlived their runners by the hundred, each holding an inotify
instance, until every file-watching test failed with "Too many open files".

  * `--own <command>`: the compositor ends with the command, whether it exits
    0, exits 1, or is killed outright (a panic's abort, a tool's time cap);
  * `--sweep`: a `postio-storyboard-<pid>` or `postio-determinism-<pid>`
    compositor whose pid is gone is stopped and its files removed; one whose
    pid lives, and the shared `postio-headless*`, are left alone.

mutter is stubbed on PATH with a Python script that names itself `mutter`
and binds the socket the script waits for.

Usage: scripts/tests/test-test-headless.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import signal
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above
import prereq  # noqa: E402

prereq.only_on("linux", reason="mutter, setpriv and /proc are Linux's")

SCRIPT = Path(__file__).resolve().parent.parent / "test-headless.sh"
SANDBOXES = Path(__file__).resolve().parents[2] / "target" / "tmp"

MUTTER_STUB = """#!/usr/bin/env python3
import ctypes, os, socket, sys, time
ctypes.CDLL(None).prctl(15, b"mutter")  # PR_SET_NAME, so `pgrep -x mutter` sees it
name = next(a.split("=", 1)[1] for a in sys.argv if a.startswith("--wayland-display="))
with open(os.path.join(os.environ["STUB_DIR"], "mutter.pids"), "a") as f:
    f.write(f"{os.getpid()} {name}\\n")
s = socket.socket(socket.AF_UNIX)
s.bind(os.path.join(os.environ["XDG_RUNTIME_DIR"], name))
time.sleep(120)
"""

FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    print(f"{'ok  ' if condition else 'FAIL'}  {name}")
    if not condition:
        FAILURES.append(f"{name}: {detail}")


def alive(pid: int) -> bool:
    try:
        state = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0]
    except (FileNotFoundError, IndexError):
        return False
    return state != "Z"


def gone_within(pid: int, seconds: float) -> bool:
    end = time.monotonic() + seconds * patience.patience()
    while time.monotonic() < end:
        if not alive(pid):
            return True
        time.sleep(0.05)
    return False


def stubs(stub_dir: Path) -> dict[str, int]:
    try:
        lines = (stub_dir / "mutter.pids").read_text().splitlines()
    except FileNotFoundError:
        return {}
    return {name: int(pid) for pid, name in (line.split() for line in lines)}


def environment(stub_dir: Path, runtime: Path, display: str) -> dict[str, str]:
    env = dict(os.environ)
    env.update(
        PATH=f"{stub_dir}:{env['PATH']}",
        STUB_DIR=str(stub_dir),
        XDG_RUNTIME_DIR=str(runtime),
        POSTIO_TEST_DISPLAY=display,
    )
    return env


def dead_pid() -> int:
    child = subprocess.Popen(["true"])
    child.wait()
    return child.pid


def main() -> int:
    SANDBOXES.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as raw:
        base = Path(raw)
        stub_dir = base / "stubs"
        stub_dir.mkdir()
        (stub_dir / "mutter").write_text(MUTTER_STUB)
        (stub_dir / "mutter").chmod(0o755)
        runtime = base / "run"
        runtime.mkdir()
        spawned: list[int] = []

        try:
            for label, command in (
                ("exits 0", "exit 0"),
                ("exits 1", "exit 1"),
                ("is killed outright", "kill -9 $$"),
            ):
                display = f"postio-storyboard-{os.getpid()}-{len(spawned)}"
                patience.run(
                    ["bash", str(SCRIPT), "--own", "bash", "-c", command],
                    env=environment(stub_dir, runtime, display),
                    capture_output=True,
                    timeout=30,
                )
                pid = stubs(stub_dir).get(display)
                case(f"own-started: a command that {label}", pid is not None,
                     f"no compositor was started for {display}")
                if pid is None:
                    continue
                spawned.append(pid)
                case(f"own-reaped: a command that {label} takes its compositor with it",
                     gone_within(pid, 5), f"mutter {pid} outlived its command")

            # The compositors a sweep must judge, each started as the shared ones are.
            live = os.getpid()
            dead = dead_pid()
            names = {
                "orphan": f"postio-storyboard-{dead}",
                "orphan shard": f"postio-storyboard-{dead}-2",
                "orphan determinism": f"postio-determinism-{dead}-1",
                "living runner": f"postio-storyboard-{live}-9",
                "living determinism": f"postio-determinism-{live}",
                "shared": "postio-headless",
                "shared slot": f"postio-headless-{dead}",
            }
            for display in names.values():
                subprocess.Popen(
                    ["mutter", "--headless", f"--wayland-display={display}"],
                    env=environment(stub_dir, runtime, display),
                    start_new_session=True,
                )
                (runtime / f"{display}.log").write_text("")
            end = time.monotonic() + 10 * patience.patience()
            while time.monotonic() < end and not all(
                (runtime / d).exists() for d in names.values()
            ):
                time.sleep(0.05)
            pids = {label: stubs(stub_dir)[display] for label, display in names.items()}
            spawned.extend(pids.values())

            result = patience.run(
                ["bash", str(SCRIPT), "--sweep"],
                env=environment(stub_dir, runtime, "postio-headless"),
                capture_output=True,
                text=True,
                timeout=30,
            )
            case("sweep-exits", result.returncode == 0,
                 f"exit {result.returncode}: {result.stderr.strip()}")
            for label in ("orphan", "orphan shard", "orphan determinism"):
                case(f"sweep-stops: {label}", gone_within(pids[label], 5),
                     f"{names[label]} outlived its runner's sweep")
                left = sorted(p.name for p in runtime.glob(f"{names[label]}*"))
                case(f"sweep-cleans: {label}", not left, f"left behind: {left}")
            for label in ("living runner", "living determinism", "shared", "shared slot"):
                case(f"sweep-spares: {label}", alive(pids[label]),
                     f"{names[label]} was stopped")
                case(f"sweep-keeps-files: {label}", (runtime / f"{names[label]}.log").exists(),
                     f"{names[label]}'s log was removed")
        finally:
            for pid in spawned:
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

    if FAILURES:
        print("\nFAILED:", file=sys.stderr)
        for failure in FAILURES:
            print(f"  {failure}", file=sys.stderr)
        return 1
    print("all cases behaved")
    return 0


if __name__ == "__main__":
    sys.exit(main())
