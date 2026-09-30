#!/usr/bin/env python3
"""Self-test for scripts/checks/check-no-dead-css.py.

Throwaway trees in a temp dir, one per way a class can be used or not, and an
assertion for each. The real repository is never touched.

Usage: scripts/tests/test-check-no-dead-css.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

CHECK = Path(__file__).resolve().parent.parent / "checks" / "check-no-dead-css.py"
FAILURES: list[str] = []


def run(css: str, files: dict[str, str]) -> subprocess.CompletedProcess[str]:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "data").mkdir()
        (root / "data" / "shell.css").write_text(css)
        for name, text in files.items():
            path = root / "crates" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        (root / "crates").mkdir(exist_ok=True)
        return subprocess.run(
            [sys.executable, str(CHECK), str(root / "data"), str(root / "crates")],
            capture_output=True,
            text=True,
        )


def expect(name: str, result: subprocess.CompletedProcess[str], ok: bool, *seen: str) -> None:
    if (result.returncode == 0) != ok:
        FAILURES.append(f"{name}: exit {result.returncode}, wanted {'0' if ok else '1'}\n{result.stdout}")
    for text in seen:
        if text not in result.stdout:
            FAILURES.append(f"{name}: output lacks {text!r}\n{result.stdout}")


expect(
    "a literal class is used",
    run(".postio-row { color: red; }", {"gtk/src/row.rs": 'w.add_css_class("postio-row");'}),
    True,
)
expect(
    "an unset class is dead",
    run(
        ".postio-row, .postio-gone { color: red; }",
        {"gtk/src/row.rs": 'w.add_css_class("postio-row");'},
    ),
    False,
    ".postio-gone",
)
expect(
    "a comment line is not a use",
    run(".postio-gone {}", {"gtk/src/row.rs": '// "postio-gone" used to live here'}),
    False,
    ".postio-gone",
)
expect(
    "a commented-out rule is not a selector",
    run("/* .postio-gone {} */ .postio-row {}", {"gtk/src/row.rs": '"postio-row"'}),
    True,
)
expect(
    "a composed prefix covers its family",
    run(".postio-account-3 {}", {"gtk/src/a.rs": 'format!("postio-account-{index}")'}),
    True,
)
expect(
    "a composed suffix covers its family",
    run(".postio-keycap-hint {}", {"gtk/src/k.rs": 'format!("{class}-hint")'}),
    True,
)
expect(
    "a test's composed name is not a class",
    run(".postio-settings-header {}", {"gtk/tests/s.rs": 'format!("postio-settings-{}", 1)'}),
    False,
    ".postio-settings-header",
)
expect(
    "a test that looks a class up is a use",
    run(".postio-mono {}", {"gtk/tests/style.rs": 'l.add_css_class("postio-mono");'}),
    True,
)
expect(
    "the token generator writing a rule is not a use",
    run(".postio-meta {}", {"postio-ui/src/tokens.rs": '".postio-meta {{"'}),
    False,
    ".postio-meta",
)

if FAILURES:
    print("\n".join(FAILURES))
    sys.exit(1)
print("check-no-dead-css: all cases behaved")
