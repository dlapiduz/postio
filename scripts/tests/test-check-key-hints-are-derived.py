#!/usr/bin/env python3
"""Self-test for scripts/checks/check-key-hints-are-derived.py.

Throwaway source trees in a temp dir, one per way a key hint can be derived
or typed in, and an assertion for each. The real repository is never touched.

Usage: scripts/tests/test-check-key-hints-are-derived.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

CHECK = Path(__file__).resolve().parent.parent / "checks" / "check-key-hints-are-derived.py"
FAILURES: list[str] = []


def run(files: dict[str, str]) -> subprocess.CompletedProcess[str]:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        for name, text in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        return subprocess.run(
            [sys.executable, str(CHECK), str(root)], capture_output=True, text=True
        )


def expect(name: str, files: dict[str, str], ok: bool, *seen: str) -> None:
    result = run(files)
    if (result.returncode == 0) != ok:
        FAILURES.append(f"{name}: exit {result.returncode}\n{result.stdout}")
    for text in seen:
        if text not in result.stdout:
            FAILURES.append(f"{name}: output lacks {text!r}\n{result.stdout}")


expect(
    "a derived hint passes",
    {"header.rs": 'keyhint::labelled("Compose", hints::key(k, CommandId::Compose).as_deref());'},
    True,
)
expect(
    "a literal labelled hint fails",
    {"header.rs": 'labelled("Compose", "c");'},
    False,
    "header.rs:1: a literal key hint",
)
expect(
    "a literal Some() hint fails",
    {"parts.rs": 'keyhint::labelled("Save", Some("s"));'},
    False,
    "parts.rs:1",
)
expect(
    "a literal set_key fails",
    {"unavailable.rs": 'retry.set_key(Some("Ret"));'},
    False,
    "unavailable.rs:1",
)
expect(
    "a hand-built cap fails outside the owner",
    {"list_state.rs": 'key.add_css_class("postio-keyhint");'},
    False,
    "a cap built by hand",
)
expect(
    "the owner builds caps",
    {"widgets/keyhint.rs": 'label.add_css_class("postio-keyhint");'},
    True,
)
expect(
    "a retired notation fails",
    {"composer.rs": 'let hint = "C-⇧-A";'},
    False,
    "a retired key notation",
)
expect(
    "a comment may name a key",
    {"composer.rs": '// these were `labelled("Send", "C-Ret")` until #828'},
    True,
)
expect(
    "a unit test may spell a key",
    {"parts.rs": '#[cfg(test)]\nmod tests { fn t() { labelled("Save", "s"); } }'},
    True,
)
expect(
    "an allowed fixed hint passes",
    {"search.rs": 'hints::fixed("Tab", "refine", "focus order");'},
    True,
)
expect(
    "an unlisted fixed hint fails",
    {"header.rs": 'hints::fixed("c", "Compose", "because");'},
    False,
    "header.rs: 1 hints::fixed call(s), 0 allowed",
)

if FAILURES:
    print("\n".join(FAILURES))
    sys.exit(1)
print("check-key-hints-are-derived: all cases behaved")
