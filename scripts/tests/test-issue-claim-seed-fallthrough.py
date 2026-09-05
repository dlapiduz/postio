#!/usr/bin/env python3
"""Self-test for #1191: a busy seed candidate does not cost the whole seed.

`seed_target` picks the newest sibling `target/debug` to copy from -- and
the newest is, by construction, the most likely to still have a build
actively writing into it. A `cp -a` reading a tree mid-write can fail on a
file that changed or vanished between `readdir` and `open`; #1190 made that
failure say so instead of claiming there was nothing to seed from, but a
single failed candidate still meant a fully cold build even when an older,
quiescent sibling sat right next to it with everything a fresh tree needed.

This is that fallthrough: the newest candidate's copy is made to fail (`cp`
is stubbed to fail once, for the exact path of the newest sibling, and
succeed for everything else -- including the second call `seed_target`
makes for the next-newest candidate), and the claim must still end up
seeded from the sibling that actually worked.

Usage: scripts/tests/test-issue-claim-seed-fallthrough.py
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
        "number": 4247,
        "title": "An issue to work",
        "labels": [{"name": "ready"}, {"name": "p2"}],
        "assignees": [],
        "milestone": None,
        "blockedBy": {"nodes": []},
    }
]

GH_STUB = """#!/bin/bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then cat "$STUB_DIR/issues.json"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then echo "OPEN ready,p2"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
if [ "$1" = "api" ]; then echo "null"; exit 0; fi
exit 1
"""

# Fails the *first* `-a --reflink=auto` copy it is asked for -- the newest
# candidate, since `seed_target` tries candidates in that order -- and
# succeeds (delegating to the real `cp`) for every call after that,
# including the fallthrough attempt on the next-newest candidate. A marker
# file is how it remembers "already failed once" across separate `cp`
# invocations, since each is a fresh process with no shared memory.
CP_STUB = """#!/bin/bash
reflink=0
for arg in "$@"; do
    [ "$arg" = "--reflink=auto" ] && reflink=1
done
if [ "$1" = "-a" ] && [ "$reflink" = "1" ] && [ ! -f "$STUB_DIR/cp-failed-once" ]; then
    touch "$STUB_DIR/cp-failed-once"
    echo "cp: simulated failure for the seed-fallthrough test" >&2
    exit 1
fi
exec /bin/cp "$@"
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
    cp = stub_dir / "bin" / "cp"
    cp.write_text(CP_STUB, encoding="utf-8")
    cp.chmod(0o755)
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
    environment.pop("POSTIO_CLAIM_SEED", None)
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
    (tree / "target" / "debug" / "deps" / artifact).write_text("compiled\n", encoding="utf-8")
    return tree


def main() -> int:
    with tempfile.TemporaryDirectory() as directory:
        base = Path(directory)
        repo, stub_dir = world(base)
        worktrees = base / "worktrees"

        older = sibling(repo, worktrees, 1, "libolder.rlib")
        time.sleep(1.1)  # mtimes at one-second resolution on some filesystems
        newer = sibling(repo, worktrees, 2, "libnewer.rlib")

        result = claim(repo, base, stub_dir, "4247")
        tree = worktrees / "issue-4247"

        if result.returncode != 0:
            fail("fallthrough", "the claim failed", result)
        elif not (tree / "target" / "debug" / "deps" / "libolder.rlib").is_file():
            fail(
                "fallthrough",
                "did not fall through to the older, working candidate",
                result,
            )
        elif (tree / "target" / "debug" / "deps" / "libnewer.rlib").is_file():
            fail(
                "fallthrough",
                "the newer candidate's failed copy left files behind instead of "
                "being cleaned up before the fallthrough attempt",
                result,
            )
        elif "seeded by copy" not in result.stdout or str(older) not in result.stdout:
            fail(
                "fallthrough",
                "did not report success from the candidate that actually worked",
                result,
            )
        elif "nothing to seed from" in result.stdout:
            fail(
                "fallthrough",
                "a seed that succeeded on its second candidate was reported as none at all",
                result,
            )
        elif not (newer / "target" / "debug" / "deps" / "libnewer.rlib").is_file():
            fail(
                "fallthrough",
                "the failed copy attempt moved the newer sibling's own artifacts "
                "instead of leaving them in place",
                result,
            )

    if FAILURES:
        for failure in FAILURES:
            print(failure, file=sys.stderr)
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-claim seed-fallthrough self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
