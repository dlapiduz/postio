#!/usr/bin/env python3
"""A claim that fails must leave no lock behind (#1255).

The claim lock is taken with `mkdir` *before* the worktree is cut, which is
what makes it atomic. Nothing released it when a later step failed, so one
failed claim made an issue permanently unclaimable: every attempt afterwards
found the wreckage of the previous one and reported

    #N is claimed by another session, trying the next one.
    Every candidate is genuinely taken. Stop here and say so.

with no other session, no worktree, and a lock the script itself had orphaned.
The state was stable and wrong, and the closing line told the reader to stop.

The way in was a local branch left behind when a worktree was reused for
another issue: `git worktree add -b <branch>` fails with "a branch named ...
already exists", the script exits 255, and the lock stays.

Both halves are covered here: the lock is freed however the claim fails, and a
stale local branch holding nothing unlanded is cleared rather than being a
permanent wall.

Usage: scripts/tests/test-issue-claim-failed-claim-frees-lock.py
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

FAILURES: list[str] = []

ISSUE_NUMBER = 50
ISSUE_TITLE = "a claim whose worktree cannot be cut"
# `slug()` in issue-claim.sh: lowercase, non-alphanumerics to `-`, 40 chars.
BRANCH = "issue-50-a-claim-whose-worktree-cannot-be-cut"

FIXTURE_ISSUES = [
    {
        "number": ISSUE_NUMBER,
        "title": ISSUE_TITLE,
        "labels": [{"name": "ready"}, {"name": "p2"}],
        "assignees": [],
        "milestone": None,
        "blockedBy": {"nodes": []},
    }
]

GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then cat "$STUB_DIR/issues.json"; exit 0; fi
if [ "$1" = "pr" ]; then echo "[]"; exit 0; fi
if [ "$1" = "api" ]; then echo "[]"; exit 0; fi
exit 0
"""


def git(*args: str, cwd: Path) -> None:
    subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
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
    git("add", "-A", cwd=repo)
    git("commit", "-q", "-m", "init", cwd=repo)
    origin = base / "origin.git"
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
    git("remote", "add", "origin", str(origin), cwd=repo)
    git("push", "-q", "origin", "main", cwd=repo)
    return repo, stub_dir


def run_claim(repo: Path, stub_dir: Path, base: Path, *args: str):
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_MAIN_CHECKOUT"] = str(repo)
    environment["POSTIO_WORKTREES"] = str(base / "worktrees")
    environment["POSTIO_CLAIMS"] = str(base / "claims")
    return patience.run(
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
        claims = base / "claims"
        lock = claims / f"issue-{ISSUE_NUMBER}"

        # -- The trap, proved by a failure the script does not anticipate ----
        #
        # A read-only worktrees directory: `git worktree add` cannot create
        # anything under it, so the claim dies *after* taking its lock and
        # before earning it. The point is that the release does not depend on
        # the script having foreseen this particular failure -- it is the one
        # class of bug that produced #1255.
        worktrees = base / "worktrees"
        worktrees.mkdir(parents=True, exist_ok=True)
        worktrees.chmod(0o500)
        try:
            failed = run_claim(repo, stub_dir, base)
        finally:
            worktrees.chmod(0o700)

        case(
            "a claim that dies mid-way does not claim the issue",
            failed.returncode != 0,
            f"the claim reported success with an unusable worktree directory: "
            f"{failed.stdout!r}",
        )
        case(
            "and it leaves no lock behind",
            not lock.exists(),
            f"{lock} survived a failed claim, so every later attempt reports a "
            f"session that does not exist. "
            f"stdout={failed.stdout!r} stderr={failed.stderr!r}",
        )

        after = run_claim(repo, stub_dir, base)
        combined = after.stdout + after.stderr
        case(
            "so the next attempt is not blocked by it",
            "claimed by another session" not in combined,
            f"the next attempt blamed a session that does not exist:\n{combined}",
        )
        case(
            "and it claims the issue",
            after.returncode == 0 and f"claimed #{ISSUE_NUMBER}" in after.stdout,
            f"exit {after.returncode}: {combined}",
        )

    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)
        repo, stub_dir = world(base)

        # -- A stale local branch is not a permanent wall --------------------
        #
        # What reusing a worktree for another issue leaves behind. Nothing on
        # it is missing from main, so it is not a claim on anything.
        git("branch", BRANCH, "main", cwd=repo)
        landed = run_claim(repo, stub_dir, base)
        case(
            "a stale local branch holding nothing unlanded is cleared",
            landed.returncode == 0 and f"claimed #{ISSUE_NUMBER}" in landed.stdout,
            f"exit {landed.returncode}: {landed.stdout!r} {landed.stderr!r}",
        )

    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)
        repo, stub_dir = world(base)

        # -- But one holding work is refused, not deleted --------------------
        git("checkout", "-q", "-b", BRANCH, cwd=repo)
        (repo / "somebody-was-here.txt").write_text("unlanded\n", encoding="utf-8")
        git("add", "-A", cwd=repo)
        git("commit", "-q", "-m", "unlanded work", cwd=repo)
        git("checkout", "-q", "main", cwd=repo)

        refused = run_claim(repo, stub_dir, base)
        combined = refused.stdout + refused.stderr
        case(
            "a stale local branch holding work is refused rather than deleted",
            "unlanded work" in combined or "not on main" in combined,
            f"the claim did not say why it stopped:\n{combined}",
        )
        still_there = subprocess.run(
            ["git", "-C", str(repo), "show-ref", "--verify", "--quiet",
             f"refs/heads/{BRANCH}"],
        ).returncode == 0
        case(
            "and the branch survives",
            still_there,
            "somebody's unlanded work was deleted by a claim",
        )
        case(
            "and that refusal leaves no lock either",
            not (base / "claims" / f"issue-{ISSUE_NUMBER}").exists(),
            "a refused claim kept its lock",
        )

    for failure in FAILURES:
        print(f"FAIL  {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed", file=sys.stderr)
        return 1
    print("\nissue-claim failed-claim lock: all cases behaved")
    return 0


if __name__ == "__main__":
    sys.exit(main())
