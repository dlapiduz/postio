#!/usr/bin/env python3
"""Self-test for #1190: a failed target seed must not read like no seed at all.

`seed_target` in `issue-claim.sh` finds the newest sibling worktree's
`target/debug` and copies it in. If the copy itself fails -- permissions,
disk full, a sibling deleted mid-copy, a filesystem without reflinks and
without a plain-copy fallback (macOS) -- the code fell back to `rm -rf` and
left `SEEDED` empty, exactly as it would if no candidate had existed at all.
The claim then printed "cold -- nothing to seed from", which is true about
the outcome and false about the cause: there *was* something to seed from,
and copying it did not work. A session reading that line has no reason to
suspect the seed path is broken.

`cp` is stubbed on PATH to fail only for the exact invocation `seed_target`
makes (`-a --reflink=auto`), and delegates everything else to the real `cp`
-- this is the only `cp` call in the script, but failing narrowly costs
nothing and keeps the stub honest about what it is standing in for.

Usage: scripts/tests/test-issue-claim-seed-failure.py
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
        "number": 4246,
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

# Fails only the exact call `seed_target` makes; everything else -- `git
# worktree add`'s own bookkeeping, the test harness setting up fixtures --
# reaches the real `cp` untouched.
CP_STUB = """#!/bin/bash
reflink=0
for arg in "$@"; do
    [ "$arg" = "--reflink=auto" ] && reflink=1
done
if [ "$1" = "-a" ] && [ "$reflink" = "1" ]; then
    echo "cp: simulated failure for the seed-failure test" >&2
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

        candidate = sibling(repo, worktrees, 1, "libnewer.rlib")

        result = claim(repo, base, stub_dir, "4246")
        tree = worktrees / "issue-4246"

        if result.returncode != 0:
            fail("seed-failure", "the claim failed outright", result)
        elif (tree / "target" / "debug").exists():
            fail(
                "seed-failure",
                "target/debug exists after a failed copy -- the fallback rm -rf did not run",
                result,
            )
        elif not (tree / "target" / "tmp").is_dir():
            fail("seed-failure", "target/tmp was not created (#178, #219)", result)
        elif "nothing to seed from" in result.stdout:
            fail(
                "seed-failure",
                "a failed copy was reported the same as no candidate at all -- "
                "the exact bug #1190 is about",
                result,
            )
        elif str(candidate) not in result.stdout:
            fail(
                "seed-failure",
                "the failure message does not name which candidate the copy failed from",
                result,
            )
        elif "copy failed" not in result.stdout and "failed" not in result.stdout.lower():
            fail(
                "seed-failure",
                "nothing in the output says the copy itself failed",
                result,
            )

    if FAILURES:
        for failure in FAILURES:
            print(failure, file=sys.stderr)
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-claim seed-failure self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
