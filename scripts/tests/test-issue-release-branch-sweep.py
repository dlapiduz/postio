#!/usr/bin/env python3
"""Self-test for #1077: a merged branch does not survive on `origin`, and
unlanded work is never swept.

`issue-land.sh` deletes the remote branch as its **last** step. A run killed
between the merge and that line leaves it there for ever, and `issue-claim.sh`
refuses an issue that has a remote branch -- so the leftover makes that issue
permanently unclaimable. Measured when this was written: 36 `issue-*` branches
on origin, 30 for issues that were closed.

The dangerous direction is the other one. A branch whose commits are *not*
upstream is somebody's unlanded work and may be the only copy, so the sweep
has to tell the two apart -- by patch id, because a landing rebases and the
shas never match even when the content did land.

Cases:
  * branch whose patch is upstream        -> deleted;
  * branch with a commit that is not      -> left alone, and says why;
  * `--abandon`                           -> left alone (the work is being
    handed back, so the branch is the point).

Usage: scripts/tests/test-issue-release-branch-sweep.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
ISSUE_RELEASE = HERE / "issue-release.sh"

GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
exit 0
"""

FAILURES: list[str] = []


def git(repo: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), *args],
        capture_output=True,
        text=True,
        check=True,
        env={**os.environ, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.com",
             "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.com"},
    )
    return result.stdout.strip()


def world(root: Path, landed: bool) -> tuple[Path, Path]:
    """A bare `origin`, a checkout of it, and an `issue-42-x` branch that has
    either landed on `main` or not."""
    origin = root / "origin.git"
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
    repo = root / "repo"
    subprocess.run(["git", "clone", "-q", str(origin), str(repo)], check=True)
    (repo / "README.md").write_text("start\n")
    git(repo, "add", "README.md")
    git(repo, "commit", "-qm", "start")
    git(repo, "push", "-q", "origin", "main")

    git(repo, "checkout", "-q", "-b", "issue-42-x")
    (repo / "work.txt").write_text("the work\n")
    git(repo, "add", "work.txt")
    git(repo, "commit", "-qm", "the work")
    git(repo, "push", "-q", "origin", "issue-42-x")

    if landed:
        # The same *patch* on main under a different sha, which is what a
        # rebase-merge leaves behind and what `git cherry` is for.
        git(repo, "checkout", "-q", "main")
        (repo / "work.txt").write_text("the work\n")
        git(repo, "add", "work.txt")
        git(repo, "commit", "-qm", "the work")
        git(repo, "push", "-q", "origin", "main")
    git(repo, "checkout", "-q", "main")
    return origin, repo


def release(root: Path, repo: Path, *args: str) -> subprocess.CompletedProcess:
    stub_dir = root / "bin"
    stub_dir.mkdir(exist_ok=True)
    gh = stub_dir / "gh"
    gh.write_text(GH_STUB)
    gh.chmod(0o755)
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir}:{environment['PATH']}"
    environment["POSTIO_MAIN_CHECKOUT"] = str(repo)
    environment["POSTIO_WORKTREES"] = str(root / "worktrees")
    environment["POSTIO_CLAIMS"] = str(root / "claims")
    (root / "worktrees").mkdir(exist_ok=True)
    (root / "claims").mkdir(exist_ok=True)
    return subprocess.run(
        ["bash", str(ISSUE_RELEASE), "42", *args],
        capture_output=True,
        text=True,
        env=environment,
        timeout=60,
    )


def branches(origin: Path) -> list[str]:
    out = subprocess.run(
        ["git", "-C", str(origin), "for-each-ref", "--format=%(refname:short)", "refs/heads"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.split()
    return sorted(out)


def check(name: str, condition: bool, detail: str = "") -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}\n    {detail}")


def main() -> int:
    if not ISSUE_RELEASE.exists():
        print(f"missing {ISSUE_RELEASE}", file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        origin, repo = world(root, landed=True)
        result = release(root, repo)
        check(
            "a branch whose patch is upstream is deleted",
            "issue-42-x" not in branches(origin),
            f"origin still has {branches(origin)}\n{result.stdout}{result.stderr}",
        )

    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        origin, repo = world(root, landed=False)
        result = release(root, repo)
        check(
            "a branch with unlanded work is left alone",
            "issue-42-x" in branches(origin),
            f"the sweep deleted somebody's only copy\n{result.stdout}{result.stderr}",
        )
        check(
            "and it says why, naming the branch",
            "issue-42-x" in result.stderr and "not on main" in result.stderr,
            f"stderr was: {result.stderr!r}",
        )

    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        origin, repo = world(root, landed=True)
        result = release(root, repo, "--abandon")
        check(
            "--abandon never sweeps: the branch is the point of handing it back",
            "issue-42-x" in branches(origin),
            f"origin has {branches(origin)}\n{result.stdout}{result.stderr}",
        )

    for failure in FAILURES:
        print(f"FAIL: {failure}")
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed")
        return 1
    print("issue-release branch-sweep check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
