#!/usr/bin/env python3
"""Self-test for scripts/headless-runner.sh.

The runner is cargo's `runner`, so it fronts EVERY binary cargo executes --
`cargo run -p postio-app` included. It exists to put *test* binaries on a
hidden compositor; the application itself must reach the real display, or the
README's own run instruction launches Postio invisibly (#315).

Cargo names test and bench binaries with a 16-hex metadata suffix
(target/debug/deps/gtk_list-0123456789abcdef); `cargo run` binaries and
examples carry their plain names. That suffix is what the runner keys on:

  * a hash-suffixed binary runs with WAYLAND_DISPLAY pointed at the private
    compositor;
  * anything else is exec'd unchanged -- no compositor started, environment
    untouched;
  * POSTIO_HEADLESS=0 bypasses everything, as before.

mutter is stubbed on PATH with a script that binds the Wayland socket the
runner waits for, so this runs anywhere, fast, without a real compositor.

Usage: scripts/tests/test-headless-runner.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import signal
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
RUNNER = HERE / "headless-runner.sh"

DISPLAY = "postio-runner-selftest"

MUTTER_STUB = """#!/usr/bin/env bash
# Pretend to be mutter: record the call, bind the socket the runner waits
# for, and linger. The test kills us by the recorded pid.
echo "$$" >> "$STUB_DIR/mutter.pids"
exec python3 - "$XDG_RUNTIME_DIR/$POSTIO_TEST_DISPLAY" <<'PY'
import socket, sys, time
s = socket.socket(socket.AF_UNIX)
s.bind(sys.argv[1])
time.sleep(60)
PY
"""

TARGET_STUB = """#!/usr/bin/env bash
printf '%s' "${WAYLAND_DISPLAY:-<unset>}"
"""

FAILURES: list[str] = []


def run(binary: Path, stub_dir: Path, runtime_dir: Path, **env_extra: str) -> str:
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["XDG_RUNTIME_DIR"] = str(runtime_dir)
    environment["POSTIO_TEST_DISPLAY"] = DISPLAY
    environment.pop("WAYLAND_DISPLAY", None)
    environment.pop("POSTIO_HEADLESS", None)
    environment.update(env_extra)
    result = subprocess.run(
        ["bash", str(RUNNER), str(binary)],
        env=environment,
        capture_output=True,
        text=True,
        timeout=30,
    )
    return result.stdout


def mutter_calls(stub_dir: Path) -> int:
    try:
        return len((stub_dir / "mutter.pids").read_text().split())
    except FileNotFoundError:
        return 0


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)
        stub_dir = base / "stubs"
        runtime_dir = base / "runtime"
        stub_dir.mkdir()
        runtime_dir.mkdir()

        (stub_dir / "mutter").write_text(MUTTER_STUB)
        (stub_dir / "mutter").chmod(0o755)

        app = stub_dir / "postio-app"
        app.write_text(TARGET_STUB)
        app.chmod(0o755)
        test_bin = stub_dir / "gtk_list-0123456789abcdef"
        test_bin.write_text(TARGET_STUB)
        test_bin.chmod(0o755)
        near_miss = stub_dir / "tool-abcdefgh12345678"  # 16 chars, not all hex
        near_miss.write_text(TARGET_STUB)
        near_miss.chmod(0o755)

        try:
            out = run(app, stub_dir, runtime_dir)
            case(
                "a plain-named binary keeps the real display",
                out == "<unset>",
                f"WAYLAND_DISPLAY was {out!r}; the app would run invisibly",
            )
            case(
                "a plain-named binary starts no compositor",
                mutter_calls(stub_dir) == 0,
                "mutter was launched for a non-test binary",
            )

            out = run(near_miss, stub_dir, runtime_dir)
            case(
                "a dash-suffix that is not a hash keeps the real display",
                out == "<unset>",
                f"WAYLAND_DISPLAY was {out!r}",
            )

            out = run(test_bin, stub_dir, runtime_dir)
            case(
                "a hash-suffixed test binary goes to the compositor",
                out == DISPLAY,
                f"WAYLAND_DISPLAY was {out!r}, expected {DISPLAY!r}",
            )
            case(
                "the compositor was started for it",
                mutter_calls(stub_dir) == 1,
                "mutter was never launched for a test binary",
            )

            out = run(test_bin, stub_dir, runtime_dir, POSTIO_HEADLESS="0")
            case(
                "POSTIO_HEADLESS=0 still bypasses everything",
                out == "<unset>",
                f"WAYLAND_DISPLAY was {out!r}",
            )
        finally:
            pids = base / "stubs" / "mutter.pids"
            if pids.exists():
                for pid in pids.read_text().split():
                    try:
                        os.killpg(int(pid), signal.SIGKILL)
                    except (OSError, ValueError):
                        try:
                            os.kill(int(pid), signal.SIGKILL)
                        except (OSError, ValueError):
                            pass

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("headless-runner check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
