#!/usr/bin/env python3
"""Self-test for #1107's other half: getting back to a branch whose PR went
red after the session moved on.

With auto-merge the landing script arms the merge and returns, and the
session claims its next issue at once (test-issue-claim-reuse-default.py
covers moving off a pushed branch). If CI then fails, the branch is on
origin with an open PR and nobody in front of it. Two things make that
recoverable:

  * `issue-claim.sh --resume <n>` cuts a worktree from the existing remote
    branch rather than from the base, so the fix goes onto the same PR;
  * a plain claim first lists the caller's open PRs with failing checks,
    naming that command -- so the next session to claim anything sees the
    red one before taking new work. `/steward` sweeps the same list.

Usage: scripts/tests/test-issue-claim-resume.py
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
ISSUE_RELEASE = HERE / "issue-release.sh"

FIXTURE_ISSUES = [
    {
        "number": number,
        "title": "An issue to work",
        "labels": [{"name": "ready"}, {"name": "p2"}],
        "assignees": [],
        "milestone": None,
        "blockedBy": {"nodes": []},
    }
    for number in (4242, 4243)
]

GH_STUB = """#!/bin/bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then cat "$STUB_DIR/issues.json"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then echo "OPEN ready,p2"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then cat "$STUB_DIR/prs.json" 2>/dev/null || echo "[]"; exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then echo "OPEN"; exit 0; fi
if [ "$1" = "api" ]; then echo "null"; exit 0; fi
exit 1
"""

FAILURES: list[str] = []


def git(*args: str, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd, check=True, capture_output=True, text=True,
    )


def world(base: Path) -> tuple[Path, Path]:
    repo = base / "repo"
    stub_dir = base / "stub"
    (stub_dir / "bin").mkdir(parents=True)
    (repo / "scripts").mkdir(parents=True)
    shutil.copy(ISSUE_CLAIM, repo / "scripts" / "issue-claim.sh")
    shutil.copy(ISSUE_RELEASE, repo / "scripts" / "issue-release.sh")
    (repo / "scripts" / "issue-claim.sh").chmod(0o755)
    (repo / "scripts" / "issue-release.sh").chmod(0o755)
    shutil.copytree(HERE / "lib", repo / "scripts" / "lib")
    gh = stub_dir / "bin" / "gh"
    gh.write_text(GH_STUB, encoding="utf-8")
    gh.chmod(0o755)
    (stub_dir / "issues.json").write_text(json.dumps(FIXTURE_ISSUES), encoding="utf-8")
    git("init", "-q", "-b", "main", cwd=repo)
    (repo / "README.md").write_text("fixture repo\n", encoding="utf-8")
    (repo / ".gitignore").write_text("target/\n", encoding="utf-8")
    git("add", "-A", cwd=repo)
    git("commit", "-q", "-m", "init", cwd=repo)
    origin = base / "origin.git"
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
    git("remote", "add", "origin", str(origin), cwd=repo)
    git("push", "-q", "origin", "main", cwd=repo)
    return repo, stub_dir


def claim(repo: Path, base: Path, stub_dir: Path, *args: str):
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_WORKTREES"] = str(base / "worktrees")
    environment["POSTIO_CLAIMS"] = str(base / "claims")
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["POSTIO_CLAIM_SEED"] = "0"
    return subprocess.run(
        ["bash", str(repo / "scripts" / "issue-claim.sh"), *args],
        cwd=repo, env=environment, capture_output=True, text=True, timeout=120,
    )


def fail(name: str, message: str, result) -> None:
    FAILURES.append(
        f"{name}: {message} (exit {result.returncode})\n"
        f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}"
    )


def main() -> int:
    with tempfile.TemporaryDirectory() as directory:
        base = Path(directory)
        repo, stub_dir = world(base)
        worktrees = base / "worktrees"

        # A branch on origin with work on it and, per the stub, an open PR
        # whose checks failed.
        git("checkout", "-q", "-b", "issue-4242-red-pr", cwd=repo)
        (repo / "work.txt").write_text("landed on the branch\n", encoding="utf-8")
        git("add", "-A", cwd=repo)
        git("commit", "-q", "-m", "feat: the work", cwd=repo)
        tip = git("rev-parse", "HEAD", cwd=repo).stdout.strip()
        git("push", "-q", "-u", "origin", "issue-4242-red-pr", cwd=repo)
        git("checkout", "-q", "main", cwd=repo)
        git("branch", "-q", "-D", "issue-4242-red-pr", cwd=repo)
        (stub_dir / "prs.json").write_text(json.dumps([{
            "number": 77,
            "headRefName": "issue-4242-red-pr",
            "url": "https://example.com/pull/77",
            "statusCheckRollup": [
                {"name": "Tests", "conclusion": "FAILURE"},
                {"name": "Clippy", "conclusion": "SUCCESS"},
            ],
        }]), encoding="utf-8")

        # ── a plain claim names the red PR before taking new work ────────
        result = claim(repo, base, stub_dir, "--dry-run")
        out = result.stdout + result.stderr
        if "#77" not in out or "--resume 4242" not in out:
            fail("notice", "a dry run did not name the red PR and the resume command", result)

        # ── --resume cuts the worktree from the remote branch ────────────
        result = claim(repo, base, stub_dir, "--resume", "4242")
        tree = worktrees / "issue-4242"
        if result.returncode != 0:
            fail("resume", "the resume failed", result)
        elif not tree.is_dir():
            fail("resume", f"no worktree at {tree}", result)
        else:
            head = git("rev-parse", "HEAD", cwd=tree).stdout.strip()
            branch = git("rev-parse", "--abbrev-ref", "HEAD", cwd=tree).stdout.strip()
            if head != tip:
                fail("resume", f"HEAD is {head[:7]}, not the branch tip {tip[:7]}", result)
            if branch != "issue-4242-red-pr":
                fail("resume", f"checked out {branch!r}, not the PR's branch", result)
            if not (tree / "work.txt").is_file():
                fail("resume", "the branch's work is not in the tree", result)
            upstream = git("rev-parse", "--abbrev-ref", "@{upstream}", cwd=tree).stdout.strip()
            if upstream != "origin/issue-4242-red-pr":
                fail("resume", f"upstream is {upstream!r}; a push would not update the PR", result)
            if "77" not in result.stdout:
                fail("resume", "did not name the PR being resumed", result)

        # ── no such branch: refuse rather than start from the base ───────
        result = claim(repo, base, stub_dir, "--resume", "4243")
        if result.returncode == 0:
            fail("resume-missing", "resumed an issue with no branch on origin", result)
        elif (worktrees / "issue-4243").exists():
            fail("resume-missing", "created a worktree anyway", result)

    if FAILURES:
        for failure in FAILURES:
            print(failure, file=sys.stderr)
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-claim --resume self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
