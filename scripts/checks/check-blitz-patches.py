#!/usr/bin/env python3
"""Refuse a `vendor/blitz-*` that is not the release plus `patches/blitz/`.

Blitz builds from `vendor/`, and reviewers read `patches/blitz/`: a hand
edit to the vendored source that never became a patch is a change nobody
reviewed, and one the next upstream upgrade silently drops.
`scripts/blitz-patches.sh verify` rebuilds the tree from the registry's
release (cargo's cache, or crates.io) and compares.
"""

import subprocess
import sys
from pathlib import Path


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    result = subprocess.run(
        [str(root / "scripts" / "blitz-patches.sh"), "verify"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        print("blitz-patches check FAILED:\n")
        print((result.stdout + result.stderr).strip())
        print(
            "\nFix: save the change as the next patch (`scripts/blitz-patches.sh diff`,"
            "\nkeeping only the new hunks) and list it in patches/blitz/series, or undo it."
        )
        return 1
    print(f"blitz-patches check passed ({result.stdout.strip().removeprefix('blitz-patches: ')}).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
