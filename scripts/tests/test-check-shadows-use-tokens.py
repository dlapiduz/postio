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

if FAILURES:
    print("\n".join(FAILURES))
    sys.exit(1)
print("check-shadows-use-tokens: all cases behaved")
