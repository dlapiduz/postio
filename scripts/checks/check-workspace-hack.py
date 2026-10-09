#!/usr/bin/env python3
"""Refuse a workspace-hack that has fallen behind the manifests.

`postio-workspace-hack` (ADR 0047) names every third-party crate the
workspace builds with more than one feature set, with the union of those
features, and every member depends on it. That is what lets `cargo test -p
<crate>` reuse what the workspace build already compiled instead of
rebuilding a hundred crates with different features -- measured, a
`postio-storage` unit-test build went from 98s to 1s in a warm tree.

Nothing breaks when it falls behind. A dependency added without
regenerating it, or a new member without the line naming it, compiles and
passes every test; the rebuilds simply come back, one crate at a time, and
nobody connects them to the change that caused them. So this asks
`cargo hakari` both questions:

  * `generate --diff`: would the hack's contents change?
  * `manage-deps --dry-run`: is a member missing its dependency on it?

# Why a missing cargo-hakari is a skip rather than a failure

The trade `check-unused-deps.py` makes for cargo-machete: a fresh clone must
pass `check.sh` before it has built any tool. Locally this skips and says so;
under CI it fails (`scripts/lib/prereq.py`), and `ci.yml`'s boundaries job
installs the pinned version with `scripts/install-hakari.sh`.

Usage: scripts/checks/check-workspace-hack.py [--root <workspace>]
Exit status: 0 in step (or skipped locally), 1 behind, 2 could not run.
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import prereq  # noqa: E402  -- enabled by the sys.path line above

TIMEOUT_SECONDS = 300
FIX = "cargo hakari generate && cargo hakari manage-deps --yes"


def hakari(root: Path, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["cargo", "hakari", "--color", "never", *args],
        cwd=root,
        capture_output=True,
        text=True,
        timeout=TIMEOUT_SECONDS,
        check=False,
    )


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    root = parser.parse_args(argv).root

    verdict = prereq.verdict(
        is_ci=prereq.is_ci(), present=shutil.which("cargo-hakari") is not None
    )
    if verdict == prereq.SKIP_AND_SAY:
        print(
            "workspace-hack check SKIPPED: cargo-hakari is not installed.\n"
            "  scripts/install-hakari.sh installs the pinned version; until then\n"
            "  a manifest change that leaves the hack behind goes unnoticed here."
        )
        return 0
    if verdict == prereq.FAIL:
        print("workspace-hack check FAILED: cargo-hakari is missing on a CI runner.\n"
              "  scripts/install-hakari.sh installs it.")
        return 2

    found = []
    try:
        generate = hakari(root, "generate", "--diff")
        manage = hakari(root, "manage-deps", "--dry-run")
    except subprocess.TimeoutExpired:
        print(f"workspace-hack check FAILED: cargo hakari did not finish in {TIMEOUT_SECONDS}s")
        return 2
    # Both exit 1 when they would change something, and 2 or more when they
    # could not run at all -- which is not the same finding.
    for proc, what in (
        (generate, "the hack's contents are out of date"),
        (manage, "a member's dependency on the hack is missing or stale"),
    ):
        if proc.returncode == 1:
            found.append((what, (proc.stdout + proc.stderr).strip()))
        elif proc.returncode != 0:
            print(f"workspace-hack check could not run `cargo hakari`:\n{proc.stdout}{proc.stderr}")
            return 2

    if found:
        print("workspace-hack check FAILED (ADR 0047):\n")
        for what, detail in found:
            print(f"  - {what}:\n{detail}\n")
        print(f"Fix: {FIX}, and commit what it changes.")
        return 1
    print("workspace-hack check passed (contents and every member's line in step).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
