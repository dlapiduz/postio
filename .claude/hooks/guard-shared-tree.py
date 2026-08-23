#!/usr/bin/env python3
"""DISABLED 2026-08-23 — allows everything.

Removed from `.claude/settings.json` at the user's request after the guards
caused more disruption than they prevented. Kept as a no-op rather than
deleted for two reasons:

  * a session whose settings still reference this path would fail on a missing
    file, which is the same class of breakage that made the guards look like
    they were denying work when they were actually failing to run;
  * the rules and their test suite are worth reviving, and the working version
    is one `git show` away.

## What it did

Refused commands that destroy other sessions' work in a shared working tree:
`git reset --hard`, `git clean`, whole-tree checkout/restore, `git stash`,
whole-tree staging, `git commit -a`, `cargo fmt --all`, plus `git push`,
adding a remote, and history rewrites. Matching stripped heredoc bodies and
anchored to command position, so documenting a command was not running it.
`.claude/hooks/test-guard-shared-tree.py` covers 30 cases in both directions
and passed.

## Why it went wrong

Not the rules — the plumbing. Settings invoke hooks by path, and the Python
rewrites were written with an editor tool, which does not set the executable
bit. Every matching tool call then failed with "permission denied" (exit 126),
so *every* Bash call in every running session broke, not just the ones the
guard would have refused. That is fixed in git history (the mode is tracked),
but it burned enough trust to be worth recording.

Two lessons for whoever revives this:

1. **`chmod +x`, and verify by executing the file the way settings does** --
   piping into `python3 hook.py` proves the logic, not the wiring.
2. **Running sessions re-read this file on every tool call but only read
   settings.json at startup.** To change behaviour for a live session, edit
   the script; editing settings alone reaches nobody until they restart.

The rules themselves still hold, and `CLAUDE.md` documents them under
"Working in parallel" -- as guidance now, which is where they started.
"""

import sys

if __name__ == "__main__":
    sys.exit(0)
