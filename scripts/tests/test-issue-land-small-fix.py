#!/usr/bin/env python3
"""A small fix can land without an issue (maintainer, 2026-09-10).

CLAUDE.md's ten-minute rule says to fix anything small on the spot rather than
file it. That left a change with no way to land: `issue-land.sh` refused any
branch not named `issue-<n>-<slug>`, so the only route was to file the issue
the rule exists to avoid.

`fix/`, `docs/` and `chore/` branches are that route, and `feature/` is the
same route for the opposite size of work — spec-driven work is not decomposed
into one issue per task (constitution 1.1.0), so its branch has no issue to
name. What this asserts is the
shape of the guard and of what depends on it -- that such a branch is
accepted, that a real issue branch still is, that something clearly wrong is
still refused, and that no `Closes` or `Refs:` is written when there is no
issue to name.

Reading the script rather than driving a landing: a landing is minutes and
network, and the question here is which branch names get past one line.

Usage: scripts/tests/test-issue-land-small-fix.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

SCRIPT = Path(__file__).resolve().parents[2] / "scripts" / "issue-land.sh"

# The guard, lifted so the branch names can be tried against the real
# expression rather than a copy of it that can drift.
LIFT = re.compile(
    r"SMALL=0\n(if \[ -z \"\$ISSUE\" \] && printf .*?\nfi)\n", re.S
)


def accepts(guard: str, branch: str) -> bool:
    probe = f'BRANCH="{branch}"\nISSUE=""\nSMALL=0\n{guard}\necho "$SMALL"\n'
    # `patience.run`, not `subprocess.run`: a hand-rolled deadline measures
    # the process it runs in, and on a shared workstation that is a flake
    # nobody can reproduce alone (#842, #957).
    out = patience.run(["bash", "-c", probe], capture_output=True, text=True, timeout=30)
    return out.stdout.strip() == "1"


def main() -> int:
    source = SCRIPT.read_text(encoding="utf-8")
    problems = 0

    lifted = LIFT.search(source)
    if not lifted:
        print(
            "FAIL: the small-fix guard is gone or reshaped; this test cannot "
            "find it, and would otherwise pass while testing nothing",
            file=sys.stderr,
        )
        return 1
    guard = lifted.group(1)

    for branch in ["fix/a-thing", "docs/the-rule", "chore/tidy-up", "fix/one.two_three"]:
        if not accepts(guard, branch):
            print(f"FAIL: {branch!r} should land as a small fix", file=sys.stderr)
            problems += 1

    # `feature/` is the same route for the opposite size of work: spec-driven
    # work is not decomposed into one issue per task (constitution 1.1.0), so
    # its branch has no issue to name and could not otherwise land at all.
    for branch in ["feature/compose-editor", "feature/mailbox-roles"]:
        if not accepts(guard, branch):
            print(
                f"FAIL: {branch!r} should land as a spec feature branch",
                file=sys.stderr,
            )
            problems += 1

    # The control. Without these the guard could be `SMALL=1` unconditionally
    # and every case above would pass.
    for branch in ["main", "wip", "fix/", "fix/a/b", "feature/", "feature/a/b",
                   "random-branch"]:
        if accepts(guard, branch):
            print(
                f"FAIL: {branch!r} was accepted without an issue; only "
                "fix/<slug>, docs/<slug>, chore/<slug> and feature/<slug> may "
                "skip it",
                file=sys.stderr,
            )
            problems += 1

    # And nothing may be closed or referred to when there is no issue.
    if 'CLOSES_LINE="No issue' not in source:
        print(
            "FAIL: a small fix's PR does not say it has no issue -- a reader "
            "is left looking for the one it forgot to name",
            file=sys.stderr,
        )
        problems += 1
    if 'git commit -m "$MSG"' not in source:
        print(
            "FAIL: the auto-written commit still adds `Refs:` for a small "
            "fix; a made-up issue number is worse than none",
            file=sys.stderr,
        )
        problems += 1

    if problems:
        print(f"\n{problems} problem(s).", file=sys.stderr)
        return 1
    print("issue-land small-fix check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
