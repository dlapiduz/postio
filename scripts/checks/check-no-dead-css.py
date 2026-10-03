#!/usr/bin/env python3
"""Every Postio class the GTK stylesheets style is one the code can set.

`shell.css` and the generated `tokens.css` had grown ~150 lines of rules for
widgets that no longer exist: the thread column that became the conversation,
the old search bar, a settings header, the conversation view's first action
row. Nothing failed, because a CSS rule for a class nobody sets is not an
error -- it is only a paragraph a reader has to understand before learning it
does nothing, and a place the next restyle edits to no effect.

The rule: a `.postio-*` or `.conversation-*` class selector in a desktop
crate's stylesheets -- `crates/{postio-widgets,postio-gtk}/data/*.css`,
the shared crate's included (ADR 0043) -- must be named somewhere in the Rust
sources.
A mention is the class as a whole token outside a `//` comment line, or a
string that composes it -- `"postio-account-{index}"` covers every class
starting `postio-account-`, and `"{class}-hint"` every class ending `-hint`.
Tests count, because a test that looks a class up is using it; the token
generator (`crates/postio-ui/src/tokens.rs`) does not, because it is where
`tokens.css`'s rules are *written*, not where a widget wears one.

Fix: delete the rule (or just the dead selector from its group). If the class
is in `tokens.css`, delete it from the generator in
`crates/postio-ui/src/tokens.rs` and rebuild -- the file is generated. If the
class really is set, spell it so it can be found: a literal, or a `format!`
whose literal prefix or suffix names it.

Usage:
    python3 scripts/checks/check-no-dead-css.py                  # the repository
    python3 scripts/checks/check-no-dead-css.py --root DIR       # a fixture repository
    python3 scripts/checks/check-no-dead-css.py SHEETS_DIR CRATES_DIR
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# The crates whose `data/` holds stylesheets: the desktop app, and the crate
# holding the widget rules it draws with (ADR 0043; specs/007-postio-focus R1).
CRATES = ("postio-widgets", "postio-gtk")


def stylesheets(root: Path) -> list[Path]:
    return [sheet for crate in CRATES for sheet in sorted((root / "crates" / crate / "data").glob("*.css"))]


CLASS = re.compile(r"\.((?:postio|conversation)-[A-Za-z0-9_-]+)")
COMMENT = re.compile(r"/\*.*?\*/", re.S)
PREFIX = re.compile(r"((?:postio|conversation)-[A-Za-z0-9_-]*-)\{")
SUFFIX = re.compile(r"\}(-[A-Za-z0-9_-]+)")


def styled(stylesheets: list[Path]) -> dict[str, tuple[Path, int]]:
    """Each class selector, with where it first appears."""
    found: dict[str, tuple[Path, int]] = {}
    for sheet in stylesheets:
        text = sheet.read_text(encoding="utf-8")
        # Blank comments out without moving line numbers.
        text = COMMENT.sub(lambda m: re.sub(r"[^\n]", " ", m.group(0)), text)
        # Only selectors: drop declaration blocks, so a value such as a
        # `url(...postio-x.svg)` is never mistaken for one.
        text = re.sub(r"\{[^{}]*\}", lambda m: re.sub(r"[^\n]", " ", m.group(0)), text)
        for match in CLASS.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            found.setdefault(match.group(1), (sheet, line))
    return found


def mentions(sources: Path, generator: Path) -> tuple[str, str]:
    """All the code, and the part of it outside `tests/`."""
    everywhere: list[str] = []
    shipped: list[str] = []
    for path in sorted(sources.rglob("*.rs")):
        parts = path.relative_to(sources).parts
        if "target" in parts or path == generator:
            continue
        for line in path.read_text(encoding="utf-8").splitlines():
            if not line.lstrip().startswith("//"):
                everywhere.append(line)
                if "tests" not in parts:
                    shipped.append(line)
    return "\n".join(everywhere), "\n".join(shipped)


def dead(classes: dict[str, tuple[Path, int]], code: tuple[str, str]) -> list[str]:
    code, shipped = code
    # A composed name only counts from shipped code: a test's
    # `format!("postio-settings-{}", pid)` is a temp directory, not a class.
    prefixes = set(PREFIX.findall(shipped))
    suffixes = set(SUFFIX.findall(shipped))
    words = set(re.findall(r"(?<![A-Za-z0-9_-])((?:postio|conversation)-[A-Za-z0-9_-]+)", code))
    unused = []
    for name in sorted(classes):
        if name in words:
            continue
        if any(name.startswith(p) for p in prefixes):
            continue
        if any(name.endswith(s) for s in suffixes):
            continue
        unused.append(name)
    return unused


def main(argv: list[str]) -> int:
    # Arguments are for the self-test: `--root <repository>`, or
    # <stylesheets dir> <sources dir>.
    root = ROOT
    if len(argv) == 3 and argv[1] == "--root":
        root = Path(argv[2])
    if len(argv) == 3 and argv[1] != "--root":
        sheets = sorted(Path(argv[1]).glob("*.css"))
        sources, generator = Path(argv[2]), Path(argv[2]) / "postio-ui/src/tokens.rs"
    else:
        sheets = stylesheets(root)
        sources, generator = root / "crates", root / "crates/postio-ui/src/tokens.rs"

    classes = styled(sheets)
    unused = dead(classes, mentions(sources, generator))
    if not unused:
        return 0

    print("Stylesheet rules style classes no Rust source sets:")
    for name in unused:
        sheet, line = classes[name]
        try:
            where = sheet.relative_to(root)
        except ValueError:
            where = sheet
        print(f"  {where}:{line}: .{name}")
    print()
    print("Fix: delete the rule, or just that selector from its group. A class in")
    print("tokens.css is generated -- delete it in crates/postio-ui/src/tokens.rs.")
    print("If the class is set, spell it findably: a literal, or a format! whose")
    print('literal prefix ("postio-x-{n}") or suffix ("{class}-hint") names it.')
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
