#!/usr/bin/env python3
"""Self-test for scripts/checks/check-spacing-literals-ratchet.py.

Throwaway source trees and baselines, one per way the ratchet can hold or
slip, and an assertion for each. The real repository is never touched.

Usage: scripts/tests/test-check-spacing-literals-ratchet.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

CHECK = Path(__file__).resolve().parent.parent / "checks" / "check-spacing-literals-ratchet.py"
FAILURES: list[str] = []


def expect(name: str, files: dict[str, str], baseline: str, ok: bool, *seen: str) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp) / "src"
        for rel, text in files.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        root.mkdir(exist_ok=True)
        base = Path(tmp) / "baseline.txt"
        base.write_text(baseline)
        result = subprocess.run(
            [sys.executable, str(CHECK), str(root), str(base)], capture_output=True, text=True
        )
    if (result.returncode == 0) != ok:
        FAILURES.append(f"{name}: exit {result.returncode}\n{result.stdout}")
    for text in seen:
        if text not in result.stdout:
            FAILURES.append(f"{name}: output lacks {text!r}\n{result.stdout}")


TWO = "w.set_margin_top(18);\nlet b = gtk::Box::new(gtk::Orientation::Vertical, 6);\n"
expect("at the baseline passes", {"settings.rs": TWO}, "settings.rs 2\n", True)
expect("a new literal fails", {"settings.rs": TWO + "x.set_spacing(4);\n"}, "settings.rs 2\n", False, "settings.rs: 3")
expect("a new file with a literal fails", {"header.rs": "w.set_margin_end(8);"}, "", False, "header.rs")
expect("the ramp is not a literal", {"header.rs": "w.set_margin_end(space::S2);"}, "", True)
expect("zero is not a literal", {"header.rs": "gtk::Box::new(gtk::Orientation::Vertical, 0);"}, "", True)
expect("widgets/ is exempt", {"widgets/plate.rs": "w.set_margin_top(18);"}, "", True)
expect("unit tests are exempt", {"a.rs": "#[cfg(test)]\nmod t { fn f() { w.set_margin_top(3); } }"}, "", True)
expect(
    "fewer than the baseline passes and says to lower it",
    {"settings.rs": "w.set_margin_top(18);"},
    "settings.rs 2\n",
    True,
    "lower the baseline",
)

if FAILURES:
    print("\n".join(FAILURES))
    sys.exit(1)
print("check-spacing-literals-ratchet: all cases behaved")
