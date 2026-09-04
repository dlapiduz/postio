#!/usr/bin/env python3
"""Self-test for #1012: `issue-claim.sh --reuse` works the next issue in the
worktree the session is already in, instead of a fresh one with a cold
`target/`.

A new worktree rebuilds Postio's own ~20 crates before it can report a single
gate — twelve minutes on #860's landing, for a four-line shell change. The
third-party crates come back from sccache; these do not.

**What makes it safe is that it is one workspace, not two.** Sharing a
`CARGO_TARGET_DIR` between worktrees is the p1 in #76: two trees present cargo
the same relative paths and package versions, land in the same build slot, and
hand each other stale libraries — so a suite can pass against a library the
change never reached. One tree with a different branch checked out cannot do
that. This test is therefore as much about the refusals as about the reuse:
every precondition that keeps it to one tree, and keeps work from being
stranded, is asserted here.

Usage: scripts/tests/test-issue-claim-reuse.py
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
        "number": 4242,
        "title": "A second issue to work",
        "labels": [{"name": "ready"}, {"name": "p2"}],
        "assignees": [],
    }
]

GH_STUB = """#!/bin/bash
# `require-gh.sh` gates on this before anything else runs.
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
    cat "$STUB_DIR/issues.json"
    exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then echo "OPEN ready,p2"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
if [ "$1" = "api" ]; then echo "null"; exit 0; fi
exit 1
"""

FAILURES: list[str] = []


def git(*args: str, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
        text=True,
    )


def world(base: Path) -> tuple[Path, Path]:
    """A fixture repo with a local bare origin, a stubbed gh, and the scripts."""
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
    # As the real repo does. Without it the build cache this feature exists
    # to keep would itself read as uncommitted work, and the clean check
    # would refuse every reuse.
    (repo / ".gitignore").write_text("target/\n", encoding="utf-8")
    git("add", "-A", cwd=repo)
    git("commit", "-q", "-m", "init", cwd=repo)
    origin = base / "origin.git"
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
    git("remote", "add", "origin", str(origin), cwd=repo)
    git("push", "-q", "origin", "main", cwd=repo)
    return repo, stub_dir


def claim(repo: Path, base: Path, stub_dir: Path, *args: str, cwd: Path | None = None):
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_WORKTREES"] = str(base / "worktrees")
    environment["POSTIO_CLAIMS"] = str(base / "claims")
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    # By absolute path: the caller's cwd is the worktree under test, and the
    # script resolves REPO_ROOT from its own location rather than from cwd.
    return subprocess.run(
        ["bash", str(repo / "scripts" / "issue-claim.sh"), *args],
        cwd=cwd or repo,
        env=environment,
        capture_output=True,
        text=True,
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

        # A worktree to reuse, as a landed session leaves one: on its own
        # branch, clean, with nothing that is not on main. A file under
        # `target/` stands in for the build cache the whole feature exists
        # to keep.
        first = worktrees / "issue-1"
        worktrees.mkdir(parents=True, exist_ok=True)
        git("worktree", "add", "--quiet", "-b", "issue-1-done", str(first), "main", cwd=repo)
        (first / "target").mkdir()
        (first / "target" / "warm").write_text("a compiled artifact\n", encoding="utf-8")

        # ── it reuses, renames, and keeps target/ ────────────────────────
        result = claim(repo, base, stub_dir, "--reuse", "4242", cwd=first)
        moved = worktrees / "issue-4242"
        if result.returncode != 0:
            fail("reuse", "the claim failed", result)
        elif not moved.is_dir():
            fail("reuse", f"the tree is not at {moved}", result)
        elif not (moved / "target" / "warm").is_file():
            fail("reuse", "target/ did not come along, which is the whole point", result)
        elif first.exists():
            fail("reuse", "the old path is still there, so it was copied not moved", result)
        else:
            branch = git("rev-parse", "--abbrev-ref", "HEAD", cwd=moved).stdout.strip()
            if not branch.startswith("issue-4242-"):
                fail("reuse", f"checked out {branch!r}, not the new issue's branch", result)
            if "reused, already warm" not in result.stdout:
                fail("reuse", "it did not say the target was reused", result)

        # Everything below reuses the moved tree, so a failure above makes
        # them meaningless rather than merely red.
        if FAILURES:
            for failure in FAILURES:
                print(failure, file=sys.stderr)
            return 1

        # ── a dirty tree is refused, and left exactly as it was ──────────
        (moved / "scratch.txt").write_text("uncommitted\n", encoding="utf-8")
        result = claim(repo, base, stub_dir, "--reuse", "4242", cwd=moved)
        if result.returncode == 0:
            fail("dirty", "reused a tree with uncommitted changes", result)
        elif "uncommitted" not in result.stderr:
            fail("dirty", "refused without saying why", result)
        elif not (moved / "scratch.txt").is_file():
            fail("dirty", "the refusal ate the uncommitted file", result)
        (moved / "scratch.txt").unlink()

        # ── unlanded commits are refused rather than stranded ────────────
        (moved / "work.txt").write_text("landed nowhere\n", encoding="utf-8")
        git("add", "-A", cwd=moved)
        git("commit", "-q", "-m", "feat: not landed", cwd=moved)
        head = git("rev-parse", "HEAD", cwd=moved).stdout.strip()
        result = claim(repo, base, stub_dir, "--reuse", "4242", cwd=moved)
        if result.returncode == 0:
            fail("unlanded", "reused a tree holding commits nobody merged", result)
        elif "ahead of origin/main" not in result.stderr:
            fail("unlanded", "refused without naming the unlanded commits", result)
        elif git("rev-parse", "HEAD", cwd=moved).stdout.strip() != head:
            fail("unlanded", "the refusal moved HEAD", result)

        # ── the shared checkout is never reused ──────────────────────────
        result = claim(repo, base, stub_dir, "--reuse", "4242", cwd=repo)
        if result.returncode == 0:
            fail("shared", "reused the shared checkout, where other work lives", result)
        elif "only reuses a worktree under" not in result.stderr:
            fail("shared", "refused without saying why", result)

    if FAILURES:
        for failure in FAILURES:
            print(failure, file=sys.stderr)
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-claim --reuse self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
