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

# The shared dial (#1249). `scripts/lib`, not beside this file, because CI
# runs every `scripts/tests/*.py` it finds as a self-test.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

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
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
    # Real `gh` applies `--jq` to what it prints, and #1401's fix reads a
    # single field that way. A stub that ignored the filter would hand the
    # script a whole JSON array where it expects a branch name, and the test
    # would be exercising something the tool never does.
    if printf '%s' "$*" | grep -q -- "baseRefName"; then
        printf '%s' "$PR_BASE_REF"
        exit 0
    fi
    cat "$STUB_DIR/prs.json" 2>/dev/null || echo "[]"
    exit 0
fi
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


def claim(repo: Path, base: Path, stub_dir: Path, *args: str, env_extra: dict[str, str] | None = None):
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_WORKTREES"] = str(base / "worktrees")
    environment["POSTIO_CLAIMS"] = str(base / "claims")
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["POSTIO_CLAIM_SEED"] = "0"
    environment.update(env_extra or {})
    return patience.run(
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

        # ── --resume records the base the PR actually targets (#1401) ────
        # `--base` is a claim-time argument and nobody passes it to
        # `--resume`; the branch already exists and its PR already targets
        # something. Recording the default `main` on a branch claimed with
        # `--base feature/x` made `issue-land.sh` refuse to merge -- rightly,
        # because a worktree and a PR that disagree is not a thing to guess
        # about, but the disagreement was this script's own doing and the
        # landing that hit it had already run every gate and pushed.
        result = claim(
            repo, base, stub_dir, "--resume", "4242",
            env_extra={"PR_BASE_REF": "feature/conversation-reading-pane"},
        )
        recorded = (worktrees / "issue-4242" / ".git")
        base_file = None
        if recorded.is_file():
            # a worktree's `.git` is a pointer file
            pointer = recorded.read_text(encoding="utf-8").split(":", 1)[1].strip()
            base_file = Path(pointer) / "postio-base"
        if result.returncode != 0:
            fail("resume base", "the resume failed", result)
        elif base_file is None or not base_file.is_file():
            fail("resume base", "no postio-base was recorded", result)
        elif base_file.read_text(encoding="utf-8").strip() != "feature/conversation-reading-pane":
            fail(
                "resume base",
                "recorded "
                f"{base_file.read_text(encoding='utf-8').strip()!r} rather than the "
                "branch the PR targets, so the landing will refuse to merge",
                result,
            )
        shutil.rmtree(worktrees / "issue-4242", ignore_errors=True)
        subprocess.run(["git", "worktree", "prune"], cwd=repo, check=False, capture_output=True)
        git("branch", "-q", "-D", "issue-4242-red-pr", cwd=repo)
        shutil.rmtree(base / "claims", ignore_errors=True)

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
