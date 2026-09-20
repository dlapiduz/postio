#!/usr/bin/env python3
"""The default landing runs doctests for the crates it changed (#1440).

`run_doctests` existed and was called only under `--full`. CLAUDE.md says to
land on the default and that `--full` needs a specific reason, so in practice
a broken doctest was found by CI sixteen minutes later, on a branch whose
landing had gone green.

That happened. Three doc comments quoted evidence as an indented block, which
rustdoc reads as *Rust* and compiles:

    Reader::scroll_to_fragment (line 1373) ... FAILED
    error: expected one of `!` or `::`, found `started`

The run aborts on the first error, so the other two were queued behind it --
one round trip each.

What this asserts is the shape of the gate, not the behaviour of `cargo`:
that the default branch of the gate chain calls `run_doctests` per changed
crate, and that `run_doctests` is still the `--doc` call it claims to be.
Driving a whole landing to find out would cost minutes and prove the same
thing.

Usage: scripts/tests/test-issue-land-doctests.py
Exit status: 0 if the gate is in place, 1 otherwise.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[2] / "scripts" / "issue-land.sh"


def fail(what: str) -> None:
    print(f"FAIL: {what}", file=sys.stderr)


def main() -> int:
    source = SCRIPT.read_text(encoding="utf-8")
    problems = 0

    # `run_doctests` must still be the thing its name says. If it stops
    # passing `--doc`, every assertion below is about a call that runs
    # something else.
    body = re.search(r"run_doctests\(\)\s*\{(.*?)\n\}", source, re.S)
    if not body:
        fail("run_doctests is gone or reshaped; this test cannot find it")
        return 1
    if "--doc" not in body.group(1):
        fail(f"run_doctests no longer passes --doc: {body.group(1).strip()!r}")
        problems += 1

    # The default branch -- the one an ordinary landing takes -- must call it.
    # Split on the `--full` branch so a call that only exists there does not
    # satisfy this.
    full_branch = source.index('echo "--- test (full)')
    default_branch = source.index("        # The suites the sanity tier cannot fail for")
    if default_branch < full_branch:
        fail("the branches moved; this test is reading the wrong halves")
        return 1
    default = source[default_branch:]

    if "run_doctests" not in default:
        fail(
            "the default landing does not run doctests. A broken one is then "
            "CI's to find, sixteen minutes later, on a branch whose landing "
            "went green (#1440)"
        )
        problems += 1
    elif 'run_doctests -p "$crate"' not in default:
        fail(
            "the default landing calls run_doctests, but not per changed "
            "crate -- a whole-workspace doctest run is the slow gate this "
            "was meant to avoid"
        )
        problems += 1

    if problems:
        print(f"\n{problems} problem(s).", file=sys.stderr)
        return 1
    print("issue-land doctest gate check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
