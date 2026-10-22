#!/usr/bin/env python3
"""Refuse a second integration binary in a crate unless it says why.

Cargo compiles every `tests/*.rs`, and every `tests/<dir>/main.rs`, into an
executable of its own, and each one links the crate's whole dependency
stack. #841 and #1128 consolidated the workspace's integration tests into
one suite per crate for exactly that reason, and nothing then stopped new
files landing beside the suites: by 2026-10-22, 57 had, costing 57 links per
build and 1.1 GiB of binaries in every worktree.

Some tests genuinely need a process of their own -- one that sets an
environment variable or a global subscriber or allocator, one that runs
itself again as a child by its exact test name, a measurement whose numbers
a neighbour would disturb. Those say so, and this check holds them to it.

# The rule

In each `crates/*/tests/`, at most one integration target may stand without
a stated reason: the suite, or a crate's only test file. Every other
top-level `tests/*.rs` carries a doc line of its own at the top level,

    //! POSTIO-OWN-BINARY: <why it cannot be a module of the suite>

or the measurement tier's `//! POSTIO-MEASUREMENT:` marker.

# Exit status

0 clean, 1 a crate has an unexplained extra binary, 2 the check could not run.
"""

from __future__ import annotations

import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

MARKER = re.compile(r"^//! POSTIO-(?:OWN-BINARY|MEASUREMENT):", re.M)
TARGET = re.compile(r"^crates/([^/]+)/tests/(?:([^/]+)\.rs|([^/]+)/main\.rs)$")


class CheckError(Exception):
    """The check could not be run, as opposed to: the check failed."""


def tracked_targets() -> dict[str, list[Path]]:
    """Every tracked integration target, by crate."""
    try:
        listed = subprocess.run(
            ["git", "ls-files", "crates/*/tests/*.rs", "crates/*/tests/*/main.rs"],
            capture_output=True,
            text=True,
            check=True,
        )
    except FileNotFoundError as error:
        raise CheckError("git is not on PATH") from error
    except subprocess.CalledProcessError as error:
        raise CheckError(f"git ls-files failed: {error.stderr.strip()}") from error
    targets: dict[str, list[Path]] = defaultdict(list)
    for line in listed.stdout.splitlines():
        match = TARGET.match(line)
        if match:
            targets[match.group(1)].append(Path(line))
    return targets


def explained(path: Path) -> bool:
    if path.name == "main.rs":
        return False
    try:
        return bool(MARKER.search(path.read_text(encoding="utf-8")))
    except OSError as error:
        raise CheckError(f"cannot read {path}: {error}") from error


def main() -> int:
    try:
        targets = tracked_targets()
        problems: list[tuple[str, list[Path]]] = []
        for crate, paths in sorted(targets.items()):
            bare = sorted(path for path in paths if not explained(path))
            if len(bare) > 1:
                problems.append((crate, bare))
    except CheckError as error:
        print(f"cannot run the check: {error}", file=sys.stderr)
        return 2

    if not problems:
        print(f"tests-share-a-suite check passed ({len(targets)} crates).")
        return 0

    print("tests-share-a-suite check FAILED\n", file=sys.stderr)
    for crate, bare in problems:
        suites = [path.parent.name for path in bare if path.name == "main.rs"]
        home = f"tests/{suites[0]}/" if suites else f"a new tests/<crate>_suite/"
        print(f"  {crate}: {len(bare)} integration binaries without a reason; "
              f"move the files into {home}", file=sys.stderr)
        for path in bare:
            if path.name != "main.rs":
                print(f"    {path}", file=sys.stderr)
    print(
        "\nEvery file in tests/ is a binary of its own and links the whole\n"
        "dependency stack. Move the file into the crate's suite directory and\n"
        "declare it with `mod <name>;` in its main.rs (a nested `mod helper;`\n"
        "becomes one declaration in main.rs, used as `crate::helper`, and an\n"
        "`include_str!` path gains a `../`).\n\n"
        "If it truly needs its own process, say why on a doc line at its top:\n"
        "    //! POSTIO-OWN-BINARY: <the process-global it sets, or the like>",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
