#!/usr/bin/env python3
"""Every key hint the GTK frontends draw is read from the keymap.

docs/PRODUCT.md §8: the key hints are derived from the command registry, the
same one table the keymap, the palette and the cheat sheet read. A hint typed
in as a literal goes on naming its key after a `[keys]` rebind has moved the
command somewhere else -- `Compose c`, `Render once H` and `Attach another
C-⇧-A` all did (#828 fixed three more before them). A hint that lies is
worse than none.

So in the `src/` of every desktop crate -- the shared `postio-widgets` (ADR
0043) and `postio-focus` -- this refuses:

* a cap built by hand: adding the `postio-keyhint` / `postio-key` class
  anywhere but `widgets/keyhint.rs`, which is where a cap is drawn;
* a literal handed to a hint: `labelled("Compose", "c")`,
  `labelled(.., Some("c"))`, `set_key(Some("Ret"))`;
* a retired notation in a string: `C-x`, or the `⇧` glyph.

A key that is genuinely not a command -- `Tab` between a form's fields,
`Return` on a screen's only button -- goes through
`postio_ui::hints::fixed(key, label, because)`, and each file's count of
those is held to ALLOWED_FIXED below, with the reason. A new one is a new
line here, which is the point: "this is not a command" is a claim somebody
should read.

Fix: build the hint from the keymap -- `postio_ui::hints::{hint, key, pair}`
with the command's `CommandId` -- and draw it with `widgets::keyhint`
(`cap`, `labelled`, `chip`, `KeyLine`). For a real non-command key, use
`hints::fixed` and add the file to ALLOWED_FIXED with its reason.

Usage:
    python3 scripts/checks/check-key-hints-are-derived.py             # the repository
    python3 scripts/checks/check-key-hints-are-derived.py --root DIR  # a fixture repository
    python3 scripts/checks/check-key-hints-are-derived.py SRC_DIR     # one crate's src/
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# The crates whose `src/` draws key hints: the desktop app, and the crate
# holding what it draws (ADR 0043; specs/007-postio-focus R1).
CRATES = ("postio-widgets", "postio-focus")
OWNER = "widgets/keyhint.rs"

# A file is named by its path inside its crate's `src/`, so one that moves
# between the desktop crates keeps its line.
# file -> (count, why no command carries it)
ALLOWED_FIXED: dict[str, tuple[int, str]] = {
    "onboarding.rs": (
        2,
        "Return submits the sign-in form from any field, and Tab moves between "
        "its fields: both are the toolkit's, not registry commands",
    ),
    "unavailable.rs": (1, "Return is the default action of the screen's only button"),
    "search.rs": (1, "Tab into the refine column is the toolkit's focus order"),
    "bar.rs": (1, "Tab steps between the command bar's chips: the entry's own key, not a command"),
}

# file -> why it may still say something this check forbids
ALLOWED_LITERAL: dict[str, str] = {
    # The reader branch owns this file while it is in flight; its `J/K` and
    # `⇧I` move to postio_ui::hints when that lands.
    "reader/rail.rs": "owned by the reader branch in flight; migrates when it lands",
}

CAP_CLASS = re.compile(r'add_css_class\(\s*"postio-key(?:hint)?"')
LITERAL_HINT = re.compile(
    r'\blabelled\(\s*[^,()]*,\s*(?:Some\(\s*)?"'
    r"|\bset_key\(\s*Some\(\s*\""
)
RETIRED = re.compile(r'"[^"\n]*(?:\bC-\S|⇧|\\u\{21e7\})[^"\n]*"')
FIXED = re.compile(r"\bhints::fixed\(")


def code_lines(text: str) -> list[tuple[int, str]]:
    out = []
    for number, line in enumerate(text.splitlines(), 1):
        stripped = line.lstrip()
        if stripped.startswith("//"):
            continue
        out.append((number, line))
    return out


def sources_of(argv: list[str]) -> list[tuple[Path, str]] | None:
    """Each `src/` to scan, with how its files are shown; None if one is missing."""
    args = argv[1:]
    if not args or args[0] == "--root":
        root = Path(args[1]) if args else ROOT
        sources = [(root / "crates" / crate / "src", f"crates/{crate}/src/") for crate in CRATES]
        missing = [shown for path, shown in sources if not path.is_dir()]
        if missing:
            print(f"key-hint check could not run: {', '.join(missing)} missing", file=sys.stderr)
            return None
        return sources
    return [(Path(args[0]), "")]


def main(argv: list[str]) -> int:
    sources = sources_of(argv)
    if sources is None:
        return 2
    problems: list[str] = []
    fixed_counts: dict[str, tuple[str, int]] = {}

    for base, shown in sources:
        for path in sorted(base.rglob("*.rs")):
            rel = path.relative_to(base).as_posix()
            name = shown + rel
            text = path.read_text(encoding="utf-8")
            # Unit tests may spell a key to assert on it.
            text = text.split("#[cfg(test)]", 1)[0]
            count = len(FIXED.findall("\n".join(line for _, line in code_lines(text))))
            if count:
                fixed_counts[name] = (rel, count)
            if rel == OWNER or rel in ALLOWED_LITERAL:
                continue
            for number, line in code_lines(text):
                if CAP_CLASS.search(line):
                    problems.append(f"  {name}:{number}: a cap built by hand: {line.strip()}")
                if LITERAL_HINT.search(line):
                    problems.append(f"  {name}:{number}: a literal key hint: {line.strip()}")
                if RETIRED.search(line):
                    problems.append(f"  {name}:{number}: a retired key notation: {line.strip()}")

    for name, (rel, count) in sorted(fixed_counts.items()):
        allowed = ALLOWED_FIXED.get(rel, (0, ""))[0]
        if count > allowed:
            problems.append(
                f"  {name}: {count} hints::fixed call(s), {allowed} allowed -- "
                "add the file to ALLOWED_FIXED with the reason no command carries the key"
            )

    if not problems:
        return 0
    print("Key hints must be read from the keymap (docs/PRODUCT.md §8):")
    print("\n".join(problems))
    print()
    print("Fix: postio_ui::hints::{hint, key, pair} with the command's CommandId,")
    print("drawn by widgets::keyhint (cap, labelled, chip, KeyLine). A key that is")
    print("not a command goes through hints::fixed and a line in ALLOWED_FIXED.")
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
