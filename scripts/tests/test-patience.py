#!/usr/bin/env python3
"""Self-test for scripts/lib/patience.py (#1249).

The dial `POSTIO_TEST_PATIENCE` has moved every deadline in the Rust suite
since #842, and `check-test-deadlines-scale.py` has required Rust deadlines to
reach it since #957. Both stop at the crate boundary. The 81 self-tests under
`scripts/tests/` shell out to the scripts they cover with deadlines written by
hand, so the one dial anybody knows about moves none of them -- and #1243 is
what that costs: a 30-second deadline for milliseconds of stubbed work,
expiring once on a runner building 81 sandboxes four at a time, reported as a
plain FAILED indistinguishable from the script getting the answer wrong.

This covers the shared helper that closes both halves.

Usage: scripts/tests/test-patience.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- the sys.path line above is what makes it importable

FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def with_patience(value: str | None):
    """Set (or clear) the dial for one call, and put it back."""
    before = os.environ.get(patience.PATIENCE_ENV)
    if value is None:
        os.environ.pop(patience.PATIENCE_ENV, None)
    else:
        os.environ[patience.PATIENCE_ENV] = value
    try:
        return patience.patience()
    finally:
        if before is None:
            os.environ.pop(patience.PATIENCE_ENV, None)
        else:
            os.environ[patience.PATIENCE_ENV] = before


def main() -> int:
    case("no dial set is 1x", with_patience(None) == 1.0, f"got {with_patience(None)}")
    case("the dial multiplies", with_patience("4") == 4.0, f"got {with_patience('4')}")
    case(
        "a fraction is honoured",
        with_patience("0.5") == 0.5,
        f"got {with_patience('0.5')}",
    )

    # A typo in a workflow must not set every deadline in the suite to zero and
    # turn every wait into an instant failure. Same rule as the Rust side.
    case("a typo is ignored", with_patience("soon") == 1.0, f"got {with_patience('soon')}")
    case("zero is ignored", with_patience("0") == 1.0, f"got {with_patience('0')}")
    case(
        "a negative is ignored",
        with_patience("-2") == 1.0,
        f"got {with_patience('-2')}",
    )
    case("an empty value is ignored", with_patience("") == 1.0, f"got {with_patience('')}")

    os.environ[patience.PATIENCE_ENV] = "3"
    case("deadline scales", patience.deadline(10) == 30.0, f"got {patience.deadline(10)}")
    os.environ.pop(patience.PATIENCE_ENV, None)

    # The wrapper behaves like `subprocess.run` for anything that finishes.
    done = patience.run(
        ["bash", "-c", "printf hello"], capture_output=True, text=True, timeout=30
    )
    case(
        "a command that finishes comes back whole",
        done.returncode == 0 and done.stdout == "hello",
        f"got exit {done.returncode}, stdout {done.stdout!r}",
    )

    # And a command that does not says *that*, rather than looking like a
    # wrong answer. This is the half #1243 was really about.
    try:
        patience.run(["bash", "-c", "sleep 5"], timeout=0.05)
        case("a hang raises ScriptHung", False, "it returned instead of raising")
    except patience.ScriptHung as hung:
        message = str(hung)
        case(
            "a hang raises ScriptHung",
            True,
            "",
        )
        case(
            "and the message names the deadline, not the behaviour",
            "did not finish" in message and patience.PATIENCE_ENV in message,
            f"got {message!r}",
        )
    except subprocess.TimeoutExpired:
        case(
            "a hang raises ScriptHung",
            False,
            "the raw TimeoutExpired escaped, which is the thing being fixed",
        )

    # The dial reaches the wrapper too: the same call that timed out above
    # survives once the runner is declared busy.
    os.environ[patience.PATIENCE_ENV] = "200"
    try:
        slow = patience.run(
            ["bash", "-c", "sleep 0.2; printf late"],
            capture_output=True,
            text=True,
            timeout=0.05,
        )
        case(
            "the dial reaches the wrapper's deadline",
            slow.stdout == "late",
            f"got {slow.stdout!r}",
        )
    except patience.ScriptHung as hung:
        case("the dial reaches the wrapper's deadline", False, str(hung))
    finally:
        os.environ.pop(patience.PATIENCE_ENV, None)

    for failure in FAILURES:
        print(f"FAIL  {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed", file=sys.stderr)
        return 1
    print("\npatience: all cases behaved")
    return 0


if __name__ == "__main__":
    sys.exit(main())
