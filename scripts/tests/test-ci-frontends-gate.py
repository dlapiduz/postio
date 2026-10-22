#!/usr/bin/env python3
"""Self-test for scripts/ci-frontends-gate.sh.

`Frontends` is the required check that stands for `Tests (GTK widgets N/2)` and
`Tests (application)`. The trap it must not fall into is the one a skipped job
sets: GitHub counts a skip as passed, so a gate that treats "skipped" as fine
reports green having run nothing. A skip is legitimate only when the `changes`
classification said no Rust changed.

Usage: scripts/tests/test-ci-frontends-gate.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "ci-frontends-gate.sh"

FAILURES: list[str] = []


def gate(rust: str, changes: str, *results: str) -> int:
    result = patience.run(
        ["bash", str(SCRIPT), rust, changes, *results],
        capture_output=True, text=True, timeout=30,
    )
    return result.returncode


def case(name: str, expected: int, rust: str, changes: str, *results: str) -> None:
    got = gate(rust, changes, *results)
    if got != expected:
        FAILURES.append(f"{name}\n    expected exit {expected}, got {got}")


def main() -> int:
    if not SCRIPT.exists():
        print(f"missing {SCRIPT}", file=sys.stderr)
        return 1

    ok = ("test-gtk=success", "test-app=success")

    case("all succeeded passes", 0, "yes", "success", *ok)
    case("a failed shard fails", 1, "yes", "success", "test-gtk=failure", "test-app=success")
    case("a failed app suite fails", 1, "yes", "success", "test-gtk=success", "test-app=failure")
    case("a cancelled job fails", 1, "yes", "success", "test-gtk=cancelled", "test-app=success")

    # The trap: a skip with Rust changed means the work was not done.
    case("skipped when Rust changed fails", 1, "yes", "success", "test-gtk=skipped", "test-app=skipped")
    case("one skipped when Rust changed fails", 1, "yes", "success", "test-gtk=success", "test-app=skipped")

    # And the legitimate skip.
    case("skipped when no Rust changed passes", 0, "no", "success", "test-gtk=skipped", "test-app=skipped")
    case("a failure is a failure even with no Rust changed", 1, "no", "success",
         "test-gtk=failure", "test-app=skipped")

    # A classification that did not happen proves nothing.
    case("a failed classification fails", 1, "", "failure", "test-gtk=skipped", "test-app=skipped")
    case("a skipped classification fails", 1, "", "skipped", "test-gtk=skipped", "test-app=skipped")
    case("an unreadable rust answer fails", 1, "", "success", "test-gtk=skipped", "test-app=skipped")
    case("an unknown rust answer fails", 1, "maybe", "success", "test-gtk=skipped", "test-app=skipped")
    case("an unknown result fails", 1, "yes", "success", "test-gtk=success", "test-app=")
    case("no jobs to check fails", 1, "yes", "success")

    for failure in FAILURES:
        print(f"FAIL: {failure}")
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed")
        return 1
    print("ci-frontends-gate self-test passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
