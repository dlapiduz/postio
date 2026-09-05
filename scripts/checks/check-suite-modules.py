#!/usr/bin/env python3
"""Refuse a test file that sits in a suite directory and is never declared.

Consolidating 197 integration binaries into a handful of suites (#841) trades
one risk for another. The old shape could not lose a test: cargo compiles
every `tests/*.rs` into its own binary and runs it, so a file that existed
ran. Inside a suite directory, a file is only compiled if `main.rs` says
`mod <name>;` — and a file nobody declared is not an error, it is silence.
It compiles, the suite passes, and the cases in it have simply stopped
running.

That is the same failure this repository has now paid for three times in
different costumes: sixty GTK tests that skipped for want of a display and
reported success (#114), three files running half their cases because two
display-needing tests raced and the loser returned through its own guard
(#355), and a reader test that could not fail because nothing told the reader
anything (#596). A green tick that means "nothing ran" is worse than a red
one.

# The rule

Every `.rs` file in a directory containing a `main.rs` under `crates/*/tests/`
must be named by a `mod` declaration in that `main.rs`.

`harness = false` suites need more than that — a module can be declared and
still have no row in `CASES` — but that half is checked by the suite's own
`--list` count, and this check deliberately does not try to parse a Rust
array. One rule, enforced exactly.

# Exit status

0 clean, 1 a file is undeclared, 2 the check could not run.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

MOD = re.compile(r"^\s*(?:pub\s+)?mod\s+([a-z_0-9]+)\s*;", re.M)


class CheckError(Exception):
    """The check could not be run, as opposed to: the check failed."""


def tracked_suite_dirs() -> list[Path]:
    """Every directory under `crates/*/tests/` that has a `main.rs`."""
    try:
        listed = subprocess.run(
            ["git", "ls-files", "crates/*/tests/*/main.rs"],
            capture_output=True,
            text=True,
            check=True,
        )
    except FileNotFoundError as error:
        raise CheckError("git is not on PATH") from error
    except subprocess.CalledProcessError as error:
        raise CheckError(f"git ls-files failed: {error.stderr.strip()}") from error
    return [Path(line).parent for line in listed.stdout.splitlines() if line]


def undeclared(directory: Path) -> list[Path]:
    """Files in `directory` that its `main.rs` never declares."""
    main = directory / "main.rs"
    try:
        declared = set(MOD.findall(main.read_text(encoding="utf-8")))
    except OSError as error:
        raise CheckError(f"cannot read {main}: {error}") from error

    missing = []
    for path in sorted(directory.glob("*.rs")):
        if path.name == "main.rs":
            continue
        if path.stem not in declared:
            missing.append(path)
    return missing


def main() -> int:
    try:
        directories = tracked_suite_dirs()
        problems: list[Path] = []
        for directory in directories:
            problems.extend(undeclared(directory))
    except CheckError as error:
        print(f"cannot run the check: {error}", file=sys.stderr)
        return 2

    if not problems:
        print(f"suite-modules check passed ({len(directories)} suites).")
        return 0

    print("suite-modules check FAILED\n", file=sys.stderr)
    for path in problems:
        print(f"  {path}: never declared in {path.parent / 'main.rs'}", file=sys.stderr)
    print(
        f"\n{len(problems)} file(s).\n\n"
        "A file in a suite directory is compiled only if `main.rs` declares\n"
        "it. An undeclared one is not an error -- it is silence: the suite\n"
        "still passes, and every test in that file has stopped running.\n\n"
        "Add `mod <name>;` to the suite's main.rs. If the file is a helper\n"
        "with no tests it still needs declaring; if it is dead, delete it.\n"
        "For a `harness = false` suite, remember the CASES row too -- a\n"
        "declared module with no row compiles and still never runs.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
