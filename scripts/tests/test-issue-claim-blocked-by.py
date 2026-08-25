#!/usr/bin/env python3
"""Self-test for issue #247: issue-claim.sh's blockedBy check never fires.

`gh issue list --json blockedBy` does not emit a `closed` field -- each node
is `{"id": ..., "number": ..., "state": "OPEN"|"CLOSED", "title": ..., "url":
...}`, per the real payload quoted in #247. The candidate filter used to read
`b.get("closed", False)`, which is always `False`, so `not False` was always
`True` and *every* issue with a non-empty `blockedBy` was reported as blocked
regardless of whether its blocker was open or closed.

This stubs `gh issue list` with a fixture shaped exactly like that real
payload -- one issue whose only blocker is CLOSED (must be claimable) and a
higher-priority issue whose only blocker is still OPEN (must stay blocked) --
and runs the real `issue-claim.sh --dry-run` against it. Before the fix both
issues are wrongly hidden and the script reports nothing ready at all; after
the fix the CLOSED-blocker issue is offered despite being lower priority, and
the OPEN-blocker issue is named in a "note: ... skipped: blocked" line.

Usage: scripts/tests/test-issue-claim-blocked-by.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
ISSUE_CLAIM = HERE / "issue-claim.sh"

# Shaped exactly like the real `gh issue list --json ...,blockedBy,...`
# payload quoted in #247 -- a hand-written dict missing a field is exactly
# how the original bug got in.
FIXTURE_ISSUES = [
    {
        "number": 12,
        "title": "closed blocker, should be claimable",
        "labels": [{"name": "ready"}, {"name": "p2"}],
        "assignees": [],
        "milestone": None,
        "blockedBy": {
            "nodes": [
                {
                    "id": "I_kwDOclosed",
                    "number": 70,
                    "state": "CLOSED",
                    "title": "already done",
                    "url": "https://github.com/example/example/issues/70",
                }
            ]
        },
    },
    {
        "number": 13,
        "title": "open blocker, must stay blocked",
        "labels": [{"name": "ready"}, {"name": "p0"}],
        "assignees": [],
        "milestone": None,
        "blockedBy": {
            "nodes": [
                {
                    "id": "I_kwDOopen",
                    "number": 99,
                    "state": "OPEN",
                    "title": "still open",
                    "url": "https://github.com/example/example/issues/99",
                }
            ]
        },
    },
]

GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
    cat "$STUB_DIR/issues.json"
    exit 0
fi
exit 1
"""

FAILURES: list[str] = []


def run_claim(repo: Path, stub_dir: Path) -> subprocess.CompletedProcess[str]:
    import os

    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_WORKTREES"] = str(repo / "worktrees")
    environment["POSTIO_CLAIMS"] = str(repo / "claims")
    return subprocess.run(
        ["bash", str(repo / "scripts" / "issue-claim.sh"), "--dry-run"],
        cwd=repo,
        env=environment,
        capture_output=True,
        text=True,
        timeout=30,
    )


def git(*args: str, cwd: Path) -> None:
    subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
    )


def main() -> int:
    with tempfile.TemporaryDirectory() as directory:
        repo = Path(directory) / "repo"
        stub_dir = Path(directory) / "stub"
        (stub_dir / "bin").mkdir(parents=True)
        (repo / "scripts").mkdir(parents=True)

        shutil.copy(ISSUE_CLAIM, repo / "scripts" / "issue-claim.sh")
        (repo / "scripts" / "issue-claim.sh").chmod(0o755)

        gh = stub_dir / "bin" / "gh"
        gh.write_text(GH_STUB, encoding="utf-8")
        gh.chmod(0o755)
        (stub_dir / "issues.json").write_text(
            json.dumps(FIXTURE_ISSUES), encoding="utf-8"
        )

        git("init", "-q", "-b", "main", cwd=repo)
        (repo / "README.md").write_text("fixture repo\n", encoding="utf-8")
        git("add", "-A", cwd=repo)
        git("commit", "-q", "-m", "init", cwd=repo)
        # issue-claim.sh refuses a base branch that origin does not carry, so
        # the fixture needs a real (local, bare) origin with main on it -- the
        # guard arrived after this test did, and without this the run dies at
        # the guard instead of exercising blockedBy at all.
        origin = Path(directory) / "origin.git"
        subprocess.run(["git", "init", "-q", "--bare", str(origin)], check=True)
        git("remote", "add", "origin", str(origin), cwd=repo)
        git("push", "-q", "origin", "main", cwd=repo)

        result = run_claim(repo, stub_dir)

        if "would claim #12" not in result.stdout:
            FAILURES.append(
                "issue #12 (blocker CLOSED) should have been offered despite "
                f"being lower priority than #13:\n--- stdout ---\n{result.stdout}\n"
                f"--- stderr ---\n{result.stderr}"
            )
        if "would claim #13" in result.stdout:
            FAILURES.append(
                "issue #13 (blocker still OPEN) must never be offered:\n"
                f"--- stdout ---\n{result.stdout}"
            )
        if "#13" not in result.stderr or "blocked" not in result.stderr:
            FAILURES.append(
                "issue #13 should be named as skipped: blocked in stderr, since "
                f"it outranks #12:\n--- stderr ---\n{result.stderr}"
            )

    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed:\n", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}\n", file=sys.stderr)
        return 1
    print("issue-claim blockedBy check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
