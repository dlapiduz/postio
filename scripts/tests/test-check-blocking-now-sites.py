#!/usr/bin/env python3
"""Self-test for scripts/checks/check-blocking-now-sites.py.

`blocking::now` in a frontend runs a future on the GTK main thread (#1608).
The desktop app is a frontend, and its presenters live in the shared crate
(ADR 0043), so a new call is refused in either crate that runs on that
thread. Throwaway repositories in a temp dir, one per way
the rule holds or breaks, and an assertion for each. The real repository is
never touched.

Usage: scripts/tests/test-check-blocking-now-sites.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

CHECK = Path(__file__).resolve().parent.parent / "checks" / "check-blocking-now-sites.py"
FAILURES: list[str] = []
CRATES = ("postio-widgets", "postio-focus")


def expect(name: str, files: dict[str, str], ok: bool, *seen: str, crates=CRATES) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        for crate in crates:
            (root / "crates" / crate / "src").mkdir(parents=True)
        for rel, text in files.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        result = subprocess.run(
            [sys.executable, str(CHECK), "--root", str(root), "--allow", "{}"],
            capture_output=True,
            text=True,
        )
    output = result.stdout + result.stderr
    if (result.returncode == 0) != ok:
        FAILURES.append(f"{name}: exit {result.returncode}\n{output}")
    for text in seen:
        if text not in output:
            FAILURES.append(f"{name}: output lacks {text!r}\n{output}")


expect("no call passes", {"crates/postio-focus/src/window.rs": "fn f() {}"}, True)
expect(
    "a comment naming it passes",
    {"crates/postio-focus/src/window.rs": "// blocking::now(read) used to be here"},
    True,
)
expect(
    "a call planted in postio-widgets fails",
    {"crates/postio-widgets/src/present/reading.rs": "let x = blocking::now(read());"},
    False,
    "crates/postio-widgets/src/present/reading.rs: 1 call(s) to blocking::now, 0 allowed",
)
expect(
    "a call planted in postio-focus fails",
    {"crates/postio-focus/src/window.rs": "let x = blocking::now(read());"},
    False,
    "crates/postio-focus/src/window.rs: 1 call(s) to blocking::now, 0 allowed",
)
expect(
    "a frontend crate that is missing is a check that cannot run",
    {},
    False,
    "crates/postio-focus/src is missing",
    crates=("postio-widgets",),
)

if FAILURES:
    print("\n".join(FAILURES))
    sys.exit(1)
print("check-blocking-now-sites: all cases behaved")
