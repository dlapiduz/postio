#!/usr/bin/env python3
"""Self-test for scripts/checks/check-reader-header-has-one-home.py.

The header's toolkit-free rules live in `postio_ui::reader::header`, and a
desktop crate that defines its own copy of one fails. The reader the
desktop app draws lives in the shared crate (ADR 0043), so a private copy
can reappear in either crate. Throwaway
repositories in a temp dir, one per way the rule holds or breaks, and an
assertion for each. The real repository is never touched.

Usage: scripts/tests/test-check-reader-header-has-one-home.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

CHECK = Path(__file__).resolve().parent.parent / "checks" / "check-reader-header-has-one-home.py"
FAILURES: list[str] = []

OWNER = """
pub const NO_SUBJECT: &str = "(no subject)";
pub fn address_line() {}
pub fn address_list() {}
pub fn subject_text() {}
pub fn absolute_date() {}
"""


def expect(name: str, files: dict[str, str], ok: bool, *seen: str, owner: bool = True) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        for crate in ("postio-widgets", "postio-focus"):
            (root / "crates" / crate / "src").mkdir(parents=True)
        if owner:
            files = {"crates/postio-ui/src/reader/header.rs": OWNER, **files}
        for rel, text in files.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        result = subprocess.run(
            [sys.executable, str(CHECK), "--root", str(root)], capture_output=True, text=True
        )
    if (result.returncode == 0) != ok:
        FAILURES.append(f"{name}: exit {result.returncode}\n{result.stdout}{result.stderr}")
    for text in seen:
        if text not in result.stdout:
            FAILURES.append(f"{name}: output lacks {text!r}\n{result.stdout}")


expect("one home passes", {}, True)
expect(
    "a call through to the shared rules passes",
    {"crates/postio-widgets/src/reader/message_header.rs": "let s = header::subject_text(m);"},
    True,
)
expect(
    "no shared home fails",
    {},
    False,
    "crates/postio-ui/src/reader/header.rs is missing",
    owner=False,
)
expect(
    "a private copy planted in postio-widgets fails",
    {"crates/postio-widgets/src/reader/message_header.rs": "fn address_line() {}"},
    False,
    "crates/postio-widgets/src/reader/message_header.rs: defines address_line",
)
expect(
    "a private copy planted in postio-focus fails",
    {"crates/postio-focus/src/open.rs": 'const NO_SUBJECT: &str = "";'},
    False,
    "crates/postio-focus/src/open.rs: defines NO_SUBJECT",
)

if FAILURES:
    print("\n".join(FAILURES))
    sys.exit(1)
print("check-reader-header-has-one-home: all cases behaved")
