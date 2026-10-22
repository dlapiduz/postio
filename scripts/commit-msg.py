#!/usr/bin/env python3
"""Refuse a commit message or PR title that CLAUDE.md's rules forbid.

Those rules were prose for most of the project's life, and prose drifts:
31 of the 300 commits on main before this existed were off them --
`style(...)`, `fix:` with no scope, summaries well past 50 characters.
One rule is not cosmetic at all. GitHub reads `close`, `fix` and
`resolve` followed by an issue reference as an instruction wherever it
appears, so a body that says "this does not close #12" closes #12 the
moment the squash lands (#1234). Nothing short of refusing the words
stops that.

Two callers:

    scripts/commit-msg.py <message-file>     # .githooks/commit-msg
    scripts/commit-msg.py --title "<title>"  # the PR title, in CI

PRs merge squashed, so the PR title *is* the subject that reaches main,
and every commit body on the branch goes into the squash's body. The hook
catches a message while it is cheap to fix; the title check catches what
the hook never saw (`--no-verify`, a title edited on GitHub).

Exit status: 0 the message is fine, 1 it is refused (reasons on stderr),
2 usage.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

TYPES = ("feat", "fix", "docs", "test", "refactor", "perf", "chore", "ci", "build", "revert")

# Areas that are not a crate and not a top-level directory, but that work
# lands in often enough to deserve a name. Everything else is derived from
# the tree, so a new crate or directory needs no edit here.
AREAS = {"workspace", "tooling", "ci", "deps", "release", "hooks", "skills", "checks", "land", "claim"}
# Directories at the root that no commit should be scoped to.
NOT_AREAS = {"target", "vendor", "Design", "patches"}

SUMMARY_MAX = 50
BODY_MAX = 72

SUBJECT = re.compile(r"^(?P<type>[a-z]+)\((?P<scope>[^)]+)\): (?P<summary>.+)$")
# Shapes git and the tooling write themselves. Their subjects are not ours
# to police: a revert quotes the subject it reverts, a fixup is squashed
# away, and a merge never reaches main through a squash anyway.
EXEMPT_SUBJECT = re.compile(r'^(?:Revert "|Merge |fixup! |squash! |amend! )')

# Not a grammar, a net for the usual slips: "added", "fixes".
NOT_IMPERATIVE = {
    "added", "adds", "fixed", "fixes", "removed", "removes", "updated", "updates",
    "changed", "changes", "moved", "moves", "renamed", "renames", "made", "makes",
    "implemented", "implements", "refactored", "refactors", "improved", "improves",
    "cleaned", "cleans", "bumped", "bumps", "deleted", "deletes", "replaced", "replaces",
}

# GitHub's closing keywords, with or without a colon, before a same-repo
# number, a cross-repo reference or an issue URL.
CLOSING = re.compile(
    r"\b(?P<word>close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s*:?\s+"
    r"(?P<ref>#\d+|[\w.-]+/[\w.-]+#\d+|https?://github\.com/\S+/issues/\d+)",
    re.IGNORECASE,
)

TRAILER = re.compile(r"^[A-Za-z][A-Za-z-]*: \S")
SCISSORS = re.compile(r"^# -+ >8 -+$")


def scopes() -> set[str]:
    names = set(AREAS)
    for manifest in (ROOT / "crates").glob("*/Cargo.toml"):
        names.add(manifest.parent.name.removeprefix("postio-"))
    for entry in ROOT.iterdir():
        if entry.is_dir() and not entry.name.startswith(".") and entry.name not in NOT_AREAS:
            names.add(entry.name)
    return names


def clean(text: str) -> list[str]:
    """The message as git will record it: no comments, nothing below the
    scissors, no trailing blank lines."""
    lines: list[str] = []
    for line in text.splitlines():
        if SCISSORS.match(line):
            break
        if line.startswith("#"):
            continue
        lines.append(line.rstrip())
    while lines and not lines[-1]:
        lines.pop()
    while lines and not lines[0]:
        lines.pop(0)
    return lines


def check_subject(subject: str) -> list[str]:
    if EXEMPT_SUBJECT.match(subject):
        return []
    match = SUBJECT.match(subject)
    if not match:
        return [f"the subject must be <type>(<scope>): <summary>, not {subject!r}"]
    problems = []
    kind, summary = match["type"], match["summary"]
    if kind not in TYPES:
        problems.append(f"type {kind!r} is not one of: {' '.join(TYPES)}")
    known = scopes()
    for scope in match["scope"].split(","):
        scope = scope.strip()
        if scope not in known:
            problems.append(
                f"scope {scope!r} is not a crate (without postio-), a top-level "
                f"directory, or one of: {' '.join(sorted(AREAS))}"
            )
    if len(summary) > SUMMARY_MAX:
        problems.append(f"the summary is {len(summary)} characters; at most {SUMMARY_MAX}")
    if summary.endswith("."):
        problems.append("the summary ends with a period")
    first = summary.split()[0].lower() if summary.split() else ""
    if first in NOT_IMPERATIVE:
        problems.append(f"the summary should be imperative: {first!r} -> 'add', 'fix', ...")
    return problems


def check_body(lines: list[str]) -> list[str]:
    if len(lines) < 2:
        return []
    if lines[1]:
        return ["the subject needs a blank line after it"]
    body = lines[2:]
    # The last paragraph, when it is all trailers (Refs, Co-Authored-By,
    # Claude-Session), may run long: a trailer cannot be wrapped.
    start = len(body)
    while start > 0 and body[start - 1]:
        start -= 1
    tail = body[start:]
    trailers = start if tail and all(TRAILER.match(line) for line in tail) else len(body)
    problems = []
    for number, line in enumerate(body[:trailers], start=3):
        if len(line) <= BODY_MAX:
            continue
        # Code, quoted output, a URL or one unbreakable token cannot wrap.
        if line[:1] in (" ", "\t") or "://" in line or " " not in line.strip():
            continue
        problems.append(f"line {number} is {len(line)} characters; wrap the body at {BODY_MAX}")
    return problems


def check_closing(lines: list[str]) -> list[str]:
    problems = []
    for line in lines:
        for match in CLOSING.finditer(line):
            problems.append(
                f"{match.group(0)!r} is a closing keyword: GitHub closes {match['ref']} when "
                "this lands, even negated. Write 'Refs: ...' or 'leaves ... open'; the "
                "PR's Closes line does the closing"
            )
    return problems


def check_refs(lines: list[str], branch: str) -> list[str]:
    if not lines or EXEMPT_SUBJECT.match(lines[0]):
        return []
    issue = re.match(r"^issue-(\d+)-", branch)
    if issue and not any(re.match(r"^Refs: .*#\d+", line) for line in lines):
        return [f"a commit on {branch} ends with 'Refs: #{issue[1]}' (or the issue it is for)"]
    if branch.startswith("feature/") and not any(line.startswith("Refs: specs/") for line in lines):
        return [f"a commit on {branch} ends with 'Refs: specs/<nnn>-<name>' and its task id"]
    return []


def current_branch() -> str:
    result = subprocess.run(
        ["git", "symbolic-ref", "--short", "-q", "HEAD"], capture_output=True, text=True
    )
    return result.stdout.strip()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("file", nargs="?", help="a commit message file")
    parser.add_argument("--title", help="check a PR title instead of a message")
    parser.add_argument("--branch", help="the branch (default: the checked-out one)")
    args = parser.parse_args()

    if args.title is not None:
        title = re.sub(r" \(#\d+\)$", "", args.title.strip())
        problems = check_subject(title) + check_closing([title])
        what = "PR title"
    elif args.file:
        lines = clean(Path(args.file).read_text())
        if not lines:
            return 0  # git aborts an empty message itself
        branch = args.branch if args.branch is not None else current_branch()
        problems = (
            check_subject(lines[0])
            + check_body(lines)
            + check_closing(lines)
            + check_refs(lines, branch)
        )
        what = "commit message"
    else:
        parser.print_usage(sys.stderr)
        return 2

    if not problems:
        return 0
    print(f"Refusing this {what} (CLAUDE.md, 'Commits and git'):", file=sys.stderr)
    for problem in problems:
        print(f"  - {problem}", file=sys.stderr)
    if what == "commit message":
        print(
            f"Nothing was committed. The message is kept in {args.file}; fix it and "
            f"commit again with -F {args.file}.",
            file=sys.stderr,
        )
    return 1


if __name__ == "__main__":
    sys.exit(main())
