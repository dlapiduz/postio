#!/usr/bin/env python3
"""Self-test for scripts/test-with-flake-retry.sh.

#886: two full workspace test runs while cutting v0.2.0 each threw a couple
of failures, never the same targets twice, none touching the release
commit's own diff -- and every one of them passed clean the moment it was
rerun alone. This is that triage, mechanised, against a stubbed `cargo`.

#1504: cutting v0.3.0 found the script itself had drifted -- it still ran
plain `cargo test`, from before this workspace adopted nextest, so it never
saw `.config/nextest.toml`'s `default-filter` and had no timeout backstop
when a measurement-tier test hung. The script now runs `cargo nextest run`
and parses nextest's own `Summary` section instead of cargo's per-target
one; this stub speaks nextest's shape.

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

sys.path.insert(0, str(HERE / "lib"))
import patience  # noqa: E402  -- enabled by the sys.path line above

FAILURES: list[str] = []

# A stub `cargo` that answers just enough like nextest for the script under
# test: the first `nextest run --workspace ...` call fails with two tests
# named the way nextest's own `Summary` section names them, one per line
# after a `Summary [...]` marker (nextest repeats the same line once live
# and once in the summary; only the summary copy is meant to be read). A
# later call carrying `-E "binary_id(...) & test(=...)"` is a retry, and its
# verdict comes from files the test writes into $STUB_DIR before running.
CARGO_STUB = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/calls"

if printf '%s' "$*" | grep -q -- "nextest run" && printf '%s' "$*" | grep -q -- "--workspace"; then
    if [ -f "$STUB_DIR/workspace-passes" ]; then
        echo "Summary [   0.010s] 400 tests run: 400 passed, 0 skipped"
        exit 0
    fi
    if [ -f "$STUB_DIR/no-summary" ]; then
        echo "error: could not compile \\`postio-core\\`" >&2
        exit 100
    fi
    echo "test result: FAILED"
    echo "        FAIL [   0.010s] (1/2) fake-crate::fake_suite tests::a_thing"
    echo "        FAIL [   0.020s] (2/2) fake-crate::fake_suite tests::b_thing"
    echo "Summary [   0.030s] 400 tests run: 398 passed, 2 failed, 0 skipped"
    echo "        FAIL [   0.010s] (1/2) fake-crate::fake_suite tests::a_thing"
    echo "        FAIL [   0.020s] (2/2) fake-crate::fake_suite tests::b_thing"
    exit 100
fi

if printf '%s' "$*" | grep -q -- "test(=tests::a_thing)"; then
    [ ! -f "$STUB_DIR/a-thing-fails-again" ] && exit 0
    echo "Summary [   0.010s] 1 test run: 0 passed, 1 failed, 0 skipped"
    echo "        FAIL [   0.010s] (1/1) fake-crate::fake_suite tests::a_thing"
    exit 100
fi

if printf '%s' "$*" | grep -q -- "test(=tests::b_thing)"; then
    [ ! -f "$STUB_DIR/b-thing-fails-again" ] && exit 0
    echo "Summary [   0.010s] 1 test run: 0 passed, 1 failed, 0 skipped"
    echo "        FAIL [   0.010s] (1/1) fake-crate::fake_suite tests::b_thing"
    exit 100
fi

echo "unexpected invocation: cargo $*" >&2
exit 1
"""


def run(stub_dir: Path) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    return patience.run(
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
            calls.strip().count("\n") == 0,
            f"expected exactly one cargo invocation, got:\n{calls}",
        )

    # ── both failures are flakes: pass ───────────────────────────────
    with tempfile.TemporaryDirectory() as directory:
        stub_dir = stub(Path(directory))
        result = run(stub_dir)
        out = result.stdout + result.stderr
        calls = (stub_dir / "calls").read_text(encoding="utf-8")
        case(
            "when every failing test passes alone, the release gate passes",
            result.returncode == 0,
            f"exit {result.returncode}; output:\n{out}",
        )
        case(
            "both failing tests were retried in isolation, by nextest filter expression",
            "test(=tests::a_thing)" in calls and "test(=tests::b_thing)" in calls,
            f"not every failing test was retried:\n{calls}",
        )
        case(
            "the retry names the test's own binary_id, not a bare crate name",
            "binary_id(fake-crate::fake_suite)" in calls,
            f"expected a binary_id() filter naming the test binary:\n{calls}",
        )
        case(
            "the output says which tests were confirmed flakes",
            "a_thing" in out and "b_thing" in out and "flake" in out,
            f"no flake confirmation in output:\n{out}",
        )

    # ── one failure reproduces alone: block the release ──────────────
    with tempfile.TemporaryDirectory() as directory:
        stub_dir = stub(Path(directory), flags=("b-thing-fails-again",))
        result = run(stub_dir)
        out = result.stdout + result.stderr
        case(
            "a test that fails twice is not a flake and blocks the release",
            result.returncode != 0,
            f"expected a non-zero exit, got {result.returncode}; output:\n{out}",
        )
        case(
            "a_thing, which only failed once, is still named as a flake",
            "a_thing" in out,
            f"a_thing should still be mentioned as confirmed:\n{out}",
        )
        case(
            "b_thing is named as the real failure",
            "b_thing" in out,
            f"b_thing should be named as the blocker:\n{out}",
        )

    # ── a failure with nothing to retry: original status stands ──────
    with tempfile.TemporaryDirectory() as directory:
        stub_dir = stub(Path(directory), flags=("no-summary",))
        result = run(stub_dir)
        calls = (stub_dir / "calls").read_text(encoding="utf-8")
        case(
            "a failure with no Summary section is not retried, and fails",
            result.returncode != 0,
            f"expected a non-zero exit, got {result.returncode}",
        )
        case(
            "nothing naming a_thing or b_thing was invoked -- there was nothing to retry",
            "a_thing" not in calls and "b_thing" not in calls,
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
