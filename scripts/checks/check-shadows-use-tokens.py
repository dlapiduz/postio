#!/usr/bin/env python3
"""A drop shadow in the GTK stylesheet comes from the token scale.

Five overlays each typed `box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18)` beside
a token scale that already had `--postio-shadow-sm/-md/-lg` -- generated from
the design system, and redefined for the dark scheme, where a black shadow at
18% on a near-black ground is no lift at all. The literal was right on the
day it was typed and could not follow the tokens anywhere after that.

The rule: no `box-shadow` in `crates/postio-gtk/data/shell.css` may carry a
colour of its own -- no `rgba(`, `rgb(`, `hsl(` or `#hex`. A shadow is
`var(--postio-shadow-*)`, or an inset hairline drawn with a colour token
(`inset 0 -1px var(--postio-hairline)`), or `none`. `tokens.css` is exempt:
it is generated from the design system, and is where the scale is defined.

Fix: use `var(--postio-shadow-sm|md|lg)`, or a colour token. For a lift the
scale does not have, add it to the design system's tokens and regenerate,
rather than typing it here.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SHEET = ROOT / "crates/postio-gtk/data/shell.css"

COMMENT = re.compile(r"/\*.*?\*/", re.S)
DECLARATION = re.compile(r"box-shadow\s*:([^;}]*)", re.S)
LITERAL = re.compile(r"\b(?:rgba?|hsla?)\(|#[0-9a-fA-F]{3,8}\b")


def main(argv: list[str]) -> int:
    sheet = Path(argv[1]) if len(argv) == 2 else SHEET
    text = COMMENT.sub(lambda m: re.sub(r"[^\n]", " ", m.group(0)), sheet.read_text(encoding="utf-8"))
    problems = []
    for match in DECLARATION.finditer(text):
        if LITERAL.search(match.group(1)):
            line = text.count("\n", 0, match.start()) + 1
            value = " ".join(match.group(1).split())
            problems.append(f"  {sheet.name}:{line}: box-shadow:{value}")
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
