#!/usr/bin/env python3
"""Self-test for scripts/checks/check-buttons-have-a-kind.py.

Throwaway source trees, one per way a button can be styled, and an assertion
for each. The real repository is never touched.

Usage: scripts/tests/test-check-buttons-have-a-kind.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

CHECK = Path(__file__).resolve().parent.parent / "checks" / "check-buttons-have-a-kind.py"
FAILURES: list[str] = []


def expect(name: str, files: dict[str, str], ok: bool, *seen: str, whole: bool = False) -> None:
    """`whole`: `files` are a repository's, and the check scans all of it."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        if whole:
            for crate in ("postio-widgets", "postio-focus"):
                (root / "crates" / crate / "src").mkdir(parents=True)
        for rel, text in files.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        args = ["--root", str(root)] if whole else [str(root)]
        result = subprocess.run(
            [sys.executable, str(CHECK), *args], capture_output=True, text=True
        )
    if (result.returncode == 0) != ok:
        FAILURES.append(f"{name}: exit {result.returncode}\n{result.stdout}{result.stderr}")
    for text in seen:
        if text not in result.stdout:
            FAILURES.append(f"{name}: output lacks {text!r}\n{result.stdout}")


expect("a kind passes", {"header.rs": "button::style(&b, Kind::Primary, Size::Regular);"}, True)
expect("a raw primary fails", {"header.rs": 'b.add_css_class("suggested-action");'}, False)
expect("a raw ghost fails", {"list_view.rs": 'b.add_css_class("postio-ghost");'}, False)
expect("a retired settings class fails", {"settings.rs": 'b.add_css_class("postio-settings-primary");'}, False)
expect("widgets/ may", {"widgets/button.rs": 'b.add_css_class("suggested-action");'}, True)
expect("a comment may", {"header.rs": '// was add_css_class("suggested-action")'}, True)

# The desktop app draws buttons, from the shared crate's kinds (ADR 0043).
expect(
    "a raw class planted in postio-widgets fails",
    {"crates/postio-widgets/src/reader/view.rs": 'b.add_css_class("postio-ghost");'},
    False,
    "crates/postio-widgets/src/reader/view.rs:1",
    whole=True,
)
expect(
    "a raw class planted in postio-focus fails",
    {"crates/postio-focus/src/window.rs": 'b.add_css_class("destructive-action");'},
    False,
    "crates/postio-focus/src/window.rs:1",
    whole=True,
)
expect(
    "the shared crate's widgets/ may",
    {"crates/postio-widgets/src/widgets/button.rs": 'b.add_css_class("suggested-action");'},
    True,
    whole=True,
)

if FAILURES:
    print("\n".join(FAILURES))
    sys.exit(1)
print("check-buttons-have-a-kind: all cases behaved")
