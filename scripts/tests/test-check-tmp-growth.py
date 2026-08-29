#!/usr/bin/env python3
"""Self-test for scripts/checks/check-tmp-growth.py.

Throwaway git trees, never the real repository — the same shape
test-check-toolchain-pinned.py uses. This check is a diagnostic rather than
an invariant (#605), so every case here expects exit 0; what's asserted is
which stream carries the message and whether it names the leak.

Usage: scripts/tests/test-check-tmp-growth.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-tmp-growth.py"

FAILURES: list[str] = []


def run(root: Path) -> subprocess.CompletedProcess[str]:
    """Run a copy of the check against `root`, the way check.sh would."""
    subprocess.run(["git", "init", "-q"], cwd=root, check=True)
    scripts = root / "scripts"
    scripts.mkdir(exist_ok=True)
    (scripts / CHECK.name).write_bytes(CHECK.read_bytes())
    return subprocess.run(
        [sys.executable, str(scripts / CHECK.name)],
        cwd=root,
        capture_output=True,
        text=True,
    )


def expect(name: str, result: subprocess.CompletedProcess[str], *, status: int = 0) -> None:
    if result.returncode != status:
        FAILURES.append(
            f"{name}: expected exit {status}, got {result.returncode} "
            f"(stdout={result.stdout!r} stderr={result.stderr!r})"
        )


def main() -> int:
    # ── no target/tmp at all: the ordinary state for a fresh worktree ──────
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        result = run(root)
        expect("a tree with no target/tmp passes quietly", result)
        if "does not exist" not in result.stdout:
            FAILURES.append(
                f"a tree with no target/tmp should say so on stdout: {result.stdout!r}"
            )

    # ── an empty target/tmp: nothing has leaked ────────────────────────────
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        (root / "target" / "tmp").mkdir(parents=True)
        result = run(root)
        expect("an empty target/tmp passes quietly", result)
        if result.stderr.strip():
            FAILURES.append(f"an empty target/tmp should print nothing on stderr: {result.stderr!r}")
        if "passed" not in result.stdout:
            FAILURES.append(f"an empty target/tmp should say passed: {result.stdout!r}")

    # ── a handful of small leftovers: under both thresholds ────────────────
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        tmp = root / "target" / "tmp"
        for n in range(5):
            leftover = tmp / f"postio-test-{n}"
            leftover.mkdir(parents=True)
            (leftover / "state").write_text("x" * 1024, encoding="utf-8")
        result = run(root)
        expect("a handful of small leftovers still passes quietly", result)
        if result.stderr.strip():
            FAILURES.append(
                f"a handful of small leftovers should print nothing on stderr: {result.stderr!r}"
            )

    # ── past the entry-count threshold: noticed, but still exit 0 ──────────
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        tmp = root / "target" / "tmp"
        for n in range(60):
            (tmp / f"postio-test-{n}").mkdir(parents=True)
        result = run(root)
        expect("past the entry-count threshold still exits 0", result)
        if "target/tmp" not in result.stderr or "#605" not in result.stderr:
            FAILURES.append(
                f"a growth warning should name target/tmp and #605: {result.stderr!r}"
            )
        if "rm -rf" not in result.stderr:
            FAILURES.append(f"a growth warning should name the one-line fix: {result.stderr!r}")

    # ── past the byte-size threshold with few entries: still noticed ───────
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        tmp = root / "target" / "tmp"
        big = tmp / "postio-blob-store"
        big.mkdir(parents=True)
        (big / "blob").write_bytes(b"\0" * (201 * 1024 * 1024))
        result = run(root)
        expect("one oversized leftover is noticed by size alone", result)
        if "target/tmp" not in result.stderr:
            FAILURES.append(f"an oversized leftover should be reported: {result.stderr!r}")

    if FAILURES:
        print("FAILED:")
        for failure in FAILURES:
            print(f"  {failure}")
        return 1
    print(f"{Path(__file__).name}: all cases passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
