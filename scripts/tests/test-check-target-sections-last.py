#!/usr/bin/env python3
"""Prove `check-target-sections-last.py` fails on the manifest that broke main.

The first case is #642 reconstructed verbatim in miniature: a platform header
inserted into the middle of a sorted dependency list, which is how it got past
review — it reads as a tidy alphabetical insert.

A guard that has never been seen to fail is a guard nobody should trust.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CHECK = ROOT / "scripts" / "checks" / "check-target-sections-last.py"

WORKSPACE = '[workspace]\nmembers = ["crates/leaf"]\n'

# #642: the header lands mid-list and swallows everything below it, including
# the whole of [build-dependencies] and [dev-dependencies] that follow.
SWALLOWING = """\
[package]
name = "leaf"

[dependencies]
anyhow = "1"
[target.'cfg(target_os = "macos")'.dependencies]
security-framework = "3"
serde = "1"
tokio = "1"

[build-dependencies]
cc = "1"
"""

# The same dependencies, with the platform table where it cannot swallow.
SAFE = """\
[package]
name = "leaf"

[dependencies]
anyhow = "1"
serde = "1"
tokio = "1"

[build-dependencies]
cc = "1"

[target.'cfg(target_os = "macos")'.dependencies]
security-framework = "3"
"""

# A crate with no platform section at all: the ordinary case, must pass.
PLAIN = """\
[package]
name = "leaf"

[dependencies]
anyhow = "1"

[dev-dependencies]
tempfile = "3"
"""


def sandbox(tmp: Path, manifest: str) -> Path:
    root = tmp / "tree"
    (root / "crates" / "leaf").mkdir(parents=True)
    (root / "scripts" / "checks").mkdir(parents=True)
    (root / "Cargo.toml").write_text(WORKSPACE)
    (root / "crates" / "leaf" / "Cargo.toml").write_text(manifest)
    shutil.copy(CHECK, root / "scripts" / "checks" / CHECK.name)
    return root


def run(root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(root / "scripts" / "checks" / CHECK.name)],
        capture_output=True,
        text=True,
        timeout=120,
    )


def case_swallowing_manifest_fails() -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        result = run(sandbox(Path(tmp), SWALLOWING))
        if result.returncode == 0:
            print("  FAIL: the #642 shape was accepted", file=sys.stderr)
            return False
        if "build-dependencies" not in result.stderr:
            print(
                f"  FAIL: the message does not name what was swallowed: "
                f"{result.stderr!r}",
                file=sys.stderr,
            )
            return False
    return True


def case_platform_section_at_the_foot_passes() -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        result = run(sandbox(Path(tmp), SAFE))
        if result.returncode != 0:
            print(
                f"  FAIL: a correctly placed platform section was rejected:\n"
                f"{result.stderr}",
                file=sys.stderr,
            )
            return False
    return True


def case_manifest_without_platform_sections_passes() -> bool:
    with tempfile.TemporaryDirectory() as tmp:
        result = run(sandbox(Path(tmp), PLAIN))
        if result.returncode != 0:
            print(f"  FAIL: an ordinary manifest was rejected:\n{result.stderr}",
                  file=sys.stderr)
            return False
    return True


def main() -> int:
    cases = [
        ("the #642 shape fails", case_swallowing_manifest_fails),
        ("a platform section at the foot passes", case_platform_section_at_the_foot_passes),
        ("a manifest with no platform section passes", case_manifest_without_platform_sections_passes),
    ]
    failed = 0
    for name, case in cases:
        print(f"case: {name}")
        if not case():
            failed += 1

    if failed:
        print(f"\n{failed} case(s) failed.", file=sys.stderr)
        return 1
    print("test-check-target-sections-last: all cases passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
