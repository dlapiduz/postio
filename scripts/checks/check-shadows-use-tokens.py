#!/usr/bin/env python3
"""A drop shadow in the GTK stylesheets comes from the token scale.

Five overlays each typed `box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18)` beside
a token scale that already had `--postio-shadow-sm/-md/-lg` -- generated from
the design system, and redefined for the dark scheme, where a black shadow at
18% on a near-black ground is no lift at all. The literal was right on the
day it was typed and could not follow the tokens anywhere after that.

The rule: no `box-shadow` in a desktop crate's stylesheets --
`crates/{postio-widgets,postio-gtk}/data/*.css`, the shared
crate's included (ADR 0043) -- may carry a colour of its own -- no `rgba(`,
`rgb(`, `hsl(` or `#hex`. A shadow is `var(--postio-shadow-*)`, or an inset
hairline drawn with a colour token (`inset 0 -1px var(--postio-hairline)`),
or `none`. The scale itself is defined from libadwaita's shade colour, in
`focus-colours.css`.

Fix: use `var(--postio-shadow-sm|md|lg)`, or a colour token. For a lift the
scale does not have, add it to the design system's tokens and regenerate,
rather than typing it here.

Usage:
    python3 scripts/checks/check-shadows-use-tokens.py             # the repository
    python3 scripts/checks/check-shadows-use-tokens.py --root DIR  # a fixture repository
    python3 scripts/checks/check-shadows-use-tokens.py SHEET       # one stylesheet
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# The crates whose `data/` holds stylesheets: the desktop app, and the crate
# holding the widget rules it draws with (ADR 0043; specs/007-postio-focus R1).
CRATES = ("postio-widgets", "postio-gtk")

COMMENT = re.compile(r"/\*.*?\*/", re.S)
DECLARATION = re.compile(r"box-shadow\s*:([^;}]*)", re.S)
LITERAL = re.compile(r"\b(?:rgba?|hsla?)\(|#[0-9a-fA-F]{3,8}\b")


def sheets_of(argv: list[str]) -> list[tuple[Path, str]]:
    """Each stylesheet to read, with how it is shown."""
    args = argv[1:]
    if args and args[0] != "--root":
        return [(Path(args[0]), Path(args[0]).name)]
    root = Path(args[1]) if args else ROOT
    return [
        (sheet, sheet.relative_to(root).as_posix())
        for crate in CRATES
        for sheet in sorted((root / "crates" / crate / "data").glob("*.css"))
    ]


def main(argv: list[str]) -> int:
    problems = []
    for sheet, shown in sheets_of(argv):
        text = COMMENT.sub(lambda m: re.sub(r"[^\n]", " ", m.group(0)), sheet.read_text(encoding="utf-8"))
        for match in DECLARATION.finditer(text):
            if LITERAL.search(match.group(1)):
                line = text.count("\n", 0, match.start()) + 1
                value = " ".join(match.group(1).split())
                problems.append(f"  {shown}:{line}: box-shadow:{value}")
    if not problems:
        return 0
    print("Drop shadows with a colour of their own, not the token scale:")
    print("\n".join(problems))
    print()
    print("Fix: var(--postio-shadow-sm|md|lg), or a colour token. A lift the scale")
    print("lacks goes in the design system's tokens, not typed in here.")
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
