#!/usr/bin/env python3
"""Self-test for #1102, part two: run from inside a landed worktree, the
claim script reuses it without being asked, and falls back to a fresh tree
when reuse would strand something.

`--reuse` exists since #1012 and was used 3 times in 396 claims, because it
is a flag and the loop in CLAUDE.md is `issue-claim.sh` with no flag. The
default is now the thing the docs already said to do: inside a worktree
under `$POSTIO_WORKTREES`, try to reuse; if that refuses -- dirty tree,
unlanded commits, a base gone from origin -- say why and claim a fresh
(seeded) tree instead, leaving the old one exactly as it was. `--reuse`
stays explicit and strict, so its refusals still exit 2; `--fresh` asks for
a new tree outright.

Usage: scripts/tests/test-issue-claim-reuse-default.py
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
    for number in (4242, 4243, 4244, 4245)
]

GH_STUB = """#!/bin/bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then cat "$STUB_DIR/issues.json"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then echo "OPEN ready,p2"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
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


def claim(repo: Path, base: Path, stub_dir: Path, *args: str, cwd: Path):
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_WORKTREES"] = str(base / "worktrees")
    environment["POSTIO_CLAIMS"] = str(base / "claims")
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    return subprocess.run(
        ["bash", str(repo / "scripts" / "issue-claim.sh"), *args],
        cwd=cwd, env=environment, capture_output=True, text=True, timeout=120,
    )


def fail(name: str, message: str, result) -> None:
    FAILURES.append(
        f"{name}: {message} (exit {result.returncode})\n"
        f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}"
    )


def landed_worktree(repo: Path, worktrees: Path, number: int) -> Path:
    tree = worktrees / f"issue-{number}"
    worktrees.mkdir(parents=True, exist_ok=True)
    git("worktree", "add", "--quiet", "-b", f"issue-{number}-done", str(tree), "main", cwd=repo)
    (tree / "target").mkdir()
    (tree / "target" / "warm").write_text("a compiled artifact\n", encoding="utf-8")
    return tree


def main() -> int:
    with tempfile.TemporaryDirectory() as directory:
        base = Path(directory)
        repo, stub_dir = world(base)
        worktrees = base / "worktrees"

        # ── no flag, inside a landed worktree: it is reused ──────────────
        first = landed_worktree(repo, worktrees, 1)
        result = claim(repo, base, stub_dir, "4242", cwd=first)
        moved = worktrees / "issue-4242"
        if result.returncode != 0:
            fail("implicit", "the claim failed", result)
        elif not moved.is_dir() or first.exists():
            fail("implicit", "the worktree was not moved to the new issue", result)
        elif not (moved / "target" / "warm").is_file():
            fail("implicit", "target/ did not come along", result)
        elif "reused" not in result.stdout:
            fail("implicit", "it did not say the target was reused", result)

        if FAILURES:
            for failure in FAILURES:
                print(failure, file=sys.stderr)
            return 1

        # ── a dirty tree falls back to a fresh claim, and says why ───────
        (moved / "scratch.txt").write_text("uncommitted\n", encoding="utf-8")
        result = claim(repo, base, stub_dir, "4243", cwd=moved)
        fresh = worktrees / "issue-4243"
        if result.returncode != 0:
            fail("dirty-fallback", "the claim failed instead of falling back", result)
        elif not fresh.is_dir():
            fail("dirty-fallback", "no fresh worktree was claimed", result)
        elif not moved.is_dir() or not (moved / "scratch.txt").is_file():
            fail("dirty-fallback", "the fallback disturbed the dirty tree", result)
        elif "uncommitted" not in result.stdout + result.stderr:
            fail("dirty-fallback", "did not say why the tree was not reused", result)
        (moved / "scratch.txt").unlink()

        # ── --reuse stays strict: the same dirty tree is a refusal ───────
        (moved / "scratch.txt").write_text("uncommitted\n", encoding="utf-8")
        result = claim(repo, base, stub_dir, "--reuse", "4244", cwd=moved)
        if result.returncode == 0:
            fail("explicit-strict", "--reuse reused, or fell back from, a dirty tree", result)
        elif (worktrees / "issue-4244").exists():
            fail("explicit-strict", "--reuse claimed a fresh tree; that is not what it means", result)
        (moved / "scratch.txt").unlink()

        # ── --fresh from a clean worktree leaves it alone ────────────────
        result = claim(repo, base, stub_dir, "--fresh", "4244", cwd=moved)
        if result.returncode != 0:
            fail("fresh", "the claim failed", result)
        elif not (worktrees / "issue-4244").is_dir():
            fail("fresh", "no fresh worktree was claimed", result)
        elif not moved.is_dir() or not (moved / "target" / "warm").is_file():
            fail("fresh", "--fresh moved or emptied the tree it was run from", result)

        # ── the shared checkout never reuses, flag or no flag ────────────
        result = claim(repo, base, stub_dir, "--dry-run", cwd=repo)
        if result.returncode != 0 or "would claim" not in result.stdout:
            fail("shared", "a plain claim from the shared checkout stopped working", result)
        elif "reus" in result.stdout + result.stderr:
            fail("shared", "the shared checkout was considered for reuse", result)

    if FAILURES:
        for failure in FAILURES:
            print(failure, file=sys.stderr)
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-claim reuse-by-default self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
