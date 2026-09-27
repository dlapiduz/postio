#!/usr/bin/env python3
"""Self-test for scripts/checks/check-shadows-use-tokens.py.

One throwaway stylesheet per way a shadow can be written, and an assertion
for each. The real repository is never touched.

Usage: scripts/tests/test-check-shadows-use-tokens.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

CHECK = Path(__file__).resolve().parent.parent / "checks" / "check-shadows-use-tokens.py"
FAILURES: list[str] = []


def expect(name: str, css: str, ok: bool) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        sheet = Path(tmp) / "shell.css"
        sheet.write_text(css)
        result = subprocess.run(
            [sys.executable, str(CHECK), str(sheet)], capture_output=True, text=True
        )
    if (result.returncode == 0) != ok:
        FAILURES.append(f"{name}: exit {result.returncode}\n{result.stdout}")


def expect_root(name: str, sheets: dict[str, str], ok: bool, *seen: str) -> None:
    """The check over a whole repository: every desktop crate's stylesheets."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "crates" / "postio-gtk" / "data").mkdir(parents=True)
        (root / "crates" / "postio-gtk" / "data" / "shell.css").write_text(".a {}\n")
        for rel, css in sheets.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(css)
        result = subprocess.run(
            [sys.executable, str(CHECK), "--root", str(root)], capture_output=True, text=True
        )
    if (result.returncode == 0) != ok:
        FAILURES.append(f"{name}: exit {result.returncode}\n{result.stdout}{result.stderr}")
    for text in seen:
        if text not in result.stdout:
            FAILURES.append(f"{name}: output lacks {text!r}\n{result.stdout}")


expect("a token shadow passes", ".a { box-shadow: var(--postio-shadow-lg); }", True)
expect("an inset hairline passes", ".a { box-shadow: inset 0 -1px var(--postio-hairline); }", True)
expect("none passes", ".a { box-shadow: none; }", True)
expect("an rgba literal fails", ".a { box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18); }", False)
expect("a hex literal fails", ".a { box-shadow: 0 1px 2px #000; }", False)
expect(
    "a literal in a multi-line value fails",
    ".a {\n  box-shadow:\n    inset 0 1px var(--postio-hairline),\n    0 2px 4px rgb(0 0 0 / 20%);\n}",
    False,
)
expect("a commented-out literal passes", "/* box-shadow: 0 1px rgba(0,0,0,.2); */ .a {}", True)

# Both desktop apps carry stylesheets, and the shared crate carries the widget
# rules both of them draw with (ADR 0043; specs/007-postio-focus R1).
LITERAL = ".a { box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18); }"
expect_root(
    "a literal shadow in the classic app's stylesheet fails",
    {"crates/postio-gtk/data/shell.css": LITERAL},
    False,
    "crates/postio-gtk/data/shell.css:1",
)
expect_root(
    "a literal shadow planted in postio-widgets' stylesheet fails",
    {"crates/postio-widgets/data/widgets.css": LITERAL},
    False,
    "crates/postio-widgets/data/widgets.css:1",
)
expect_root(
    "a literal shadow planted in postio-focus' stylesheet fails",
    {"crates/postio-focus/data/focus.css": LITERAL},
    False,
    "crates/postio-focus/data/focus.css:1",
)
expect_root(
    "the generated token sheet is where the scale is defined",
    {"crates/postio-gtk/data/tokens.css": ":root { --postio-shadow-sm: 0 1px 2px rgba(0,0,0,.1); } .b { box-shadow: 0 1px #000; }"},
    True,
)
expect_root(
    "a token shadow in the shared crate passes",
    {"crates/postio-widgets/data/widgets.css": ".a { box-shadow: var(--postio-shadow-md); }"},
    True,
)

if FAILURES:
    print("\n".join(FAILURES))
    sys.exit(1)
print("check-shadows-use-tokens: all cases behaved")
