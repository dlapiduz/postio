#!/usr/bin/env python3
"""Refuse commands that destroy or corrupt other sessions' work.

Several Claude sessions edit this repository concurrently, sharing one working
tree, one branch, one git index and one cargo target directory. CLAUDE.md
documents which commands are unsafe there; this hook enforces it, because
documentation is advisory and `git reset --hard` is irreversible.

Matching is deliberately careful about two false positives that would make the
hook worse than useless:

1. **Heredoc bodies.** Writing a file that *documents* a forbidden command must
   not be blocked. Heredoc bodies are stripped before matching.
2. **Quoted arguments.** A commit message that mentions `--` or `git stash`
   is data the command carries, not part of its invocation. Quoted spans
   are blanked before matching.
3. **One line at a time.** The flag-scanning classes exclude newlines. A
   negated class like `[^|;&]` matches a newline happily, so a scan for
   flags after `git commit` once ran off the end of the command and matched
   `--all` in an unrelated `cargo fmt` two lines below.
4. **Command position.** A forbidden invocation only counts at the start of a
   command -- after nothing, or after `;`, `&&`, `||`, `|`, `(`, or a newline.
   A path or message that merely contains the words is not an invocation.

PreToolUse contract: the tool call arrives as JSON on stdin; a permission
decision goes out as JSON on stdout. Exit 0 either way -- "deny" is the
payload, not the exit code.

## If you change this, verify it the way settings invokes it

An earlier version was correct in every unit test and still broke every
session, because it was invoked by path without an executable bit and failed
with "permission denied" (exit 126) on every matching call. Piping into
`python3 hook.py` proves the logic; it says nothing about the wiring. Settings
now invokes it through `python3` so the mode cannot matter, and the entry point
fails open so a bug costs a missed denial rather than a blocked session.

`POSTIO_GUARD=off` disables it without editing settings -- which a running
session would not re-read anyway. Note that a live session re-reads THIS FILE
on every call but reads settings.json only at startup, so to change behaviour
for a session already running, edit the script.
"""

from __future__ import annotations

import json
import os
import pathlib
import re
import sys
from datetime import datetime, timezone

SHARED = "Other Claude sessions are editing this tree right now."

# (pattern after the command-position anchor, reason). First match wins.
RULES: list[tuple[str, str]] = [
    (
        r"git\s+reset\s+(?:[^|;&\n]*\s)?--hard",
        f"{SHARED} Refusing 'git reset --hard': it irrecoverably deletes every "
        "session's uncommitted work, not just yours. Revert your own files by "
        "path, or commit what you have. See 'Working in parallel' in CLAUDE.md.",
    ),
    (
        # --hard is caught above with its own message. This is every other
        # form: bare `git reset`, `git reset HEAD~1`, `--soft`, `--mixed`.
        # A pathspec reset (`git reset -- <path>`) is the one safe use and is
        # allowed by requiring the `--` separator.
        r"git\s+reset(?![^|;&\n]*\s--\s)(?![^|;&\n]*--hard)",
        f"{SHARED} Refusing 'git reset': it moves the branch and DROPS commits, "
        "including ones another session landed -- and unlike --hard it leaves "
        "the files in place, so nothing looks wrong. That has already happened "
        "here: a landed commit vanished from history and its content resurfaced "
        "inside an unrelated commit. To unstage a path use "
        "'git restore --staged <path>'. To undo your own last commit, ask the "
        "user -- the branch is shared.",
    ),
    (
        r"git\s+clean\s+-",
        f"{SHARED} Refusing 'git clean': it deletes untracked files across all "
        "crates, including work another session has not committed yet.",
    ),
    (
        r"git\s+(?:checkout|restore)\s+(?:--\s+)?\.(?:\s|$)",
        f"{SHARED} Refusing a whole-tree checkout/restore: it discards other "
        "sessions' edits. Name your own paths explicitly.",
    ),
    (
        r"git\s+stash(?!\s+(?:list|show))",
        f"{SHARED} Refusing 'git stash': it stashes every session's changes, "
        "not just yours. If you need a green tree, keep working until your "
        "crate builds -- do not stash. See CLAUDE.md.",
    ),
    (
        r"git\s+add\s+(?:-A|--all|-u|\.(?:\s|$))",
        f"{SHARED} Refusing to stage everything: it commits other sessions' "
        "unfinished files. The index is SHARED, so even staging your own "
        "paths races anyone staging between your add and your commit -- "
        "that has happened three times here. Use "
        "'git commit --only <your paths> -m \"...\"' instead: it commits "
        "exactly those paths and leaves everyone else's staged work alone.",
    ),
    (
        r"git\s+commit\s+(?:[^|;&\n]*\s)?(?:-a\b|--all\b|-[a-zA-Z]*a[a-zA-Z]*\b)",
        f"{SHARED} Refusing 'git commit -a': it commits every modified file in "
        "the tree, including other sessions'. Stage your own paths first, then "
        "plain 'git commit'.",
    ),
    (
        r"cargo\s+fmt\s+(?:[^|;&\n]*\s)?(?:--all|--workspace|-p\s)(?![^|;&\n]*--check)",
        f"{SHARED} Refusing a crate-wide format: both --all and -p WRITE to every "
        "file in scope, including one another session has open. That has "
        "already put whitespace churn into someone else's diff. Format what "
        "you changed: rustfmt --edition 2024 $(git diff --name-only HEAD -- '*.rs'). "
        "The --check forms are read-only and allowed.",
    ),
    (
        r"git\s+push\s+(?:[^|;&\n]*\s)?(?:--force\b|-f\b|--mirror\b|--delete\b)",
        "Refusing a force push. History here has already been rewritten once to "
        "scrub personal data, and several sessions commit to this branch -- a "
        "force push discards whatever landed since you last fetched. Pull and "
        "merge, or ask the user.",
    ),
    (
        r"git\s+remote\s+(?:add|set-url)",
        "Refusing to add a remote: the user has not chosen to publish this "
        "repository yet, and history was rewritten to remove personal data. "
        "Ask first.",
    ),
    (
        r"git\s+(?:filter-repo|filter-branch|rebase)",
        "Refusing a history rewrite: other sessions hold refs that would become "
        "invalid, and the user must confirm the tree is quiet first. Ask before "
        "rewriting history.",
    ),
]

