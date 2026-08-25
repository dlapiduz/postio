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
5. **Quoted pathspecs are the scope.** One rule -- `unscoped_rustfmt` -- has to
   read the RAW command rather than the stripped haystack, because what makes
   a `rustfmt` safe is a pathspec that is almost always quoted. Blanking it
   would turn every correct invocation into a refusal. See that function.

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
        "you changed, scoped to your own crate: git status --porcelain -- "
        "crates/YOUR-CRATE | awk '{print $NF}' | xargs -r rustfmt --edition 2024. "
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

# Rules that apply in EVERY tree, private worktree included.
#
# #130. Every rule in `RULES` is about the shared working tree -- one index,
# one branch, one target directory -- so a private worktree is exempt from all
# of them, correctly. A push is not like that. It does not touch the tree at
# all; it touches the *remote*, which every session shares no matter whose
# checkout the command was typed in. The worktree exemption was hiding that,
# and the gap was not the `--force-with-lease` spelling the issue suspected:
# in the shared checkout both spellings were already refused, and inside a
# worktree bare `--force` was permitted just as freely.
#
# `--force-with-lease` is deliberately absent, and is the whole point of
# splitting these out. It refuses if the remote holds anything the pusher has
# not seen, which is the protection the blanket rule is reaching for -- and
# `scripts/issue-land.sh` rebases onto origin/main before pushing, so the
# second push of an already-pushed branch is necessarily non-fast-forward.
# Refusing both spellings would not make the landing flow safe, it would make
# it impossible. Refusing bare `--force` and permitting the leased form is
# what makes "rebase, then push again" work without also permitting the
# spelling that discards whatever landed while you were not looking.
#
# It stays refused in the shared checkout, through `RULES` below: `main` is
# the branch every session commits to, and "nothing landed since I last
# fetched" is a far weaker promise there than on a branch one session owns.
REMOTE_RULES: list[tuple[str, str]] = [
    (
        r"git\s+push\s+(?:[^|;&\n]*\s)?"
        r"(?:--force(?!-with-lease)\b|-f\b|--mirror\b|--delete\b)",
        "Refusing a force push. The remote is shared by every session, in a "
        "way a private worktree is not -- a bare --force discards whatever "
        "landed on the branch since you last fetched, and cannot tell that "
        "from your own rebase. If you rebased your own issue branch and the "
        "push was rejected as non-fast-forward, that is what "
        "--force-with-lease is for: it does the same thing and refuses if the "
        "remote has moved. Anything else -- --mirror, --delete, or a force "
        "onto a branch you do not own -- needs the user.",
    ),
]

# Start of string, or just after a shell command separator.
ANCHOR = r"(?:^|[;&|(]|&&|\|\||\n)\s*"

# A `rustfmt` that will WRITE: at command position, or fed by xargs, and not
# in one of the read-only forms. `--check` and `-l` only report.
RUSTFMT_WRITES = re.compile(
    r"(?:" + ANCHOR + r"|xargs\s+(?:-r\s+)?(?:-[a-zA-Z0-9]+\s+)*)"
    r"rustfmt\b(?![^|;&\n]*(?:--check\b|--emit\b|-l\b))"
)

# Something that lists files across the tree. These are the ways a file list
# gets derived rather than typed.
LISTS_FILES = re.compile(
    r"git\s+(?:diff\s+--name-only|ls-files|status\s+--porcelain)|\bfind\b"
)

# A pathspec naming an actual crate. No trailing slash required, because
# `-- crates/postio-core` is a perfectly good pathspec; but at least one name
# character is, because `crates/` alone is *every* crate, which is the bug
# wearing a pathspec rather than a fix for it.
NAMES_A_CRATE = re.compile(r"crates/[A-Za-z0-9_.-]+")


def unscoped_rustfmt(command: str, haystack: str) -> str | None:
    """Refuse a rustfmt whose file list came from an unscoped query.

    `postio-0uv0`. `rustfmt <files>` is safe -- it writes only what you name --
    so the hazard is never the tool, it is where the list came from. A list
    derived from `git diff --name-only HEAD` in a shared checkout is every
    session's dirty files, and formatting them writes into work the session
    running the command has never seen.

    This is not hypothetical and it is not a hazard anyone reasoned their way
    into: it is what `/land` told people to run, and a session ran it over 272
    lines of another session's loose work. `cargo fmt --all` has been refused
    here for exactly this reason since the guard existed, and it has strictly
    smaller blast radius than the command the skill recommended instead.

    Scope is read from the RAW command, never from `haystack`. The pathspec
    that makes one of these safe is almost always quoted --
    `-- 'crates/postio-core/*.rs'` -- and `strip_quoted` blanks quoted spans,
    so checking the haystack would find no crate name in a correctly scoped
    command and refuse every one of them. A guard that refuses the right
    answer trains people to turn it off.

    Known false negative, accepted deliberately: a crate name anywhere in the
    command counts as scope, so `cd crates/postio-core && rustfmt $(git diff
    --name-only HEAD)` is allowed even though `git diff` still reports the
    whole repository from a subdirectory. Tightening that means deciding which
    command segment a pathspec belongs to, which is a shell parser. This guard
    is defence in depth behind a documented skill, not a proof, and the cost of
    a false positive here is much higher than the cost of this miss.
    """
    if not RUSTFMT_WRITES.search(haystack):
        return None
    if not LISTS_FILES.search(haystack):
        # An explicit list of files. That is the safest form there is.
        return None
    if NAMES_A_CRATE.search(command):
        return None
    return (
        f"{SHARED} Refusing a rustfmt over an unscoped file list: "
        "'git diff --name-only HEAD' and friends list what is dirty in the "
        "WHOLE TREE, so this writes to other sessions' uncommitted files. "
        "That has already happened. Name the crates you are landing -- the "
        "same paths you will pass to git commit --only: "
        "git status --porcelain -- crates/YOUR-CRATE | awk '{print $NF}' "
        "| xargs -r rustfmt --edition 2024. Naming the files by hand is safer "
        "still, and --check is read-only and allowed."
    )


