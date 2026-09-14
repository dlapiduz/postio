#!/usr/bin/env python3
"""Refuse a dependency nothing in the crate uses -- `cargo machete`.

# What this guards

Every crate in ``Cargo.toml`` is compile time for every session on the box
and a line in the supply-chain policy's audit surface. The 2026-09-14 pass
found two declared and unused (`unicode-normalization` in postio-storage,
`base64` in postio-ui): each had been real once, lost its last caller in a
refactor, and stayed because nothing notices a dependency that is merely
present. `cargo machete` notices -- it reads the sources for `use` and path
references and reports a dependency with none.

# Why a missing cargo-machete is a skip rather than a failure

The same reason `check-dependency-policy.py` gives for cargo-deny: the tool
is a from-source build, ``mise.toml`` leaves the cargo-based tools out on
purpose, and a fresh clone must pass `check.sh` before it has built anything.
So this skips when the tool is absent and says so on stdout, every time;
``scripts/install-machete.sh`` installs the pinned version, and CI's
boundaries job runs it so the gate is real where it matters.

# What it cannot see

A dependency used only through a macro or a build script may read as unused;
``[package.metadata.cargo-machete] ignored = [...]`` in that crate's
``Cargo.toml`` is the place to say so, beside the dependency it excuses.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

TIMEOUT_SECONDS = 120


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    if shutil.which("cargo-machete") is None:
        print(
            "unused-deps check SKIPPED: cargo-machete is not installed.\n"
            "  scripts/install-machete.sh installs the pinned version; until then\n"
            "  a dependency nothing uses goes unnoticed on this machine."
        )
        return 0
    try:
        result = subprocess.run(
            ["cargo", "machete"],
            cwd=root,
            capture_output=True,
            text=True,
            timeout=TIMEOUT_SECONDS,
            check=False,
        )
    except subprocess.TimeoutExpired:
        print(f"unused-deps check FAILED: cargo machete did not finish in {TIMEOUT_SECONDS}s")
        return 1
    output = result.stdout + result.stderr
    if "found the following unused dependencies" in output or result.returncode not in (0,):
        print("unused-deps check FAILED: a crate declares a dependency nothing in it uses.\n")
        print(output.strip())
        print(
            "\nFix: delete the dependency from that crate's Cargo.toml, or -- when it is\n"
            "used only through a macro or a build script -- list it under\n"
            "[package.metadata.cargo-machete] ignored = [...] beside it, saying why."
        )
        return 1
    crates = sum(1 for _ in (root / "crates").glob("*/Cargo.toml"))
    print(f"unused-deps check passed ({crates} crates, cargo-machete).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
