#!/usr/bin/env python3
"""Self-test for scripts/checks/check-frontend-suite-budget.py.

Throwaway trees and baselines, one per way the budget can hold or slip,
and an assertion for each (specs/012-test-suite-value FR-011, ADR 0049).
The real repository is never touched.

Usage: scripts/tests/test-check-frontend-suite-budget.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

CHECK = Path(__file__).resolve().parent.parent / "checks" / "check-frontend-suite-budget.py"
FAILURES: list[str] = []

FFI = "crates/postio-ffi/tests/ffi_suite/keys.rs"
GTK = "crates/postio-gtk/tests/focus_suite/pickers.rs"
GTK_MAIN = "crates/postio-gtk/tests/focus_suite/main.rs"
TUI = "crates/postio-tui/src/app/find.rs"


def tests(n: int, prefix: str = "case") -> str:
    return "".join(f"#[test]\nfn {prefix}_{i}() {{\n    assert!(true);\n}}\n\n" for i in range(n))


def harness(names: list[str], extra: list[str]) -> tuple[str, str]:
    """A harness-free file with `names` in CASES and `extra` pub fns that are not cases."""
    body = "".join(f"pub fn {n}() {{}}\n" for n in names + extra)
    rows = "".join(f'    ("pickers::{n}", pickers::{n} as fn()),\n' for n in names)
    return body, f"mod pickers;\nconst CASES: &[(&str, fn())] = &[\n{rows}];\n"


def expect(name: str, files: dict[str, str], baseline: str, ok: bool, *seen: str) -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        for rel, text in files.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        base = root / "scripts" / "checks" / "frontend-suite-budget.txt"
        base.parent.mkdir(parents=True, exist_ok=True)
        base.write_text(baseline)
        result = subprocess.run([sys.executable, str(CHECK), "--root", str(root)], capture_output=True, text=True)
    out = result.stdout + result.stderr
    if (result.returncode == 0) != ok:
        FAILURES.append(f"{name}: exit {result.returncode}\n{out}")
        return
    for text in seen:
        if text not in out:
            FAILURES.append(f"{name}: output lacks {text!r}\n{out}")
            return
    print(f"ok  {name}")


expect("at the budget passes", {FFI: tests(3)}, f"{FFI} 3\n", True)
expect("over the budget fails and names ADR 0049 and the controller suite",
       {FFI: tests(4)}, f"{FFI} 3\n", False, f"{FFI}: 4 cases, budget 3", "ADR 0049",
       "crates/postio-focus/tests/controller_suite/", "frontend-suite-budget.txt")
expect("under the budget passes and says which line to lower", {FFI: tests(2)}, f"{FFI} 3\n", True,
       f"{FFI} 2")
expect("a file not in the baseline is not judged", {FFI: tests(9)}, "", True)
expect("tokio tests count", {FFI: "#[tokio::test(flavor = \"multi_thread\")]\nasync fn a() {}\n" + tests(1)},
       f"{FFI} 1\n", False, f"{FFI}: 2 cases, budget 1")
body, main = harness(["a", "b"], ["helper"])
expect("a harness-free file counts its CASES rows, not its helpers", {GTK: body, GTK_MAIN: main}, f"{GTK} 2\n", True)
body, main = harness(["a", "b", "c"], [])
expect("a new harness-free case is counted", {GTK: body, GTK_MAIN: main}, f"{GTK} 2\n", False,
       f"{GTK}: 3 cases, budget 2")
expect("a unit-test module in src counts its tests", {TUI: "fn x() {}\n#[cfg(test)]\nmod tests {\n" + tests(2) + "}\n"},
       f"{TUI} 2\n", True)
expect("a listed file that is gone says so", {}, f"{FFI} 3\n", False, f"{FFI}: listed but missing")
expect("comments and blank lines in the baseline are fine", {FFI: tests(1)}, f"# why\n\n{FFI} 1\n", True)

if FAILURES:
    print("\n".join(FAILURES))
    sys.exit(1)
print("check-frontend-suite-budget: all cases behaved")
