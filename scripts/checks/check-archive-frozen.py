#!/usr/bin/env python3
"""Nothing edits a document under docs/archive/.

The archive holds what no longer describes the system (#1804). A change may
move a document into it, delete one, or update the archive's own README; it
may not edit an archived document. The way that happens is never on purpose:
a branch edits a file -- engineering-notes.md, a note's index line -- that
main has since moved into the archive, and on rebase git follows the rename
and lands the edit in the archived copy, where nothing reads it. #1781 and
#1803 both met it.

The comparison is with main: `origin/main` where it exists (a worktree, the
landing), or the pull request's base commit in CI, whose checkout has no
history; anywhere else the check says it cannot compare and passes.

    python3 scripts/checks/check-archive-frozen.py
    python3 scripts/checks/check-archive-frozen.py --root DIR   # a fixture

Exit status: 0 clean (or nothing to compare against), 1 an archived
document was edited.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
from pathlib import Path

ARCHIVE = "docs/archive/"
# The archive's own index: listing what is in it is what it is for.
EDITABLE = {"docs/archive/README.md"}


def git(root: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", *args], cwd=root, capture_output=True, text=True)


def base(root: Path) -> str | None:
    """The commit this tree is compared with, or None when there is none."""
    event = os.environ.get("GITHUB_EVENT_PATH")
    if os.environ.get("GITHUB_EVENT_NAME", "").startswith("pull_request") and event:
        sha = json.loads(Path(event).read_text())["pull_request"]["base"]["sha"]
        if git(root, "cat-file", "-e", sha).returncode != 0:
            git(root, "fetch", "--quiet", "--depth=1", "origin", sha)
        if git(root, "cat-file", "-e", sha).returncode == 0:
            # CI checks out the PR merged into its base, so base..tree is
            # exactly what the pull request changes.
            return sha
        return None
    if git(root, "rev-parse", "--verify", "--quiet", "origin/main").returncode != 0:
        return None
    merge_base = git(root, "merge-base", "origin/main", "HEAD")
    return merge_base.stdout.strip() or None


def edited(root: Path, since: str) -> list[str]:
    """Archived documents modified between `since` and the working tree."""
    out = git(root, "diff", "--name-status", "--find-renames", since).stdout
    found = []
    for line in out.splitlines():
        status, *paths = line.split("\t")
        # M is an edit; R with a similarity under 100 is a move *and* an edit
        # of a document that was already archived.
        path = paths[-1]
        if not path.startswith(ARCHIVE) or path in EDITABLE:
            continue
        if status == "M" or (status.startswith("R") and status != "R100" and paths[0].startswith(ARCHIVE)):
            found.append(path)
    return found


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=str(Path(__file__).resolve().parent.parent.parent))
    root = Path(parser.parse_args().root)
    since = base(root)
    if since is None:
        print("archive-frozen check skipped: no main to compare against here.")
        return 0
    found = edited(root, since)
    if found:
        for path in found:
            print(
                f"FAIL: {path} is archived and this change edits it. If the edit was "
                "meant for the live document it came from, it belongs there "
                "(docs/notes/README.md is the notes index now); restore the archived "
                f"copy with `git checkout {since[:12]} -- {path}`."
            )
        return 1
    print("archive-frozen check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
