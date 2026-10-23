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


def holder(tree: str) -> str:
    gitdir = git(tree, "rev-parse", "--absolute-git-dir").stdout.strip()
    try:
        return str(json.loads(Path(gitdir, CLAIM_FILE).read_text()).get("session") or "")
    except (OSError, ValueError, AttributeError):
        return ""


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
            if root not in Path(tree).resolve().parents or holder(tree) != session:
                continue
            status = git(tree, "status", "--porcelain").stdout.splitlines()
            if status:
                loose.append(f"{tree} ({len(status)} path{'s' if len(status) != 1 else ''})")
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
