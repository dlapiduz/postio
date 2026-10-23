#!/usr/bin/env python3
"""Refuse a commit that stages personal data or unformatted Rust.

Both are already gates: `check.sh` scans for personal data, and
`issue-land.sh` formats before it commits. But a session commits as it
goes, and every one of those commits is pushed with the branch. A real
address in a WIP commit is public the moment the branch is, whatever the
landing later fixes; and an unformatted file committed early is
whitespace in the next rebase. So both run here, on what is staged, in
the second or so a commit can afford.

- **Personal data** goes through `scripts/checks/check-no-personal-data.py`
  scoped to the staged paths. It reads the working-tree copy of each, which
  is a superset of the risk, and its output is redacted.
- **Formatting** compares each staged `.rs` blob with what rustfmt makes of
  it. The blob goes through stdin, so what is checked is exactly what is
  committed, and rustfmt does not wander into the file's child modules.
  `rustfmt --check` cannot be used: on stdin it exits 0 with a diff.
  A file rustfmt cannot parse is left to the compiler.

Run by `.githooks/pre-commit`, at the top of the worktree.
Exit status: 0 the commit may go ahead, 1 it is refused.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

EDITION = "2024"
NOT_OURS = ("vendor/", "target/")


def staged() -> list[str]:
    out = subprocess.run(
        ["git", "diff", "--cached", "--name-only", "--diff-filter=ACMR", "-z"],
        capture_output=True, text=True, check=True,
    ).stdout
    return [p for p in out.split("\0") if p]


def unformatted(paths: list[str]) -> list[str]:
    env = {k: v for k, v in os.environ.items() if k != "RUSTUP_TOOLCHAIN"}
    found = []
    for path in paths:
        if not path.endswith(".rs") or path.startswith(NOT_OURS):
            continue
        blob = subprocess.run(["git", "show", f":{path}"], capture_output=True, check=True).stdout
        # From the file's own directory, so rustfmt finds the tree's config
        # and rustup the tree's pinned toolchain.
        where = Path(path).parent
        while not where.is_dir():
            where = where.parent
        try:
            result = subprocess.run(
                ["rustfmt", "--edition", EDITION, "--emit", "stdout"],
                input=blob, capture_output=True, cwd=where, env=env, timeout=30,
            )
        except (OSError, subprocess.TimeoutExpired):
            return []  # no rustfmt here: issue-land.sh formats regardless
        if result.returncode == 0 and result.stdout != blob:
            found.append(path)
    return found


def personal_data(paths: list[str]) -> str:
    check = Path("scripts/checks/check-no-personal-data.py")
    if not paths or not check.exists():
        return ""
    result = subprocess.run(
        [sys.executable, str(check), *paths], capture_output=True, text=True
    )
    return "" if result.returncode == 0 else result.stdout + result.stderr


def main() -> int:
    paths = staged()
    leaks = personal_data(paths)
    messy = unformatted(paths)
    if not leaks and not messy:
        return 0
    print("Refusing this commit (.githooks/pre-commit):", file=sys.stderr)
    if messy:
        print("\nNot formatted:", file=sys.stderr)
        for path in messy:
            print(f"  {path}", file=sys.stderr)
        print(f"Fix: rustfmt --edition {EDITION} {' '.join(messy)} && git add "
              f"{' '.join(messy)}", file=sys.stderr)
    if leaks:
        print("\nStaged files carry personal data:\n", file=sys.stderr)
        print(leaks.rstrip(), file=sys.stderr)
    print("\nNothing was committed.", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
