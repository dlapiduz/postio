#!/usr/bin/env python3
"""Refuse a self-test that runs cargo in a fixture without the pinned compiler.

A throwaway Cargo project in a temp dir is outside the repository, so rustup
finds no `rust-toolchain.toml` and uses its default toolchain. The test then
proves nothing about the compiler the repository pins: `test-target-sweep.py`
stayed green while the real sweep had stopped working on the pinned one.

The rule, deliberately crude: a self-test (`scripts/tests/*.py`,
`.claude/hooks/test-*.py`) that both writes a `Cargo.toml` and runs real cargo
(a `["cargo", ...]` command, or a `shutil.which("cargo")` / prereq guard for a
script that runs it) must call `fixture_toolchain.write(<fixture dir>)` from
`scripts/lib/fixture_toolchain.py`. Naming `rust-toolchain.toml` in the file (a
hand-copied pin) also satisfies it; the helper is the preferred spelling.

Usage: scripts/checks/check-fixture-toolchain.py [repo-root]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

WRITES_MANIFEST = re.compile(r"Cargo\.toml")
RUNS_CARGO = re.compile(r"""\[\s*["']cargo["']|which\(\s*["']cargo["']|(?:require|available|have)\(\s*["']cargo["']""")
USES_HELPER = re.compile(r"fixture_toolchain\.write\(|rust-toolchain\.toml")


def offenders(root: Path) -> list[Path]:
    files = sorted((root / "scripts" / "tests").glob("*.py"))
    files += sorted((root / ".claude" / "hooks").glob("test-*.py"))
    bad = []
    for path in files:
        text = path.read_text(encoding="utf-8")
        if WRITES_MANIFEST.search(text) and RUNS_CARGO.search(text) and not USES_HELPER.search(text):
            bad.append(path)
    return bad


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parent.parent.parent
    bad = offenders(root)
    if not bad:
        return 0
    for path in bad:
        print(f"{path.relative_to(root)}: writes a Cargo.toml and runs cargo without the pinned toolchain", file=sys.stderr)
    print(
        "\nA fixture outside the repository gets rustup's default compiler, not the pinned\n"
        "one, so the test can stay green while the real script is broken. Do this instead:\n"
        "    sys.path.insert(0, str(Path(__file__).resolve().parent.parent / \"lib\"))\n"
        "    import fixture_toolchain\n"
        "    fixture_toolchain.write(fixture_root)   # before the first cargo run\n"
        "(scripts/lib/fixture_toolchain.py copies the repository's rust-toolchain.toml.)",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
