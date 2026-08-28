#!/usr/bin/env python3
"""Self-test for #328: issue-claim.sh must never adopt an existing worktree.

The old behaviour printed "reusing existing worktree" and handed the caller
whatever was in it. Combined with a claim lock going missing while a session
works (the stale sweep could drop one on a label hiccup), that put two live
sessions in one worktree -- where `git add -A` and `git reset --hard` are
exactly the sanctioned commands, and each session tramples the other.

Cases:
  * queue path: the top candidate's worktree already exists -> that issue is
    skipped (its freshly taken lock released again) and the next candidate is
    claimed instead;
  * explicit `issue-claim.sh <n>`: refuses, names issue-release.sh as the
    remedy, and leaves no claim lock behind;
  * control: no pre-existing worktree -> the claim proceeds exactly as before.

Usage: scripts/tests/test-issue-claim-no-adoption.py
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

FIXTURE_ISSUES = [
    {
        "number": 7,
        "title": "top priority, but its worktree already exists",
        "labels": [{"name": "ready"}, {"name": "p0"}],
        "assignees": [],
        "milestone": None,
        "blockedBy": {"nodes": []},
    },
    {
        "number": 8,
        "title": "next in line, should be claimed instead",
        "labels": [{"name": "ready"}, {"name": "p1"}],
        "assignees": [],
        "milestone": None,
        "blockedBy": {"nodes": []},
    },
]

GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
    cat "$STUB_DIR/issues.json"
    exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then echo "stub issue"; exit 0; fi
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


def world(base: Path) -> tuple[Path, Path]:
    """A fixture repo with a local bare origin, and a stubbed gh on PATH."""
    repo = base / "repo"
    stub_dir = base / "stub"
    (stub_dir / "bin").mkdir(parents=True)
    (repo / "scripts").mkdir(parents=True)
    shutil.copy(ISSUE_CLAIM, repo / "scripts" / "issue-claim.sh")
    (repo / "scripts" / "issue-claim.sh").chmod(0o755)
    shutil.copytree(HERE / "lib", repo / "scripts" / "lib")

    gh = stub_dir / "bin" / "gh"
    gh.write_text(GH_STUB, encoding="utf-8")
    gh.chmod(0o755)
    (stub_dir / "issues.json").write_text(json.dumps(FIXTURE_ISSUES), encoding="utf-8")

    git("init", "-q", "-b", "main", cwd=repo)
    (repo / "README.md").write_text("fixture repo\n", encoding="utf-8")
    git("add", "-A", cwd=repo)
    git("commit", "-q", "-m", "init", cwd=repo)
    origin = base / "origin.git"
    subprocess.run(["git", "init", "-q", "--bare", str(origin)], check=True)
    git("remote", "add", "origin", str(origin), cwd=repo)
    git("push", "-q", "origin", "main", cwd=repo)
    return repo, stub_dir


def run_claim(repo: Path, stub_dir: Path, base: Path, *args: str):
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_WORKTREES"] = str(base / "worktrees")
    environment["POSTIO_CLAIMS"] = str(base / "claims")
    return subprocess.run(
        ["bash", str(repo / "scripts" / "issue-claim.sh"), *args],
        cwd=repo,
        env=environment,
        capture_output=True,
        text=True,
        timeout=60,
    )


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)
        repo, stub_dir = world(base)
        # The hazard: issue 7's worktree already exists, with a file in it
        # that stands in for another session's live work.
        stale = base / "worktrees" / "issue-7"
        stale.mkdir(parents=True)
        (stale / "someones-work.rs").write_text("// not yours\n", encoding="utf-8")

        result = run_claim(repo, stub_dir, base, "--dry-run")
        out = result.stdout + result.stderr
        case(
            "queue path skips the issue whose worktree exists",
            "would claim #8" in result.stdout,
            f"expected #8 to be offered instead of #7:\n{out}",
        )
        case(
            "the skipped issue's lock is released again",
            not (base / "claims" / "issue-7").exists(),
            "the lock survived the skip, so #7 is now claimable by nobody",
        )
        case(
            "the untouched worktree still holds the other session's file",
            (stale / "someones-work.rs").exists(),
            "the pre-existing worktree was modified",
        )

        result = run_claim(repo, stub_dir, base, "7")
        case(
            "an explicit claim of that issue refuses",
            result.returncode != 0,
            f"exit {result.returncode}; stdout:\n{result.stdout}",
        )
        case(
            "the refusal names the remedy",
            "issue-release.sh" in (result.stdout + result.stderr),
            f"no remedy named:\n{result.stdout}{result.stderr}",
        )
        case(
            "the explicit refusal leaves no lock behind",
            not (base / "claims" / "issue-7").exists(),
            "a lock was left behind by the refusal",
        )
        case(
            "nothing ever prints 'reusing existing worktree'",
            "reusing existing worktree" not in (result.stdout + result.stderr),
            "the adoption path still exists",
        )

        result = run_claim(repo, stub_dir, base, "8")
        case(
            "a clean claim still works",
            result.returncode == 0 and "claimed #8" in result.stdout,
            f"exit {result.returncode}; stdout:\n{result.stdout}\n{result.stderr}",
        )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("issue-claim no-adoption check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
