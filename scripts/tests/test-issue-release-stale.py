#!/usr/bin/env python3
"""Self-test for #328: the stale sweep must never clear a claim whose
worktree exists.

`issue-release.sh --stale` has an "orphaned lock" branch that drops any claim
whose issue is not labelled `in-progress`. The label can go missing while a
session is mid-work (a failed `gh issue edit` at claim time, a human
relabeling, another sweep) -- and the branch never looked at the worktree. A
dropped lock plus a live worktree is what let a second claim walk into the
first session's tree.

Cases:
  * lock + no in-progress label + worktree EXISTS  -> the lock survives;
  * lock + no in-progress label + worktree gone    -> the lock is cleared
    (the branch's legitimate purpose, kept).

Usage: scripts/tests/test-issue-release-stale.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
ISSUE_RELEASE = HERE / "issue-release.sh"

# `issue view` reports the issue open, claimed by nobody visible: no
# in-progress label, which is exactly the state the orphan branch fires on.
GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
    echo "OPEN ready,p2"
    exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
if [ "$1" = "api" ]; then echo "null"; exit 0; fi
exit 1
"""

FAILURES: list[str] = []


def sweep(base: Path, repo: Path, stub_dir: Path):
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["POSTIO_MAIN_CHECKOUT"] = str(repo)
    environment["POSTIO_WORKTREES"] = str(base / "worktrees")
    environment["POSTIO_CLAIMS"] = str(base / "claims")
    return subprocess.run(
        ["bash", str(repo / "scripts" / "issue-release.sh"), "--stale", "0"],
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
        repo = base / "repo"
        stub_dir = base / "stub"
        (stub_dir / "bin").mkdir(parents=True)
        (repo / "scripts").mkdir(parents=True)
        shutil.copy(ISSUE_RELEASE, repo / "scripts" / "issue-release.sh")
        (repo / "scripts" / "issue-release.sh").chmod(0o755)
        shutil.copytree(HERE / "lib", repo / "scripts" / "lib")
        gh = stub_dir / "bin" / "gh"
        gh.write_text(GH_STUB, encoding="utf-8")
        gh.chmod(0o755)

        # A repo with a local bare origin carrying no issue branches, so the
        # remote-branch backstop finds nothing.
        subprocess.run(["git", "init", "-q", "-b", "main", str(repo)], check=True)
        (repo / "README.md").write_text("fixture\n", encoding="utf-8")
        subprocess.run(
            ["git", "-C", str(repo), "-c", "user.email=test@example.com",
             "-c", "user.name=Test", "add", "-A"],
            check=True, capture_output=True,
        )
        subprocess.run(
            ["git", "-C", str(repo), "-c", "user.email=test@example.com",
             "-c", "user.name=Test", "commit", "-q", "-m", "init"],
            check=True, capture_output=True,
        )
        origin = base / "origin.git"
        subprocess.run(["git", "init", "-q", "--bare", str(origin)], check=True)
        subprocess.run(
            ["git", "-C", str(repo), "remote", "add", "origin", str(origin)],
            check=True,
        )
        subprocess.run(
            ["git", "-C", str(repo), "push", "-q", "origin", "main"], check=True
        )

        # Case 1: the label is gone but the worktree is alive.
        live_lock = base / "claims" / "issue-41"
        live_lock.mkdir(parents=True)
        live_tree = base / "worktrees" / "issue-41"
        live_tree.mkdir(parents=True)
        (live_tree / "half-done.rs").write_text("// mid-work\n", encoding="utf-8")

        # Case 2: same label state, but the worktree is genuinely gone.
        dead_lock = base / "claims" / "issue-42"
        dead_lock.mkdir(parents=True)

        result = sweep(base, repo, stub_dir)
        out = result.stdout + result.stderr
        case(
            "a lock with a live worktree survives the sweep",
            live_lock.exists(),
            f"the sweep cleared a claim whose worktree exists:\n{out}",
        )
        case(
            "a lock with no worktree and no label is cleared",
            not dead_lock.exists(),
            f"the orphan cleanup stopped working:\n{out}",
        )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("issue-release stale-sweep check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
