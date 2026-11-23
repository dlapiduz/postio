#!/usr/bin/env python3
"""Self-test for scripts/checks/check-tui-update-is-pure.py.

Throwaway trees with a copy of the check: a filesystem call in the app or a
view fails; the same call in run.rs, a test module or a comment passes.

Usage: scripts/tests/test-check-tui-update-is-pure.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
CHECK = REPO_ROOT / "scripts" / "checks" / "check-tui-update-is-pure.py"
SANDBOXES = REPO_ROOT / "target" / "tmp"
SANDBOXES.mkdir(parents=True, exist_ok=True)
FAILURES: list[str] = []
SRC = "crates/postio-tui/src"


def expect(name: str, files: dict[str, str], ok: bool, *seen: str) -> None:
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
        root = Path(directory)
        (root / "scripts" / "checks").mkdir(parents=True)
        shutil.copy(CHECK, root / "scripts" / "checks" / CHECK.name)
        for rel in (f"{SRC}/app.rs", f"{SRC}/app/list.rs", f"{SRC}/view/list.rs"):
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            (root / rel).write_text("fn pure() {}\n")
        for rel, text in files.items():
            (root / rel).parent.mkdir(parents=True, exist_ok=True)
            (root / rel).write_text(text)
        result = subprocess.run([sys.executable, str(root / "scripts" / "checks" / CHECK.name)], capture_output=True, text=True)
    if (result.returncode == 0) != ok:
        FAILURES.append(f"{name}: exit {result.returncode}\n{result.stdout}{result.stderr}")
    for text in seen:
        if text not in result.stdout:
            FAILURES.append(f"{name}: output lacks {text!r}\n{result.stdout}")


expect("a pure app passes", {}, True)
expect("a config write in an app module fails",
       {f"{SRC}/app/bar.rs": "fn save() { postio_focus::save_search(crate::config_file::path().as_deref(), &s); }\n"},
       False, f"{SRC}/app/bar.rs:1")
expect("std::fs in app.rs fails", {f"{SRC}/app.rs": "fn f() { std::fs::read_to_string(p); }\n"}, False, "app.rs:1")
expect("a write in a view fails", {f"{SRC}/view/list.rs": "use std::fs;\nfn f() { fs::write(p, b); }\n"}, False, "view/list.rs:2")
expect("run.rs performs effects", {f"{SRC}/run.rs": "fn perform() { std::fs::write(p, b); }\n"}, True)
expect("a test module may", {f"{SRC}/app.rs": "fn f() {}\n#[cfg(test)]\nmod tests { fn t() { std::fs::write(p, b); } }\n"}, True)
expect("a comment may name it", {f"{SRC}/app.rs": "// run.rs calls config_file::path()\nfn f() {}\n"}, True)

if FAILURES:
    print("\n\n".join(FAILURES))
    sys.exit(1)
print("tui-update-is-pure self-test: ok")
