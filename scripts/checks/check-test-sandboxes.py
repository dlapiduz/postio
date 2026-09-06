#!/usr/bin/env python3
"""A self-test's sandbox must live somewhere git ignores.

Several self-tests under `scripts/tests/` build a throwaway repository and run
the real `issue-land.sh` inside it. Those sandboxes have to sit **inside the
worktree**: the shared-tree guard only lifts its refusals for worktree paths,
so a sandbox in `/tmp` would be refused the git commands the test exists to
drive. That part is correct and is not what this checks.

What this checks is *where* inside. They were built in the repository **root**,
with `tempfile.TemporaryDirectory(dir=REPO_ROOT)`, which cleans up on a normal
exit and does not when the run is killed — and a self-test run being killed is
ordinary: the suite is slow enough to outlive a tool call's timeout, and CI and
`check.sh` both run every one of them in a batch.

What a leaked sandbox then is: an untracked directory in the root of a
worktree, which is exactly where `git add -A` is normal and allowed (the hook
only refuses it in the shared checkout). One went into a commit while #1225
was being written — 94 files, 1,324 insertions, almost all rustc incremental
cache, plus an embedded git repository that git warned about on the way past.
It was caught by reading `git show --stat`, which is not a control.

Under `target/`, the same leak is invisible to `git add`, and
`check-tmp-growth.py` already reports on `target/tmp` so it does not
accumulate unnoticed.

Fix, at the site this names:

    SANDBOXES = REPO_ROOT / "target" / "tmp"
    SANDBOXES.mkdir(parents=True, exist_ok=True)
    ...
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
TESTS = ROOT / "scripts" / "tests"

# `TemporaryDirectory(dir=...)` and `mkdtemp(dir=...)`, capturing what the
# directory is. Only the literal argument is read: a name is enough to say
# whether it resolves under `target/`, and nothing here needs to execute the
# test to find out.
SANDBOX = re.compile(r"(?:TemporaryDirectory|mkdtemp)\s*\(\s*(?:[^)]*?,\s*)?dir\s*=\s*([A-Za-z_][A-Za-z0-9_. /\"\']*)")

# The names a test may pass. `SANDBOXES` is the one this check exists to
# establish; the rest are absolute or already-temporary paths that never sit
# in the repository.
ALLOWED = {"SANDBOXES", "base", "directory", "tempfile.gettempdir()"}


def offenders() -> list[str]:
    found = []
    for path in sorted(TESTS.glob("*.py")):
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            match = SANDBOX.search(line)
            if not match:
                continue
            where = match.group(1).strip()
            if where in ALLOWED:
                continue
            found.append(
                f"  {path.relative_to(ROOT)}:{number}: sandbox at {where}\n"
                f"      {line.strip()}"
            )
    return found


def target_is_ignored() -> bool:
    """`target/` really is ignored, rather than assumed to be."""
    result = subprocess.run(
        ["git", "-C", str(ROOT), "check-ignore", "-q", "target/tmp/probe"],
        capture_output=True,
    )
    return result.returncode == 0


def main() -> int:
    if not target_is_ignored():
        print("test-sandboxes check FAILED\n", file=sys.stderr)
        print(
            "  `target/` is not gitignored in this checkout, so moving the\n"
            "  sandboxes there would not fix anything. Fix .gitignore first.",
            file=sys.stderr,
        )
        return 1

    found = offenders()
    if not found:
        print(f"test-sandboxes check passed ({len(list(TESTS.glob('*.py')))} self-tests).")
        return 0

    print("test-sandboxes check FAILED\n", file=sys.stderr)
    print("\n".join(found), file=sys.stderr)
    print(
        f"\n{len(found)} sandbox(es) outside a gitignored path.\n\n"
        "A self-test's sandbox has to be inside the worktree -- the shared-tree\n"
        "guard only lifts its refusals for worktree paths -- but not in its\n"
        "root, where a run that is killed leaves an untracked build cache that\n"
        "`git add -A` will commit. One did, in a worktree, while #1225 was\n"
        "being written: 94 files and an embedded git repository.\n\n"
        "Put it under `target/`, which git ignores and check-tmp-growth.py\n"
        "already watches:\n\n"
        "    SANDBOXES = REPO_ROOT / \"target\" / \"tmp\"\n"
        "    SANDBOXES.mkdir(parents=True, exist_ok=True)\n"
        "    ...\n"
        "    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
