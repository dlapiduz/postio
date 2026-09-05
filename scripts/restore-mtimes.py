#!/usr/bin/env python3
"""Set every tracked file's mtime to its own last-commit date. See #1126.

A fresh `actions/checkout` gives every file the mtime of the moment it was
written to disk, which is "now" for all of them alike. Cargo's freshness
check compares a compiled unit's fingerprint mtime against its source
files' mtimes, so a checkout looks like every source file changed at once
-- CI's cached `target/` is dropped rather than trusted, and the workspace
rebuilds from nothing on every run even when a PR touched one crate.

Restoring each file's mtime to the date of the commit that last touched it
fixes the comparison rather than working around it: a file untouched since
the cached build was made gets back the exact mtime cargo already fingerprinted
it against, so the build is skipped. A file a PR did change gets a newer
commit date than the one baked into the cache, so cargo sees it -- and
everything downstream of it -- as needing to be rebuilt. Getting this wrong
in the direction of "too old" is the dangerous case: a real change would
silently fail to rebuild, so read the doc on `commit_dates()` before
changing how a date is picked.

One walk of `git log`, not one `git log -1 -- <path>` per file: the latter
re-walks history from HEAD for every single file, which is fine on a
handful of paths and is the whole job on a workspace this size.

# The other direction is more dangerous, and depends on something outside
# this script

For a changed file's newer commit date to register as newer than the
cached build, `actions/cache` has to hand back `target/`'s artifacts with
the mtimes they had when they were archived -- not the moment of
extraction. Standard `tar` behaviour preserves the mtime stored in the
archive rather than stamping "now", and that is what the cache action
relies on too, but if that ever stopped being true the failure would run
the other way from a missed rebuild: *everything* would look fresher than
every commit date this script hands out, and cargo would stop rebuilding
things that actually changed, silently and permanently, until someone
noticed a fix that never shipped. Nothing here can detect that on its own
-- it is a property of the cache action, not of this script -- which is
why it is written down here rather than only in a commit message.

Usage:
    scripts/restore-mtimes.py

Run from within the repository (or a worktree of it) whose tracked files
should be touched. Nothing here reaches the network or writes outside the
working tree. Exit status: 0 once every trackable file has been touched
(files with no history behind them -- freshly added and not yet committed
-- are left at whatever mtime they already have, and counted separately
rather than treated as an error).
"""

from __future__ import annotations

import os
import subprocess
import sys
from datetime import datetime, timezone


def tracked_files() -> list[str]:
    """Every path `git` is tracking, repository-relative."""
    listed = subprocess.run(
        ["git", "ls-files", "-z"],
        capture_output=True,
        check=True,
    )
    return [path for path in listed.stdout.decode("utf-8", "surrogateescape").split("\0") if path]


def commit_dates() -> dict[str, str]:
    """`path -> ISO 8601 committer date of the newest commit that touched it`.

    One pass over `git log --name-only`, which lists commits newest-first and
    each commit's changed paths beneath it. The first time a path is seen is
    therefore its most recent touch, and every path is looked at exactly
    once regardless of how many commits it has behind it -- the difference
    between one history walk and one per file.

    Committer date, not author date: a rebase or a cherry-pick can carry an
    old author date forward onto a commit that is, from the tree's point of
    view, brand new, and a stale author date here would restore a mtime
    older than the one the previous build actually saw the file at -- the
    silent-miss failure mode this script exists to avoid. `%cI` is what
    `git log`'s default ordering and `git-restore-mtime` (the tool this
    approach is modelled on) both key off for the same reason.
    """
    walked = subprocess.run(
        ["git", "log", "--format=%x00%cI", "--name-only", "--no-renames"],
        capture_output=True,
        check=True,
        text=True,
    )
    dates: dict[str, str] = {}
    current_date: str | None = None
    for line in walked.stdout.splitlines():
        if line.startswith("\x00"):
            current_date = line[1:]
            continue
        if not line or current_date is None:
            continue
        dates.setdefault(line, current_date)
    return dates


def main() -> int:
    try:
        dates = commit_dates()
        paths = tracked_files()
    except subprocess.CalledProcessError as error:
        print(f"git failed: {error}", file=sys.stderr)
        return 1
    except FileNotFoundError:
        print("git is not on PATH", file=sys.stderr)
        return 1

    restored = 0
    untracked_in_history = 0
    for path in paths:
        date = dates.get(path)
        if date is None:
            # Added but not yet committed (working-tree-only in a test
            # fixture, or a file staged in the same run that built this
            # cache). Nothing to restore it to; leave the checkout mtime.
            untracked_in_history += 1
            continue
        timestamp = datetime.fromisoformat(date).astimezone(timezone.utc).timestamp()
        os.utime(path, (timestamp, timestamp))
        restored += 1

    print(f"restored mtimes for {restored} tracked file(s); {untracked_in_history} had no commit yet")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
