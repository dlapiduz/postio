#!/usr/bin/env python3
"""Hold a session that stops with uncommitted work in its own worktree.

CLAUDE.md: "Commit as you go -- a WIP commit beats loose files -- and land
or commit before you stop." Loose files in a worktree are invisible to
every other session, to CI and to the maintainer, and `worktree-reap.sh`
or the next claim can take the tree with them. A session that ends with
some is the one that forgot.

Which worktrees are this session's: the ones whose claim stamp names it
(`postio-claim`, which the shared-tree guard writes when a session runs a
command in a worktree). A session's shell usually sits in the shared
checkout, so its cwd says nothing; another session's loose work is never
this one's to commit.

Nor is what an earlier holder left in a tree this session adopted: running
a command in a worktree whose claim has lapsed takes it, loose files and
all. The stamp says `since` when this session took it, and only paths that
changed at or after that count.

It holds the stop once. Claude Code sets `stop_hook_active` on the stop
that follows, and that one is let through, so a session stopping on
purpose -- to ask the user something -- says so and stops.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

CLAIM_FILE = "postio-claim"


def git(cwd: str, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", "-C", cwd, *args], capture_output=True, text=True, timeout=10,
                          stdin=subprocess.DEVNULL)


def worktrees(cwd: str) -> list[str]:
    listing = git(cwd, "worktree", "list", "--porcelain")
    if listing.returncode != 0:
        return []
    return [line[len("worktree "):] for line in listing.stdout.splitlines()
            if line.startswith("worktree ")]


def holder(tree: str) -> tuple[str, float]:
    """Who holds `tree`, and since when. A stamp from before the guard wrote
    `since` falls back to `at`, its last refresh."""
    gitdir = git(tree, "rev-parse", "--absolute-git-dir").stdout.strip()
    try:
        record = json.loads(Path(gitdir, CLAIM_FILE).read_text())
        return (str(record.get("session") or ""),
                float(record.get("since") or record.get("at") or 0.0))
    except (OSError, ValueError, TypeError, AttributeError):
        return "", 0.0


def loose_since(tree: str, since: float) -> int:
    """How many loose paths in `tree` changed at or after `since`: a path by
    its own mtime, a deleted one by its directory's, which the deletion
    touched."""
    fields = git(tree, "status", "--porcelain", "-z", "--untracked-files=all").stdout.split("\0")
    count, i = 0, 0
    while i < len(fields):
        entry = fields[i]
        i += 1
        if len(entry) < 4:
            continue
        if entry[0] in "RC":
            i += 1  # a rename or copy carries its old path next
        path = Path(tree, entry[3:])
        try:
            mtime = path.lstat().st_mtime
        except OSError:
            try:
                mtime = path.parent.lstat().st_mtime
            except OSError:
                mtime = since
        if mtime >= since:
            count += 1
    return count


def main() -> int:
    try:
        payload = json.load(sys.stdin)
    except ValueError:
        return 0
    session = payload.get("session_id") or ""
    if payload.get("stop_hook_active") or not session:
        return 0
    root = Path(os.environ.get("POSTIO_WORKTREES") or Path.home() / "src/postio-worktrees")
    try:
        root = root.resolve()
        loose = []
        for tree in worktrees(payload.get("cwd") or os.getcwd()):
            if root not in Path(tree).resolve().parents:
                continue
            held_by, since = holder(tree)
            if held_by != session:
                continue
            count = loose_since(tree, since)
            if count:
                loose.append(f"{tree} ({count} path{'s' if count != 1 else ''})")
    except (OSError, subprocess.SubprocessError):
        return 0
    if not loose:
        return 0
    print(json.dumps({"decision": "block", "reason": (
        "Uncommitted work in your worktree: " + "; ".join(loose) + ". Commit it before "
        "you stop -- a WIP commit beats loose files (git add <new files>, then git commit "
        "--only <your paths>) -- or land it with scripts/issue-land.sh --detach. If you "
        "are stopping on purpose, to ask the user something, say so and stop again."
    )}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
