#!/usr/bin/env python3
"""Self-test for #1063: a merged branch is not a claim on anything.

`issue-claim.sh`'s cross-machine backstop refuses any issue with a remote
branch matching `issue-<n>-*`. The check is right about what it is for --
claim locks are per-machine, so another host's live work is invisible except
as a branch -- and it cannot tell that branch apart from one whose work
already merged. `issue-land.sh` deletes the branch it merges, so a leftover
is one whose landing was killed in between, and this workstation kills long
commands.

The consequence is not a slow claim, it is a *silent* one: the issue becomes
permanently unclaimable, and the script used to report it in the same breath
as "nothing to do".

Cases:
  * remote branch whose patch is already on the base -> claimable, and the
    stale branch is removed rather than left to collide with the new one;
  * remote branch holding a commit that is not      -> still refused, naming
    the branch, because that is what the backstop exists for.

Usage: scripts/tests/test-issue-claim-stale-branch.py
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
        "number": 4242,
        "title": "An issue whose branch is lying around",
        "labels": [{"name": "ready"}, {"name": "p2"}],
        "assignees": [],
    }
]

GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then cat "$STUB_DIR/issues.json"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then echo "OPEN ready,p2"; exit 0; fi
exit 0
"""

FAILURES: list[str] = []


def git(*args: str, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", "-c", "user.email=t@example.com", "-c", "user.name=T", *args],
        cwd=cwd, check=True, capture_output=True, text=True,
    )


def world(base: Path, landed: bool) -> tuple[Path, Path]:
    """A repo with a bare origin carrying `issue-4242-x`, whose single commit
    either is or is not already on `main` by patch id."""
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
    (repo / "README.md").write_text("fixture\n", encoding="utf-8")
    (repo / ".gitignore").write_text("target/\n", encoding="utf-8")
    git("add", "-A", cwd=repo)
    git("commit", "-q", "-m", "init", cwd=repo)
    origin = base / "origin.git"
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
    git("remote", "add", "origin", str(origin), cwd=repo)
    git("push", "-q", "origin", "main", cwd=repo)

    git("checkout", "-q", "-b", "issue-4242-x", cwd=repo)
    (repo / "work.txt").write_text("the work\n", encoding="utf-8")
    git("add", "-A", cwd=repo)
    git("commit", "-q", "-m", "the work", cwd=repo)
    git("push", "-q", "origin", "issue-4242-x", cwd=repo)
    git("checkout", "-q", "main", cwd=repo)

    if landed:
        # The same patch under a different sha, which is what a rebase-merge
        # leaves and what a sha comparison cannot see.
        (repo / "work.txt").write_text("the work\n", encoding="utf-8")
        git("add", "-A", cwd=repo)
        git("commit", "-q", "-m", "the work, rebased", cwd=repo)
        git("push", "-q", "origin", "main", cwd=repo)
    # The branch must not be checked out anywhere, or the claim cannot use it.
    git("branch", "-D", "issue-4242-x", cwd=repo)
    return repo, stub_dir


def claim(repo: Path, base: Path, stub_dir: Path, *args: str):
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_WORKTREES"] = str(base / "worktrees")
    environment["POSTIO_CLAIMS"] = str(base / "claims")
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    return subprocess.run(
        ["bash", str(repo / "scripts" / "issue-claim.sh"), *args],
        cwd=repo, env=environment, capture_output=True, text=True, timeout=120,
    )


def origin_branches(base: Path) -> list[str]:
    return sorted(
        subprocess.run(
            ["git", "-C", str(base / "origin.git"), "for-each-ref",
             "--format=%(refname:short)", "refs/heads"],
            capture_output=True, text=True, check=True,
        ).stdout.split()
    )


def check(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}\n    {detail}")


def main() -> int:
    if not ISSUE_CLAIM.exists():
        print(f"missing {ISSUE_CLAIM}", file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory() as directory:
        base = Path(directory)
        repo, stub_dir = world(base, landed=True)
        result = claim(repo, base, stub_dir, "4242")
        output = result.stdout + result.stderr
        check(
            "an issue whose branch already merged is claimable",
            result.returncode == 0 and (base / "worktrees" / "issue-4242").is_dir(),
            f"exit {result.returncode}\n{output}",
        )
        check(
            "and it says the branch was stale rather than saying nothing",
            "stale" in output.lower() and "issue-4242-x" in output,
            f"output was:\n{output}",
        )
        check(
            "the stale branch is gone, so it cannot collide with the new one",
            "issue-4242-x" not in origin_branches(base),
            f"origin still has {origin_branches(base)}",
        )

    with tempfile.TemporaryDirectory() as directory:
        base = Path(directory)
        repo, stub_dir = world(base, landed=False)
        result = claim(repo, base, stub_dir, "4242")
        output = result.stdout + result.stderr
        check(
            "an issue whose branch holds unlanded work is still refused",
            result.returncode != 0 and not (base / "worktrees" / "issue-4242").is_dir(),
            f"exit {result.returncode}\n{output}",
        )
        check(
            "the refusal names the branch and says it is unlanded",
            "issue-4242-x" in output and "not on main" in output,
            f"output was:\n{output}",
        )
        check(
            "and it leaves that branch exactly where it is",
            "issue-4242-x" in origin_branches(base),
            "the refusal deleted the work it was protecting",
        )

    for failure in FAILURES:
        print(f"FAIL: {failure}")
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed")
        return 1
    print("issue-claim stale-branch check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
