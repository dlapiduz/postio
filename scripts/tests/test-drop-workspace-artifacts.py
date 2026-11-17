#!/usr/bin/env python3
"""Self-test for scripts/lib/drop-workspace-artifacts.sh.

What a CI cache keeps of `target/` is the dependencies; the workspace's own
crates rebuild on every run, so carrying them only spends the repository's
10 GB cache quota (and, for the macOS release cache, got it evicted). This
holds both halves: every product of ours goes, every dependency's stays.

Usage: scripts/tests/test-drop-workspace-artifacts.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

LIB = Path(__file__).resolve().parent.parent / "lib" / "drop-workspace-artifacts.sh"

OURS = [
    "release/libpostio_ffi.a",
    "release/libpostio_ffi.dylib",
    "release/postio",
    "release/postio-mcp",
    "release/deps/libpostio_ffi-0123.rlib",
    "release/deps/postio_mcp-0123",
    "release/.fingerprint/postio-ffi-0123/lib",
    "release/build/postio-ffi-0123/out/x",
    "debug/incremental/postio_ffi-1/s",
]
THEIRS = [
    "release/deps/libserde-0123.rlib",
    "release/.fingerprint/serde-0123/lib",
    "release/build/ring-0123/out/x",
    "release/uniffi-bindgen",
    "release/libring.a",
    "nextest/ci/junit.xml",
]


def main() -> int:
    failures = []
    with tempfile.TemporaryDirectory() as tmp:
        target = Path(tmp) / "target"
        for path in OURS + THEIRS:
            (target / path).parent.mkdir(parents=True, exist_ok=True)
            (target / path).write_text("x", encoding="utf-8")
        done = subprocess.run(
            ["bash", "-c", f'set -euo pipefail; source "{LIB}"; drop_workspace_artifacts "{target}"'],
            capture_output=True, text=True,
        )
        if done.returncode != 0:
            failures.append(f"exit {done.returncode}: {done.stderr}")
        for path in OURS:
            if (target / path).exists():
                failures.append(f"{path} is ours and was kept")
            else:
                print(f"ok    drops {path}")
        for path in THEIRS:
            if not (target / path).exists():
                failures.append(f"{path} is not ours and was dropped")
            else:
                print(f"ok    keeps {path}")
    if failures:
        print("\nFAILED:", *failures, sep="\n  ")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
