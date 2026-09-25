#!/usr/bin/env python3
"""Every key hint the GTK frontend draws is read from the keymap.

docs/PRODUCT.md §8: the key hints are derived from the command registry, the
same one table the keymap, the palette and the cheat sheet read. A hint typed
in as a literal goes on naming its key after a `[keys]` rebind has moved the
command somewhere else -- `Compose c`, `Render once H` and `Attach another
C-⇧-A` all did (#828 fixed three more before them). A hint that lies is
worse than none.

So in `crates/postio-gtk/src` this refuses:

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
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCES = ROOT / "crates/postio-gtk/src"
OWNER = "widgets/keyhint.rs"

# file (relative to crates/postio-gtk/src) -> (count, why no command carries it)
ALLOWED_FIXED: dict[str, tuple[int, str]] = {
    "onboarding.rs": (
        2,
        "Return submits the sign-in form from any field, and Tab moves between "
        "its fields: both are the toolkit's, not registry commands",
    ),
    "unavailable.rs": (1, "Return is the default action of the screen's only button"),
    "search.rs": (1, "Tab into the refine column is the toolkit's focus order"),
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


def main(argv: list[str]) -> int:
    sources = Path(argv[1]) if len(argv) == 2 else SOURCES
    problems: list[str] = []
    fixed_counts: dict[str, int] = {}

    for path in sorted(sources.rglob("*.rs")):
        rel = path.relative_to(sources).as_posix()
        text = path.read_text(encoding="utf-8")
        # Unit tests may spell a key to assert on it.
        text = text.split("#[cfg(test)]", 1)[0]
        count = len(FIXED.findall("\n".join(line for _, line in code_lines(text))))
        if count:
            fixed_counts[rel] = count
        if rel == OWNER or rel in ALLOWED_LITERAL:
            continue
        for number, line in code_lines(text):
            if CAP_CLASS.search(line):
                problems.append(f"  {rel}:{number}: a cap built by hand: {line.strip()}")
            if LITERAL_HINT.search(line):
                problems.append(f"  {rel}:{number}: a literal key hint: {line.strip()}")
            if RETIRED.search(line):
                problems.append(f"  {rel}:{number}: a retired key notation: {line.strip()}")

    for rel, count in sorted(fixed_counts.items()):
        allowed = ALLOWED_FIXED.get(rel, (0, ""))[0]
        if count > allowed:
            problems.append(
                f"  {rel}: {count} hints::fixed call(s), {allowed} allowed -- "
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
