#!/usr/bin/env python3
"""Self-test for scripts/flake-check.sh.

The script proves whether a failing test is broken or load-sensitive. This
runs it against a stub `cargo` (FLAKE_CHECK_CARGO) and a stub load generator
(FLAKE_CHECK_LOAD_CMD), so nothing here compiles or runs a real test.

The stub load generator touches $STUB_DIR/loaded and then sleeps; the stub
`cargo` fails a run while that file exists and the case says "load" mode,
which is how a test that only fails under CPU load is modelled.

Usage: scripts/tests/test-flake-check.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "flake-check.sh"

sys.path.insert(0, str(HERE / "lib"))
import patience  # noqa: E402  -- enabled by the sys.path line above

FAILURES: list[str] = []

CARGO_STUB = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/calls"
case " $* " in *" --no-run "*)
    [ -f "$STUB_DIR/build-fails" ] && { echo "error: could not compile" >&2; exit 101; }
    exit 0 ;;
esac
case "$STUB_MODE" in
    pass) exit 0 ;;
    broken) exit 100 ;;
    flaky) [ -f "$STUB_DIR/ran" ] && exit 0; touch "$STUB_DIR/ran"; exit 100 ;;
    load) [ -f "$STUB_DIR/loaded" ] && exit 100; exit 0 ;;
esac
exit 0
"""

LOAD_STUB = """#!/usr/bin/env bash
touch "$STUB_DIR/loaded"
echo $$ >> "$STUB_DIR/loaders"
exec sleep 600
"""


def check(name: str, ok: bool, detail: str = "") -> None:
    if ok:
        print(f"ok  {name}")
    else:
        print(f"FAIL {name}: {detail}")
        FAILURES.append(name)


def run(mode: str, *args: str, build_fails: bool = False):
    with tempfile.TemporaryDirectory() as d:
        stub = Path(d)
        for name, body in (("cargo", CARGO_STUB), ("load", LOAD_STUB)):
            p = stub / name
            p.write_text(body)
            p.chmod(p.stat().st_mode | stat.S_IXUSR)
        if build_fails:
            (stub / "build-fails").write_text("")
        env = dict(
            os.environ,
            STUB_DIR=str(stub),
            STUB_MODE=mode,
            FLAKE_CHECK_CARGO=str(stub / "cargo"),
            FLAKE_CHECK_LOAD_CMD=str(stub / "load"),
        )
        r = patience.run(
            [str(SCRIPT), "-p", "some-crate", "--test", "some_suite", "a::b",
             *args],
            env=env, capture_output=True, text=True, timeout=60,
        )
        calls = (stub / "calls").read_text() if (stub / "calls").exists() else ""
        loaders = (
            (stub / "loaders").read_text().split()
            if (stub / "loaders").exists() else []
        )
        return r, calls, loaders


def alive(pid: str) -> bool:
    try:
        os.kill(int(pid), 0)
        return True
    except OSError:
        return False


def main() -> int:
    r, calls, loaders = run("pass", "--runs", "2", "--load-cores", "2")
    out = r.stdout + r.stderr
    check("passes both: exit 0", r.returncode == 0, out)
    check("passes both: verdict", "passes idle and loaded" in out, out)
    lines = calls.splitlines()
    check("builds once with --no-run", sum(" --no-run" in l + " " for l in lines) == 1, calls)
    check("2 idle + 2 loaded runs", len(lines) == 5, calls)
    check("scoped to crate, suite, exact filter",
          all("-p some-crate" in l and "--test some_suite" in l for l in lines)
          and all("--exact a::b" in l for l in lines[1:]), calls)
    check("never passes -j", not any(" -j" in " " + l for l in lines), calls)
    check("load generators were started", len(loaders) == 2, str(loaders))
    check("load generators are killed on exit",
          not any(alive(p) for p in loaders), str(loaders))
    check("reports counts and load average",
          "2/2" in out and "min" in out and "median" in out and "max" in out
          and "load average" in out, out)

    r, _, _ = run("load", "--runs", "2", "--load-cores", "1")
    out = r.stdout + r.stderr
    check("fails only loaded: exit 1", r.returncode == 1, out)
    check("fails only loaded: verdict", "fails only under load" in out, out)

    r, calls, _ = run("broken", "--runs", "2", "--load-cores", "1")
    out = r.stdout + r.stderr
    check("fails idle: exit 2", r.returncode == 2, out)
    check("fails idle: verdict", "fails idle too" in out, out)

    r, _, _ = run("flaky", "--runs", "3", "--load-cores", "1")
    out = r.stdout + r.stderr
    check("flaky idle: exit 4", r.returncode == 4, out)
    check("flaky idle: verdict", "flaky even idle (1/3 idle failures)" in out, out)
    check("flaky idle: not called broken", "broken, not flaky" not in out, out)

    r, calls, _ = run("pass", build_fails=True)
    check("build failure: exit 3, no runs",
          r.returncode == 3 and len(calls.splitlines()) == 1,
          f"{r.returncode} {calls}")

    r, _, _ = run("pass", "--runs", "zero")
    check("bad --runs: exit 3", r.returncode == 3, r.stderr)

    print()
    if FAILURES:
        print(f"{len(FAILURES)} failed: {', '.join(FAILURES)}")
        return 1
    print("all cases behaved")
    return 0


if __name__ == "__main__":
    sys.exit(main())
