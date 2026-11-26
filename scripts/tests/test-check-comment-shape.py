#!/usr/bin/env python3
"""Self-test for scripts/checks/check-comment-shape.py.

Throwaway trees with a copy of the check, one case per rule in each
direction plus the baseline ratchet. The real repository is never touched.

Usage: scripts/tests/test-check-comment-shape.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
CHECK = REPO_ROOT / "scripts" / "checks" / "check-comment-shape.py"
SANDBOXES = REPO_ROOT / "target" / "tmp"
SANDBOXES.mkdir(parents=True, exist_ok=True)
FAILURES: list[str] = []

SRC = "crates/postio-thing/src/lib.rs"
TEST = "crates/postio-thing/tests/suite/a.rs"


def run(files: dict[str, str], baseline: str, *args: str):
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
        root = Path(directory)
        (root / "scripts" / "checks").mkdir(parents=True)
        shutil.copy(CHECK, root / "scripts" / "checks" / CHECK.name)
        (root / "scripts" / "checks" / "comment-shape-baseline.txt").write_text(baseline)
        for rel, text in files.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        result = subprocess.run(
            [sys.executable, str(root / "scripts" / "checks" / CHECK.name), *args],
            capture_output=True, text=True,
        )
        written = (root / "scripts" / "checks" / "comment-shape-baseline.txt").read_text()
    return result, written


def expect(name: str, files: dict[str, str], baseline: str, ok: bool, *seen: str) -> None:
    result, _ = run(files, baseline)
    if (result.returncode == 0) != ok:
        FAILURES.append(f"{name}: exit {result.returncode}\n{result.stdout}{result.stderr}")
    for text in seen:
        if text not in result.stdout:
            FAILURES.append(f"{name}: output lacks {text!r}\n{result.stdout}")


def block(marker: str, n: int) -> str:
    return "".join(f"{marker} line {i}\n" for i in range(n)) + "fn f() {}\n"


expect("three lines pass", {SRC: block("//", 3)}, "", True)
expect("four lines fail", {SRC: block("//", 4)}, "", False, f"{SRC}:1: long-block")
expect("four doc lines fail", {SRC: block("///", 4)}, "", False, "long-block")
expect("mixed markers are separate blocks", {SRC: "// a\n// b\n// c\n/// d\n/// e\n/// f\nfn f() {}\n"}, "", True)
expect("a fenced doc example passes", {SRC: "/// Use:\n/// ```\n/// f();\n/// ```\n/// done\nfn f() {}\n"}, "", True)
expect("POSTIO- lines are not counted", {SRC: "// a\n// b\n// c\n// POSTIO-MEASUREMENT: x\nfn f() {}\n"}, "", True)
expect("trailing comments are not blocks", {SRC: "let a = 1; // a\n" * 5}, "", True)
expect("a header in a test file fails", {TEST: "//! Header.\nfn f() {}\n"}, "", False, f"{TEST}:1: test-header")
expect("a test file without a header passes", {TEST: "// why\nfn f() {}\n"}, "", True)
expect("a long test header counts once", {TEST: "//! a\n//! b\nfn f() {}\n"}, f"{TEST} 1\n", True)
expect("a header in src passes", {SRC: "//! Header.\nfn f() {}\n"}, "", True)
expect("FR id fails", {SRC: "// per FR-018\nfn f() {}\n"}, "", False, f"{SRC}:1: task-id")
expect("SC id fails", {SRC: "// deleted, SC-012\n"}, "", False, "task-id")
expect("T id fails", {SRC: "// see T041\n"}, "", False, "task-id")
expect("issue ref and ADR pass", {SRC: "// as in #123 and (ADR 0049)\n// T1234 and FR-x\n"}, "", True)
expect("a rule of dashes fails", {SRC: "// ----\n"}, "", False, f"{SRC}:1: divider")
expect("a banner fails", {SRC: "// ---- helpers ----\n"}, "", False, "divider")
expect("a box line fails", {SRC: "// ────────\n"}, "", False, "divider")
expect("MARK dash fails", {"macos/Sources/A.swift": "// MARK: - Helpers\n"}, "", False, "divider")
expect("MARK without dash passes", {"macos/Sources/A.swift": "// MARK: Helpers\n"}, "", True)
expect("a hyphen in prose passes", {SRC: "// a well-known case -- really\n"}, "", True)
expect("swift long block fails", {"macos/Sources/A.swift": block("//", 5)}, "", False, "macos/Sources/A.swift:1")

ONE = block("//", 4)
expect("at the baseline passes", {SRC: ONE}, f"{SRC} 1\n", True)
expect("over the baseline fails", {SRC: ONE + ONE}, f"{SRC} 1\n", False, f"{SRC}:6: long-block", "three lines")
expect("an unlisted file fails", {SRC: ONE}, "", False, "baseline 0")
expect("under the baseline says to lower it", {SRC: ONE + ONE}, f"{SRC} 3\n", True, "lower its line")
expect("a cleaned file says to drop its line", {SRC: "// ok\n"}, f"{SRC} 1\n", True, "remove its line")

result, written = run({SRC: ONE + ONE, TEST: "//! h\n"}, f"{SRC} 9\n", "--update")
if result.returncode != 0 or written != f"{SRC} 2\n{TEST} 1\n":
    FAILURES.append(f"--update: exit {result.returncode}, wrote {written!r}")
result, written = run({SRC: "// ok\n"}, f"{SRC} 9\n", "--update")
if written != "":
    FAILURES.append(f"--update does not lower: {written!r}")

for failure in FAILURES:
    print("FAIL", failure)
print("comment-shape self-test:", "FAILED" if FAILURES else "ok")
sys.exit(1 if FAILURES else 0)
