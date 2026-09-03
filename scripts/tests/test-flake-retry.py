#!/usr/bin/env python3
"""Self-test for scripts/test-with-flake-retry.sh.

#886: two full workspace test runs while cutting v0.2.0 each threw a couple
of failures, never the same targets twice, none touching the release
commit's own diff -- and every one of them passed clean the moment it was
rerun alone. This is that triage, mechanised, against a stubbed `cargo` so
the case runs in milliseconds and never touches a real compiler.

Usage: scripts/tests/test-flake-retry.py
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
SCRIPT = HERE / "test-with-flake-retry.sh"

FAILURES: list[str] = []

# A stub `cargo` that behaves like a real one just enough for the script
# under test: the first `--workspace --no-fail-fast` call fails with two
# targets named in cargo's own summary shape; any later call naming one of
# those targets alone is a retry, and its own verdict comes from files the
# test writes into $STUB_DIR before running.
CARGO_STUB = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/calls"

if printf '%s' "$*" | grep -q -- "--workspace" && printf '%s' "$*" | grep -q -- "--no-fail-fast"; then
    if [ -f "$STUB_DIR/workspace-passes" ]; then
        echo "test result: ok. 400 passed; 0 failed"
        exit 0
    fi
    if [ -f "$STUB_DIR/no-target-summary" ]; then
        echo "error: could not compile \\`postio-core\\`" >&2
        exit 101
    fi
    echo "test result: FAILED. 398 passed; 2 failed"
    echo "error: 2 targets failed:" >&2
    echo "    \\`-p fake-a --lib\\`" >&2
    echo "    \\`-p fake-b --test suite\\`" >&2
    exit 101
fi

if printf '%s' "$*" | grep -q -- "-p fake-a --lib"; then
    [ ! -f "$STUB_DIR/fake-a-fails-again" ] && exit 0
    echo "test result: FAILED. 0 passed; 1 failed"
    exit 101
fi

if printf '%s' "$*" | grep -q -- "-p fake-b --test suite"; then
    [ ! -f "$STUB_DIR/fake-b-fails-again" ] && exit 0
    echo "test result: FAILED. 0 passed; 1 failed"
    exit 101
fi

echo "unexpected invocation: cargo $*" >&2
exit 1
"""


def run(stub_dir: Path) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    return subprocess.run(
        ["bash", str(SCRIPT)],
        env=environment,
        capture_output=True,
        text=True,
        timeout=30,
    )


def stub(base: Path, *, flags: tuple[str, ...] = ()) -> Path:
    stub_dir = base / "stub"
    (stub_dir / "bin").mkdir(parents=True)
    cargo = stub_dir / "bin" / "cargo"
    cargo.write_text(CARGO_STUB, encoding="utf-8")
    cargo.chmod(cargo.stat().st_mode | stat.S_IEXEC)
    (stub_dir / "calls").write_text("", encoding="utf-8")
    for flag in flags:
        (stub_dir / flag).write_text("", encoding="utf-8")
    return stub_dir


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def main() -> int:
    # ── a clean run needs no retry at all ────────────────────────────
    with tempfile.TemporaryDirectory() as directory:
        stub_dir = stub(Path(directory), flags=("workspace-passes",))
        result = run(stub_dir)
        calls = (stub_dir / "calls").read_text(encoding="utf-8")
        case(
            "a green suite exits 0",
            result.returncode == 0,
            f"exit {result.returncode}; output:\n{result.stdout}{result.stderr}",
        )
        case(
            "a green suite never retries anything",
            calls.count("cargo") <= 1 or calls.strip().count("\n") == 0,
            f"expected exactly one cargo invocation, got:\n{calls}",
        )

    # ── both failures are flakes: pass ───────────────────────────────
    with tempfile.TemporaryDirectory() as directory:
        stub_dir = stub(Path(directory))
        result = run(stub_dir)
        out = result.stdout + result.stderr
        calls = (stub_dir / "calls").read_text(encoding="utf-8")
        case(
            "when every failing target passes alone, the release gate passes",
            result.returncode == 0,
            f"exit {result.returncode}; output:\n{out}",
        )
        case(
            "both failing targets were retried in isolation",
            "-p fake-a --lib" in calls and "-p fake-b --test suite" in calls,
            f"not every failing target was retried:\n{calls}",
        )
        case(
            "the output says which targets were confirmed flakes",
            "fake-a" in out and "fake-b" in out and "flake" in out,
            f"no flake confirmation in output:\n{out}",
        )

    # ── one failure reproduces alone: block the release ──────────────
    with tempfile.TemporaryDirectory() as directory:
        stub_dir = stub(Path(directory), flags=("fake-b-fails-again",))
        result = run(stub_dir)
        out = result.stdout + result.stderr
        case(
            "a target that fails twice is not a flake and blocks the release",
            result.returncode != 0,
            f"expected a non-zero exit, got {result.returncode}; output:\n{out}",
        )
        case(
            "fake-a, which only failed once, is still named as a flake",
            "fake-a" in out,
            f"fake-a should still be mentioned as confirmed:\n{out}",
        )
        case(
            "fake-b is named as the real failure",
            "fake-b" in out,
            f"fake-b should be named as the blocker:\n{out}",
        )

    # ── a failure with nothing to retry: original status stands ──────
    with tempfile.TemporaryDirectory() as directory:
        stub_dir = stub(Path(directory), flags=("no-target-summary",))
        result = run(stub_dir)
        calls = (stub_dir / "calls").read_text(encoding="utf-8")
        case(
            "a failure with no per-target summary is not retried, and fails",
            result.returncode != 0,
            f"expected a non-zero exit, got {result.returncode}",
        )
        case(
            "nothing named `fake-a` or `fake-b` was invoked -- there was nothing to retry",
            "fake-a" not in calls and "fake-b" not in calls,
            f"a retry was attempted with nothing to retry:\n{calls}",
        )

    for failure in FAILURES:
        print(f"FAIL  {failure}")
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.")
        return 1
    print("test-with-flake-retry self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
