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
2. **Command position.** A forbidden invocation only counts at the start of a
   command -- after nothing, or after `;`, `&&`, `||`, `|`, `(`, or a newline.
   A path or message that merely contains the words is not an invocation.

PreToolUse contract: the tool call arrives as JSON on stdin; a permission
decision goes out as JSON on stdout. Exit 0 either way -- "deny" is the
payload, not the exit code.
"""

from __future__ import annotations

import json
import re
import sys

SHARED = "Other Claude sessions are editing this tree right now."

# (pattern after the command-position anchor, reason). First match wins.
RULES: list[tuple[str, str]] = [
    (
        r"git\s+reset\s+(?:[^|;&]*\s)?--hard",
        f"{SHARED} Refusing 'git reset --hard': it irrecoverably deletes every "
        "session's uncommitted work, not just yours. Revert your own files by "
        "path, or commit what you have. See 'Working in parallel' in CLAUDE.md.",
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
        "unfinished files. Stage explicit paths, e.g. "
        "'git add crates/<your-crate> Cargo.lock'.",
    ),
    (
        r"git\s+commit\s+(?:[^|;&]*\s)?(?:-a\b|--all\b|-[a-zA-Z]*a[a-zA-Z]*\b)",
        f"{SHARED} Refusing 'git commit -a': it commits every modified file in "
        "the tree, including other sessions'. Stage your own paths first, then "
        "plain 'git commit'.",
    ),
    (
        r"cargo\s+fmt\s+(?:[^|;&]*\s)?(?:--all|--workspace)(?![^|;&]*--check)",
        f"{SHARED} Refusing 'cargo fmt --all': it reformats crates another "
        "session is mid-edit in, creating phantom diffs in their files. "
        "Use 'cargo fmt -p <your-crate>'.",
    ),
    (
        r"git\s+push",
        "Refusing 'git push': pushing is NOT standing-authorised in this "
        "repository -- only commits are. Nothing has been published yet and "
        "history was rewritten to scrub personal data. Ask the user first. "
        "See 'Git authority' in CLAUDE.md.",
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


def main() -> int:
    try:
        payload = json.load(sys.stdin)
    except (json.JSONDecodeError, ValueError):
        return 0

    command = (payload.get("tool_input") or {}).get("command") or ""
    if not command:
        return 0

    haystack = strip_heredocs(command)

    for pattern, reason in RULES:
        if re.search(ANCHOR + pattern, haystack):
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
    sys.exit(main())