# Start of string, or just after a shell command separator.
ANCHOR = r"(?:^|[;&|(]|&&|\|\||\n)\s*"


def strip_heredocs(command: str) -> str:
    """Remove heredoc bodies so documenting a command is not running it."""
    out: list[str] = []
    lines = command.split("\n")
    i = 0
    while i < len(lines):
        line = lines[i]
        out.append(line)
        m = re.search(r"<<-?\s*(['\"]?)([A-Za-z_][A-Za-z0-9_]*)\1", line)
        i += 1
        if not m:
            continue
        marker = m.group(2)
        # Skip the body up to and including the terminator.
        while i < len(lines) and lines[i].strip() != marker:
            i += 1
        if i < len(lines):
            i += 1
    return "\n".join(out)


def strip_quoted(command: str) -> str:
    """Blank quoted spans so an argument cannot be read as a flag.

    `git commit -m 'feat: handle -- in the parser'` is not a pathspec commit,
    and `git commit -m "fix git stash"` is not a stash. Anything inside quotes
    is data the command carries, not part of its invocation.
    """
    out, quote = [], None
    for ch in command:
        if quote:
            out.append(" " if ch != quote else ch)
            if ch == quote:
                quote = None
        elif ch in "'\"":
            quote = ch
            out.append(ch)
        else:
            out.append(ch)
    return "".join(out)


LOG = pathlib.Path(__file__).with_name("guard.log")


def log(event: str, **fields: object) -> None:
    """Append one JSONL record. Must never raise, and never blocks.

    Two things are worth having on disk. Denials, so a rule that is firing more
    than it should is visible as data rather than as a session complaining --
    the last time this guard caused trouble I estimated its blast radius from
    the mechanism and was badly wrong, when a log would have said so exactly.
    And errors, because the entry point fails open: without a record, a broken
    guard silently allows everything and looks identical to a quiet one.
    """
    try:
        record = {
            "at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            "event": event,
            **fields,
        }
        with LOG.open("a", encoding="utf-8") as fh:
            fh.write(json.dumps(record) + "\n")
    except Exception:  # noqa: BLE001 - logging must not break the guard
        pass


def main() -> int:
    try:
        payload = json.load(sys.stdin)
    except (json.JSONDecodeError, ValueError):
        return 0
    except Exception:  # noqa: BLE001 - see fail-open note in the module docs
        return 0

    command = (payload.get("tool_input") or {}).get("command") or ""
    if not command:
        return 0

    # Kill switch: export POSTIO_GUARD=off to disable without editing settings,
    # which already-running sessions would not re-read.
    if os.environ.get("POSTIO_GUARD", "").lower() in {"off", "0", "false"}:
        return 0

    # Only guard the shared repository. A session doing scratch git work in
    # /tmp is not a hazard to anyone, and blocking it is pure friction.
    project = os.environ.get("CLAUDE_PROJECT_DIR", "")
    first_cd = re.match(r"\s*cd\s+(\S+)", command)
    if first_cd and project:
        target = first_cd.group(1).strip("\"'")
        if not target.startswith(project) and target.startswith("/"):
            return 0

    haystack = strip_quoted(strip_heredocs(command))

    for pattern, reason in RULES:
        if re.search(ANCHOR + pattern, haystack):
            log(
                "deny",
                rule=pattern,
                command=command[:200],
                session=payload.get("session_id", ""),
            )
            json.dump(
                {
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "deny",
                        "permissionDecisionReason": reason,
                    }
                },
                sys.stdout,
            )
            return 0

    return 0


if __name__ == "__main__":
    # Fail OPEN, always. A guard that errors must allow the command through.
    # The first version of this hook was invoked by path without an executable
    # bit, so it failed with "permission denied" (exit 126) on EVERY matching
    # tool call -- not just the ones it would have refused. Settings now runs
    # it through `python3`, which removes that failure mode entirely, and this
    # catch-all removes the rest: a bug in here costs a missed denial, never a
    # blocked session.
    try:
        sys.exit(main())
    except Exception as exc:  # noqa: BLE001
        log("error", error=f"{type(exc).__name__}: {exc}")
        sys.exit(0)
