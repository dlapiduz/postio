#!/usr/bin/env python3
"""Self-test for #1102, part one: a fresh worktree's `target/debug` is seeded
by copying a sibling's, so the first build compiles what changed rather than
the world.

393 of the 396 claims in this repository's history were fresh worktrees, and
each paid a cold `target/`: 11 to 19 minutes before the first gate could say
anything. Measured on this box, `cp -a --reflink=always` of an 11 GB
`target/debug` took one second on btrfs, and the seeded tree then built the
whole sanity tier in 12 s compiling 3 crates -- against 1149 s and 389
crates cold. The copy is copy-on-write, so it costs no disk until files
diverge, and it is a *copy*: the sharing #76 forbids is two trees writing
one target, and each tree here still owns its own.

The seed is the newest `target/debug` among the sibling worktrees and the
shared checkout. `target/tmp` is never copied -- it is live scratch for
whatever that sibling is running right now. `--cold` and
`POSTIO_CLAIM_SEED=0` opt out.

Usage: scripts/tests/test-issue-claim-seed.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
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


def claim(repo: Path, base: Path, stub_dir: Path, *args: str, env: dict | None = None):
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_WORKTREES"] = str(base / "worktrees")
    environment["POSTIO_CLAIMS"] = str(base / "claims")
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment.pop("POSTIO_CLAIM_SEED", None)
    environment.update(env or {})
    return subprocess.run(
        ["bash", str(repo / "scripts" / "issue-claim.sh"), *args],
        cwd=repo, env=environment, capture_output=True, text=True, timeout=120,
    )


def fail(name: str, message: str, result) -> None:
    FAILURES.append(
        f"{name}: {message} (exit {result.returncode})\n"
        f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}"
    )


def sibling(repo: Path, worktrees: Path, number: int, artifact: str) -> Path:
    """A worktree another session left behind, with a built target/."""
    tree = worktrees / f"issue-{number}"
    worktrees.mkdir(parents=True, exist_ok=True)
    git("worktree", "add", "--quiet", "-b", f"issue-{number}-x", str(tree), "main", cwd=repo)
    (tree / "target" / "debug" / "deps").mkdir(parents=True)
    (tree / "target" / "debug" / ".fingerprint").mkdir()
    (tree / "target" / "debug" / "deps" / artifact).write_text("compiled\n", encoding="utf-8")
    # The sibling's own crates carry *its* path baked in (env!("CARGO_MANIFEST_DIR"));
    # a copy must lose them so cargo rebuilds them for this tree.
    (tree / "target" / "debug" / "deps" / "libpostio_core-4567.rlib").write_text("ours\n", encoding="utf-8")
    (tree / "target" / "debug" / ".fingerprint" / "postio-core-4567").mkdir(parents=True)
    (tree / "target" / "debug" / ".fingerprint" / f"{artifact}-fp").mkdir(parents=True)
    (tree / "target" / "tmp" / "live-test-scratch").mkdir(parents=True)
    (tree / "target" / "tmp" / "live-test-scratch" / "db").write_text("x", encoding="utf-8")
    return tree


def main() -> int:
    with tempfile.TemporaryDirectory() as directory:
        base = Path(directory)
        repo, stub_dir = world(base)
        worktrees = base / "worktrees"

        older = sibling(repo, worktrees, 1, "libolder.rlib")
        time.sleep(1.1)  # mtimes at one-second resolution on some filesystems
        newer = sibling(repo, worktrees, 2, "libnewer.rlib")

        # ── the newest sibling seeds the fresh tree, minus its scratch ───
        result = claim(repo, base, stub_dir, "4242")
        tree = worktrees / "issue-4242"
        if result.returncode != 0:
            fail("seed", "the claim failed", result)
        elif not (tree / "target" / "debug" / "deps" / "libnewer.rlib").is_file():
            fail("seed", "target/debug was not seeded from the newest sibling", result)
        elif (tree / "target" / "debug" / "deps" / "libolder.rlib").is_file():
            fail("seed", "seeded from the older sibling, not the newest", result)
        elif (tree / "target" / "tmp" / "live-test-scratch").exists():
            fail("seed", "copied the sibling's live target/tmp scratch", result)
        elif not (tree / "target" / "tmp").is_dir():
            fail("seed", "target/tmp was not created (#178, #219)", result)
        elif "seeded" not in result.stdout or str(newer) not in result.stdout:
            fail("seed", "did not say where the seed came from", result)
        elif not (newer / "target" / "debug" / "deps" / "libnewer.rlib").is_file():
            fail("seed", "the copy moved the sibling's artifacts instead of copying", result)
        elif (tree / "target" / "debug" / "deps" / "libpostio_core-4567.rlib").exists() \
                or (tree / "target" / "debug" / ".fingerprint" / "postio-core-4567").exists():
            fail("seed", "the sibling's own crates came along with its path baked in", result)
        elif not (tree / "target" / "debug" / ".fingerprint" / "libnewer.rlib-fp").is_dir():
            fail("seed", "a dependency's fingerprint was dropped with ours", result)
        elif not (newer / "target" / "debug" / "deps" / "libpostio_core-4567.rlib").is_file():
            fail("seed", "the drop reached into the sibling's target", result)

        # ── --cold gets a genuinely empty target ─────────────────────────
        result = claim(repo, base, stub_dir, "--cold", "4243")
        tree = worktrees / "issue-4243"
        if result.returncode != 0:
            fail("cold", "the claim failed", result)
        elif (tree / "target" / "debug").exists():
            fail("cold", "--cold still seeded target/debug", result)
        elif not (tree / "target" / "tmp").is_dir():
            fail("cold", "target/tmp was not created", result)

        # ── and so does POSTIO_CLAIM_SEED=0 ──────────────────────────────
        result = claim(repo, base, stub_dir, "4244", env={"POSTIO_CLAIM_SEED": "0"})
        tree = worktrees / "issue-4244"
        if result.returncode != 0:
            fail("env-off", "the claim failed", result)
        elif (tree / "target" / "debug").exists():
            fail("env-off", "POSTIO_CLAIM_SEED=0 still seeded target/debug", result)

        # ── with no sibling, the shared checkout's target is the seed ────
        for tree in (older, newer, worktrees / "issue-4242", worktrees / "issue-4243",
                     worktrees / "issue-4244"):
            if tree.is_dir():
                git("worktree", "remove", "--force", str(tree), cwd=repo)
        (repo / "target" / "debug" / "deps").mkdir(parents=True)
        (repo / "target" / "debug" / "deps" / "libshared.rlib").write_text("c\n", encoding="utf-8")
        result = claim(repo, base, stub_dir, "4245")
        tree = worktrees / "issue-4245"
        if result.returncode != 0:
            fail("shared", "the claim failed", result)
        elif not (tree / "target" / "debug" / "deps" / "libshared.rlib").is_file():
            fail("shared", "the shared checkout's target/debug was not used as the seed", result)

    if FAILURES:
        for failure in FAILURES:
            print(failure, file=sys.stderr)
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-claim seed self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
