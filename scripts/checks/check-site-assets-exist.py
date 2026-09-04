#!/usr/bin/env python3
"""Refuse a `site/` page that points at an asset the site does not ship.

A missing stylesheet is loud: the page renders unstyled and nobody can miss
it. A missing *icon* is silent. The browser asks for `assets/favicon.svg`,
gets a 404, and falls back to its own default document icon -- which is
exactly what the site looked like before it had a favicon at all, so the
failure and the bug are indistinguishable by looking (#1024).

That is the same shape as the icon bug this project already had once: the
app grid showed a generic icon and every layer involved was individually
correct (#427). What makes it worth a check rather than care is that the
reference and the file are in different places, and only one of them moves
when somebody reorganises `site/assets/`.

# The rule

Every `href`/`src` in a `site/**/*.html` page that is a plain relative path
must resolve to a file that exists. Absolute URLs, anchors, and anything
with a scheme are somebody else's problem and are skipped -- as is `docs/`,
which the Pages workflow assembles from the built mdBook rather than from
this tree.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SITE = ROOT / "site"

# `href="..."` or `src="..."`, single or double quoted.
REFERENCE = re.compile(r"""(?:href|src)\s*=\s*["']([^"']+)["']""", re.I)

# Not ours to resolve: a scheme, a protocol-relative URL, an in-page anchor,
# a query-only link, or the built book the deploy job drops in beside us.
def external(target: str) -> bool:
    return (
        target.startswith(("#", "?", "//", "mailto:", "data:"))
        or re.match(r"^[a-z][a-z0-9+.-]*:", target, re.I) is not None
        or target.split("/", 1)[0] == "docs"
    )


def main() -> int:
    if not SITE.is_dir():
        print("site-assets-exist check skipped (no site/).")
        return 0

    pages = sorted(SITE.rglob("*.html"))
    missing: list[tuple[Path, str]] = []
    checked = 0
    for page in pages:
        for target in REFERENCE.findall(page.read_text(encoding="utf-8")):
            target = target.split("#", 1)[0].split("?", 1)[0]
            if not target or external(target):
                continue
            checked += 1
            if not (page.parent / target).exists():
                missing.append((page.relative_to(ROOT), target))

    if missing:
        print(
            "Pages reference assets that are not in the tree, so the deployed "
            "site\nserves a 404 for each -- silently, in the case of an icon:\n",
            file=sys.stderr,
        )
        for page, target in missing:
            print(f"  {page} -> {target}", file=sys.stderr)
        print(
            "\nAdd the file under site/assets/, or fix the reference. See #1024.",
            file=sys.stderr,
        )
        return 1

    print(f"site-assets-exist check passed ({checked} references in {len(pages)} page(s)).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
