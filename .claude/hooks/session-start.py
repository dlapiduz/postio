#!/usr/bin/env python3
"""At the start of a session: fetch, say where the tree stands, and clear
the toolchain override.

Two things CLAUDE.md used to ask every session to remember, and sessions
did not:

1. **Fetch before you reason about the tree.** Other sessions land all the
   time, so a session that reads a stale checkout reasons about code that
   is no longer there. The shared checkout matters most: it is where
   Claude Code loads CLAUDE.md, the skills and these hooks from, so a stale
   one runs yesterday's rules. It is fast-forwarded here, but only when
   that cannot touch anyone's work -- on `main`, no tracked edits, and a
   fast-forward is possible. Everywhere else the session is told how far
   behind it is, and nothing moves.

2. **`RUSTUP_TOOLCHAIN` overrides rust-toolchain.toml, and mise exports
   it.** A shell that still has it builds with the wrong compiler, which
   shows up as a lint that fires only in CI. `unset` goes into
   `CLAUDE_ENV_FILE`, which Claude Code sources before every Bash command.

The output is a line or two on stdout, which Claude Code adds to the
session's context. It never fails the session: every error is reported and
the hook exits 0. `POSTIO_SESSION_FETCH=0` skips the network.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

FETCH_TIMEOUT = 20
UNSET = "unset RUSTUP_TOOLCHAIN"


def git(cwd: str, *args: str, timeout: float = 10) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", "-C", cwd, *args], capture_output=True, text=True, timeout=timeout,
        stdin=subprocess.DEVNULL,
    )


def clear_toolchain() -> None:
    env_file = os.environ.get("CLAUDE_ENV_FILE")
    if not env_file:
        return
    path = Path(env_file)
    try:
        if UNSET not in (path.read_text() if path.exists() else ""):
            with path.open("a") as out:
                out.write(UNSET + "\n")
    except OSError:
        pass


def commits(n: int) -> str:
    return f"{n} commit{'' if n == 1 else 's'}"


def report(cwd: str) -> list[str]:
    if git(cwd, "rev-parse", "--is-inside-work-tree").stdout.strip() != "true":
        return []
    lines = []
    if os.environ.get("POSTIO_SESSION_FETCH") == "0":
        lines.append("git: fetch skipped (POSTIO_SESSION_FETCH=0).")
    else:
        try:
            fetched = git(cwd, "fetch", "--quiet", "origin", timeout=FETCH_TIMEOUT)
            if fetched.returncode != 0:
                lines.append(f"git: fetch failed ({fetched.stderr.strip().splitlines()[-1:]}); "
                             "origin/main may be stale.")
        except subprocess.TimeoutExpired:
            lines.append(f"git: fetch failed (no answer in {FETCH_TIMEOUT}s); "
                         "origin/main may be stale.")

    if git(cwd, "rev-parse", "--verify", "-q", "origin/main").returncode != 0:
        return lines
    counts = git(cwd, "rev-list", "--left-right", "--count", "HEAD...origin/main").stdout.split()
    if len(counts) != 2:
        return lines
    ahead, behind = int(counts[0]), int(counts[1])
    branch = git(cwd, "symbolic-ref", "--short", "-q", "HEAD").stdout.strip() or "a detached HEAD"

    # The main worktree is the shared checkout: its git dir is the common one.
    git_dir = git(cwd, "rev-parse", "--absolute-git-dir").stdout.strip()
    common = git(cwd, "rev-parse", "--path-format=absolute", "--git-common-dir").stdout.strip()
    shared = bool(git_dir) and git_dir == common

    if behind == 0:
        lines.append(f"git: {branch} is up to date with origin/main"
                     + (f" ({commits(ahead)} ahead)." if ahead else "."))
        return lines

    clean = git(cwd, "status", "--porcelain", "--untracked-files=no").stdout.strip() == ""
    if shared and branch == "main" and ahead == 0 and clean:
        merged = git(cwd, "merge", "--ff-only", "--quiet", "origin/main")
        if merged.returncode == 0:
            lines.append(f"git: the shared checkout fast-forwarded {commits(behind)} to "
                         "origin/main; CLAUDE.md, skills and hooks are current.")
            return lines

    lines.append(
        f"git: {branch} is {commits(behind)} behind origin/main"
        + (f" and {commits(ahead)} ahead" if ahead else "")
        + ". Read the current tree from origin/main"
        + (" (git show origin/main:<path>), not this checkout." if shared
           else ", and rebase before reasoning about what landed.")
    )
    return lines


def main() -> int:
    try:
        payload = json.load(sys.stdin)
    except (json.JSONDecodeError, ValueError):
        payload = {}
    clear_toolchain()
    try:
        lines = report(payload.get("cwd") or os.getcwd())
    except (OSError, subprocess.SubprocessError) as error:
        lines = [f"git: could not check the tree ({error})."]
    if lines:
        print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main())