def contains(parent: str, child: str) -> bool:
    """Is `child` inside directory `parent`?

    Compared as paths, not as strings. A plain startswith() would say that
    /home/x/src/postio-worktrees/issue-27 lives inside /home/x/src/postio,
    which is how the worktree exemption silently failed to apply the first
    time it was written.
    """
    if not parent or not child:
        return False
    try:
        parent = os.path.realpath(parent)
        child = os.path.realpath(child)
    except OSError:
        return False
    return parent == child or child.startswith(parent.rstrip(os.sep) + os.sep)


# A `cd` at a command position: the start of the line, or after `;`, `&&`,
# `||`, `&` or a pipe. Its argument may be quoted, and the quotes are not part
# of the path.
CD = re.compile(
    r"""(?:^|[\n;&|]|&&|\|\|)\s*cd\s+("[^"]*"|'[^']*'|\S+)""",
)


def is_worktree(worktrees: str, path: str) -> bool:
    """Is `path` one of the private per-issue worktrees?

    A *strict* descendant: the worktrees directory itself holds no checkout,
    so it is not a place where the destructive commands are safe.
    """
    if not path or not contains(worktrees, path):
        return False
    try:
        return os.path.realpath(path) != os.path.realpath(worktrees)
    except OSError:
        return False


def cd_destination(command: str, cwd: str) -> str:
    """Where a leading `cd` sends the command, or `""` if none does.

    The shell expands `~` and `$HOME` before `cd` ever sees them, so a
    command that names a worktree that way is correct and the guard has to
    read it the same way. Not doing so is how issue #87 happened: the target
    did not start with `/`, the guard fell back to the session's own cwd —
    the shared checkout — and refused correct work in a tree that is
    explicitly exempt. That was the second time the worktree exemption
    silently failed on path handling, which is why this is a named function
    with tests rather than a condition inline.

    Heredoc bodies are stripped first: documenting a `cd` must not be
    performing one.

    The last `cd` wins, because that is the directory the rest of the line
    runs in.
    """
    targets = CD.findall(strip_heredocs(command))
    if not targets:
        return ""
    target = targets[-1].strip("\"'")
    target = os.path.expandvars(os.path.expanduser(target))
    if not target:
        return ""
    return target if os.path.isabs(target) else os.path.join(cwd or "", target)


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

    # Only guard the SHARED repository. Two kinds of place are exempt.
    #
    # A private per-issue worktree (scripts/issue-claim.sh) is the important
    # one: every rule below exists because sessions share one tree and one
    # index, and inside a worktree none of that is true. `git add -A` there
    # stages only that agent's own work. Blocking it would make the worktree
    # flow strictly worse than the shared tree it replaces.
    #
    # And scratch git work outside the project is nobody's hazard.
    project = os.environ.get("CLAUDE_PROJECT_DIR", "")
    worktrees = os.environ.get(
        "POSTIO_WORKTREES", os.path.expanduser("~/src/postio-worktrees")
    )

    cwd = payload.get("cwd") or ""

    # A `cd` into a private worktree is the one thing that can lift these
    # rules, and it can only ever *grant* the exemption -- never remove
    # protection. So a `cd` somewhere else leaves `where` as the session's own
    # directory rather than being trusted to have moved out of scrutiny, which
    # keeps `cd .. && git reset --hard` refused instead of arguing about
    # whether the parent of the shared checkout is the project.
    destination = cd_destination(command, cwd)
    where = destination if is_worktree(worktrees, destination) else cwd

    haystack = strip_quoted(strip_heredocs(command))

    matched: tuple[str, str] | None = None

    # Checked before the worktree exemption, never after it: these are about
    # the remote, which a private worktree shares with everybody else. See
    # REMOTE_RULES.
    for pattern, reason in REMOTE_RULES:
        if re.search(ANCHOR + pattern, haystack):
            matched = (pattern, reason)
            break

    if matched is None:
        if is_worktree(worktrees, where):
            return 0
        if project and where and not contains(project, where):
            return 0

        for pattern, reason in RULES:
            if re.search(ANCHOR + pattern, haystack):
                matched = (pattern, reason)
                break
        else:
            # Not a pattern rule: the one rule that needs two-part logic,
            # because whether a rustfmt is safe depends on where its file list
            # came from.
            why = unscoped_rustfmt(command, haystack)
            if why:
                matched = ("unscoped-rustfmt", why)

    if matched is None:
        return 0

    pattern, reason = matched
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
