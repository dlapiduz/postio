#!/usr/bin/env python3
"""Refuse a stylesheet whose braces do not balance.

CSS is the one file in this repository that nothing type-checks. GTK's parser
does not stop at a broken rule either -- it reports the parse error to stderr,
which nobody is reading during a test run, and carries on with whatever it
managed to understand. So a lost `}` silently unstyles everything after it.

This has been paid for. A merge resolution on #1496 dropped one closing brace
in `shell.css`; roughly 600 lines below it stopped applying, and the failure
surfaced as two *composer focus* tests -- a suite with no connection to the
rule that broke. The session very nearly dismissed them as pre-existing.

# The rule

Every `.css` file under `crates/` balances its braces, counted outside
comments, and never closes more than it has opened.

This does not make CSS type-checked. It catches the one structural error that
costs hours to trace back, and it costs a few milliseconds.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATES = ROOT / "crates"

COMMENT = re.compile(r"/\*.*?\*/", re.S)


def offenders() -> list[str]:
    found: list[str] = []
    for path in sorted(CRATES.rglob("*.css")):
        where = path.relative_to(ROOT)
        depth = 0
        unbalanced_at = None
        for number, line in enumerate(
            COMMENT.sub("", path.read_text()).splitlines(), start=1
        ):
            depth += line.count("{") - line.count("}")
            if depth < 0 and unbalanced_at is None:
                unbalanced_at = number
        if unbalanced_at is not None:
            found.append(f"{where}:{unbalanced_at}: closes a rule that was never opened")
        elif depth > 0:
            found.append(f"{where}: ends inside {depth} unclosed rule(s)")
    return found


def main() -> int:
    found = offenders()
    if not found:
        print("css-braces-balance check passed.")
        return 0

    print("css-braces-balance check FAILED\n", file=sys.stderr)
    for line in found:
        print(f"  {line}", file=sys.stderr)
    print(
        "\nGTK reports a CSS parse error to stderr and then keeps going with\n"
        "what it understood, so everything below the break is simply unstyled\n"
        "and nothing says so. On #1496 that surfaced as two composer focus\n"
        "tests failing -- a suite with no connection to the rule that broke.\n\n"
        "Find the rule that does not close and close it.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
