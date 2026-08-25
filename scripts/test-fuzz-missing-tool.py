#!/usr/bin/env python3
"""Self-test for scripts/fuzz.sh when cargo-fuzz is not installed.

`scripts/fuzz.sh` exists to make fuzzing runnable on a workstation that
cannot run it unaided — it works around the `RUSTUP_TOOLCHAIN` trap and seeds
the corpus. It did both and then fell off the end: with `cargo-fuzz` absent it
ran `cargo fuzz`, which made rustup download an entire nightly toolchain and
then failed with

    error: no such command: `fuzz`

That names the wrong cause twice over. Nothing says the missing piece is a
tool you install once, and the minutes spent downloading a toolchain suggest
the problem is the toolchain. #277 hit this trying to regenerate its own
reproducer, which the issue's instructions tell you to do with this script.

So the script checks first and says what to install. Two things are asserted
here, and the second is the one that matters: it must fail **before** touching
cargo at all, because the download is most of the wasted time.

Usage: scripts/test-fuzz-missing-tool.py
Exit status: 0 the script behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
FUZZ = HERE / "fuzz.sh"

FAILURES: list[str] = []

# Logs every call, so the test can prove cargo was never reached.
CARGO_STUB = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/cargo-calls"
echo "error: no such command: \\`fuzz\\`" >&2
exit 101
"""


def main() -> int:
    with tempfile.TemporaryDirectory() as directory:
        stub_dir = Path(directory)
        binaries = stub_dir / "bin"
        binaries.mkdir()
        cargo = binaries / "cargo"
        cargo.write_text(CARGO_STUB, encoding="utf-8")
        cargo.chmod(0o755)
        (stub_dir / "cargo-calls").write_text("", encoding="utf-8")

        # A PATH with the stub and the system tools the script needs, and
        # deliberately without ~/.cargo/bin -- which is where a real
        # cargo-fuzz would live, so this test means the same thing on a
        # machine that has one installed and on one that does not.
        environment = dict(os.environ)
        environment["PATH"] = f"{binaries}:/usr/bin:/bin"
        environment["STUB_DIR"] = str(stub_dir)

        result = subprocess.run(
            ["bash", str(FUZZ), "parse_message"],
            env=environment,
            capture_output=True,
            text=True,
            timeout=120,
        )
        calls = (stub_dir / "cargo-calls").read_text(encoding="utf-8")
        report = (
            f"exit={result.returncode}\n--- stdout ---\n{result.stdout}\n"
            f"--- stderr ---\n{result.stderr}\n--- cargo calls ---\n{calls}"
        )

        if result.returncode == 0:
            FAILURES.append(f"a missing cargo-fuzz must not look like success:\n{report}")

        output = result.stdout + result.stderr
        if "cargo install cargo-fuzz" not in output:
            FAILURES.append(
                "the script did not say how to install the missing tool, which "
                f"leaves the reader with rustup's error and no next step:\n{report}"
            )

        if calls.strip():
            FAILURES.append(
                "cargo was invoked before the check, so rustup still downloads a "
                f"nightly toolchain before failing:\n{report}"
            )

    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed:\n", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}\n", file=sys.stderr)
        return 1
    print("fuzz.sh missing-tool check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
