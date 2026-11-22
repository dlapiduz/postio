#!/usr/bin/env python3
"""Refuse a `vendor/` crate that is not its release plus its `patches/` queue.

`vendor/` is what builds, and reviewers read `patches/<queue>/`: a hand
edit to a vendored source that never became a patch is a change nobody
reviewed, and one the next upstream upgrade silently drops.
`scripts/vendor-patches.sh verify` rebuilds every queue's crates from the
registry's release (cargo's cache, or crates.io) and compares.
"""

import subprocess
import sys
from pathlib import Path


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    result = subprocess.run(
        [str(root / "scripts" / "vendor-patches.sh"), "verify"],
        cwd=root,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        print("vendor-patches check FAILED:\n")
        print((result.stdout + result.stderr).strip())
        print(
            "\nFix: save the change as the next patch of its queue"
            "\n(`scripts/vendor-patches.sh diff <queue>`, keeping only the new hunks)"
            "\nand list it in patches/<queue>/series, or undo it."
        )
        return 1
    queues = "; ".join(
        line.removeprefix("vendor-patches: ") for line in result.stdout.strip().splitlines()
    )
    print(f"vendor-patches check passed ({queues}).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
