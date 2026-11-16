#!/usr/bin/env python3
"""Self-test for scripts/checks/check-fixture-toolchain.py and its helper.

Usage: scripts/tests/test-check-fixture-toolchain.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-fixture-toolchain.py"
sys.path.insert(0, str(HERE / "lib"))
import fixture_toolchain  # noqa: E402

FAILURES: list[str] = []

BARE = 'write("Cargo.toml")\nsubprocess.run(["cargo", "build"])\n'
HELPED = "import fixture_toolchain\nfixture_toolchain.write(root)\n" + BARE
NO_CARGO = 'write("Cargo.toml")\n'
GUARDED = 'write("Cargo.toml")\nif shutil.which("cargo") is None: pass\n'


def run(body: str, name: str = "test-x.py") -> subprocess.CompletedProcess[str]:
    with tempfile.TemporaryDirectory() as directory:
        tests = Path(directory) / "scripts" / "tests"
        tests.mkdir(parents=True)
        (tests / name).write_text(body, encoding="utf-8")
        return subprocess.run(
            [sys.executable, str(CHECK), directory], capture_output=True, text=True
        )


def expect(name: str, result: subprocess.CompletedProcess[str], status: int, needle: str = "") -> None:
    if result.returncode != status or needle not in result.stderr:
        FAILURES.append(f"{name}: exit {result.returncode}, stderr={result.stderr!r}")


def main() -> int:
    expect("cargo in a fixture without the helper", run(BARE), 1, "fixture_toolchain.write")
    expect("a cargo guard counts as running cargo", run(GUARDED), 1, "test-x.py")
    expect("with the helper", run(HELPED), 0)
    expect("a hand-copied pin is accepted", run(BARE + 'write("rust-toolchain.toml")\n'), 0)
    expect("a manifest but no cargo", run(NO_CARGO), 0)

    with tempfile.TemporaryDirectory() as directory:
        written = fixture_toolchain.write(Path(directory))
        if written.read_text() != fixture_toolchain.PIN.read_text():
            FAILURES.append("helper did not copy the repository's rust-toolchain.toml")

    
    for failure in FAILURES:
        print(f"FAIL: {failure}", file=sys.stderr)
    print("ok" if not FAILURES else f"{len(FAILURES)} failed")
    return 1 if FAILURES else 0


if __name__ == "__main__":
    sys.exit(main())
