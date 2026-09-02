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
import socket
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


def run_full(
    binary: Path, stub_dir: Path, runtime_dir: Path, **env_extra: str
) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["XDG_RUNTIME_DIR"] = str(runtime_dir)
    environment["POSTIO_TEST_DISPLAY"] = DISPLAY
    environment.pop("WAYLAND_DISPLAY", None)
    environment.pop("POSTIO_HEADLESS", None)
    environment.update(env_extra)
    return subprocess.run(
        ["bash", str(RUNNER), str(binary)],
        env=environment,
        capture_output=True,
        text=True,
        timeout=30,
    )


def run(binary: Path, stub_dir: Path, runtime_dir: Path, **env_extra: str) -> str:
    return run_full(binary, stub_dir, runtime_dir, **env_extra).stdout


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

            # ── a socket with nothing behind it (#794) ────────────────────
            #
            # mutter can bind the socket and then exit -- on a machine with no
            # DRM device, which is every GitHub-hosted runner. `-S` still
            # passes, and committing to that display is worse than never
            # starting one: the runner unsets DISPLAY and forces
            # GDK_BACKEND=wayland, so the *working* fallback display is thrown
            # away too and every GTK test skips itself for want of a display
            # it actually had. #781 spent a day on that symptom.
            #
            # A fresh runtime dir, so the marker and socket from the cases
            # above cannot decide this one.
            stale_runtime = base / "stale"
            stale_runtime.mkdir()
            stale = stale_runtime / DISPLAY
            sock = socket.socket(socket.AF_UNIX)
            sock.bind(str(stale))
            sock.close()  # a socket file with no listener behind it
            out = run(test_bin, stub_dir, stale_runtime)
            case(
                "a socket with no compositor behind it falls back",
                out == "<unset>",
                f"WAYLAND_DISPLAY was {out!r}: the runner committed to a dead "
                "compositor and took the session's display with it",
            )
            case(
                "and says so once, so the next binary does not re-pay for it",
                (stale_runtime / f"{DISPLAY}.unavailable").exists(),
                "no marker was left; twenty binaries would each wait for the "
                "compositor to fail to start",
            )

            # ── the marker is a shortcut, not a verdict (#830) ─────────────
            #
            # It saves the next twenty binaries from re-learning that the
            # compositor will not start. But `XDG_RUNTIME_DIR` outlives a
            # test run, and nothing ever removed this file -- so one
            # transient failure silently demoted every later run on that
            # machine to the session's display for the rest of the login
            # session, throwing test windows at whoever was at the keyboard.
            # Observed after the nested compositor exited nine hours into a
            # run. It has to go stale.
            fresh_runtime = base / "fresh-marker"
            fresh_runtime.mkdir()
            (fresh_runtime / f"{DISPLAY}.unavailable").touch()
            before = mutter_calls(stub_dir)
            out = run(test_bin, stub_dir, fresh_runtime)
            case(
                "a fresh marker still short-circuits the start",
                out == "<unset>" and mutter_calls(stub_dir) == before,
                f"WAYLAND_DISPLAY was {out!r} and mutter was called "
                f"{mutter_calls(stub_dir) - before} time(s); the marker bought "
                "nothing",
            )

            stale_marker_runtime = base / "stale-marker"
            stale_marker_runtime.mkdir()
            marker = stale_marker_runtime / f"{DISPLAY}.unavailable"
            marker.touch()
            old = marker.stat().st_mtime - 3600
            os.utime(marker, (old, old))
            before = mutter_calls(stub_dir)
            out = run(test_bin, stub_dir, stale_marker_runtime)
            case(
                "an hour-old marker does not decide this run",
                out == DISPLAY and mutter_calls(stub_dir) == before + 1,
                f"WAYLAND_DISPLAY was {out!r}: a stale marker is still "
                "suppressing the compositor, so the fallback is permanent",
            )
            case(
                "and the stale marker is cleared on the way past",
                not marker.exists(),
                "the marker survived, so it will go stale again next run",
            )

            # ── every fallback says which display it chose (#830) ──────────
            #
            # The path below announces itself; the one where the socket never
            # appears at all did not, so a CI log grepped for `postio runner:`
            # came back empty whether mutter had worked perfectly or never
            # started. Two opposite outcomes, one silence, and no way to tell
            # from the log which configuration the suites had actually proved.
            mute_stubs = base / "mute-stubs"
            mute_stubs.mkdir()
            (mute_stubs / "mutter").write_text("#!/usr/bin/env bash\nexit 0\n")
            (mute_stubs / "mutter").chmod(0o755)
            mute_target = mute_stubs / "gtk_list-0123456789abcdef"
            mute_target.write_text(TARGET_STUB)
            mute_target.chmod(0o755)
            silent_runtime = base / "never-binds"
            silent_runtime.mkdir()
            result = run_full(mute_target, mute_stubs, silent_runtime)
            case(
                "a compositor that never binds falls back",
                result.stdout == "<unset>",
                f"WAYLAND_DISPLAY was {result.stdout!r}",
            )
            case(
                "and says so, rather than falling back in silence",
                "postio runner:" in result.stderr,
                "nothing on stderr: a CI log cannot distinguish this from a "
                f"compositor that worked. stderr was {result.stderr!r}",
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
