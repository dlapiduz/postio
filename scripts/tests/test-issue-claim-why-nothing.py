#!/usr/bin/env python3
"""Self-test for #1077: `issue-claim.sh` says which of the three it was.

Three separate things stop a candidate being taken -- a claim lock held by
another session, a worktree already at that path, and a branch on `origin`.
Only the first is "somebody else is on it"; the other two are recoverable, and
a leftover branch is *permanent* until somebody removes it.

The script used to report all three as `Every candidate was already claimed.
Nothing to do.` That is nearly the sentence `/issue` gives for the genuine
"nothing is ready" stop condition, so a session that reads it stops -- which
is what happened when a leftover branch blocked the top candidate while about
two dozen issues were free.

Cases, all with exactly one candidate that is blocked by a remote branch:
  * the reason is named, and the issue number with it;
  * the message does not read as the stop condition;
  * it says what removes the block.

Usage: scripts/tests/test-issue-claim-why-nothing.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
ISSUE_CLAIM = HERE / "issue-claim.sh"

ONE_READY_ISSUE = [
    {
        "number": 42,
        "title": "a perfectly claimable issue",
        "labels": [{"name": "ready"}, {"name": "p2"}],
        "assignees": [],
        "milestone": None,
        "blockedBy": {"nodes": []},
    }
]

GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
    cat "$STUB_DIR/issues.json"
    exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
exit 1
"""

FAILURES: list[str] = []


def git(*args: str, cwd: Path) -> None:
    subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
    )


def check(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}\n    {detail}")


def main() -> int:
    with tempfile.TemporaryDirectory() as directory:
        repo = Path(directory) / "repo"
        stub_dir = Path(directory) / "stub"
        (stub_dir / "bin").mkdir(parents=True)
        (repo / "scripts").mkdir(parents=True)
        shutil.copy(ISSUE_CLAIM, repo / "scripts" / "issue-claim.sh")
        (repo / "scripts" / "issue-claim.sh").chmod(0o755)
        shutil.copytree(HERE / "lib", repo / "scripts" / "lib")

        gh = stub_dir / "bin" / "gh"
        gh.write_text(GH_STUB, encoding="utf-8")
        gh.chmod(0o755)
        (stub_dir / "issues.json").write_text(json.dumps(ONE_READY_ISSUE), encoding="utf-8")

        git("init", "-q", "-b", "main", cwd=repo)
        (repo / "README.md").write_text("fixture repo\n", encoding="utf-8")
        git("add", "-A", cwd=repo)
        git("commit", "-q", "-m", "init", cwd=repo)
        origin = Path(directory) / "origin.git"
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
        git("remote", "add", "origin", str(origin), cwd=repo)
        git("push", "-q", "origin", "main", cwd=repo)

        # The leftover a killed landing leaves: the issue's branch, on origin,
        # with nothing else wrong.
        git("push", "-q", "origin", "main:refs/heads/issue-42-a-perfectly-claimable", cwd=repo)

        environment = dict(os.environ)
        environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
        environment["STUB_DIR"] = str(stub_dir)
        environment["POSTIO_WORKTREES"] = str(repo / "worktrees")
        environment["POSTIO_CLAIMS"] = str(repo / "claims")
        result = subprocess.run(
            ["bash", str(repo / "scripts" / "issue-claim.sh")],
            cwd=repo,
            env=environment,
            capture_output=True,
            text=True,
            timeout=60,
        )
        output = result.stdout + result.stderr

        check(
            "the branch is named as the reason, with its issue",
            "branch on origin" in output and "42" in output,
            f"output was:\n{output}",
        )
        check(
            "it does not read as the genuine stop condition",
            "Every candidate was already claimed" not in output
            and "Nothing to do" not in output,
            "the old wording is what sends a session home with work available:\n"
            f"{output}",
        )
        check(
            "it says what removes the block",
            "issue-release.sh 42" in output,
            f"nothing in the output tells the reader what to do:\n{output}",
        )
        check(
            "and it still exits non-zero, because nothing was claimed",
            result.returncode != 0,
            f"exit was {result.returncode}",
        )

    for failure in FAILURES:
        print(f"FAIL: {failure}")
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed")
        return 1
    print("issue-claim why-nothing check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
