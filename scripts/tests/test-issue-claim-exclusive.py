#!/usr/bin/env python3
"""Self-test for #1218: one issue, one session.

Two sessions worked #1177 at the same time on 2026-09-06 and implemented it
twice on branches of the same name, and an hour later the same thing happened
to #1142 — where the session that noticed and backed out took the *other*
session's claim with it. Both went through `issue-claim.sh <n>`, the path
that names an issue rather than taking the top of the queue.

Three things have to hold, and only the last of them did:

  * **Naming an issue does not bypass its claim.** The queue path skips an
    issue with an assignee or `in-progress`; the explicit path skipped every
    filter, including those two. A rider claim (`gh issue edit --add-assignee`,
    which CLAUDE.md sanctions and which takes no lock) was therefore invisible
    to it.
  * **`--resume` takes the lock or refuses.** It used `mkdir ... || true`, so
    a resume walked straight through another session's live claim.
  * **An older copy of the scripts can still drop a lock.** The first fix put
    the owner *inside* the lock directory, where `rmdir` refuses it — and
    every worktree carries its own copy of these scripts at whatever commit
    it was cut from, so a release run from a checkout that predates the
    change left an undroppable lock and an unclaimable issue (#1230).
  * **A claim still bypasses the *label* filters when named.** `epic`,
    `needs-architecture` and the rest are "not agent work by default", not
    "never claimable" — the architect claims a `needs-architecture` issue to
    write its ADR. That distinction is the whole reason the explicit path
    exists, so the fix must not close it.

Usage: scripts/tests/test-issue-claim-exclusive.py
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

# 4242 is free. 4243 is somebody's rider claim: assigned and `in-progress`,
# with no worktree and no claim lock, which is exactly what the rider path in
# CLAUDE.md produces. 4244 is unassigned but `needs-architecture` — not queue
# work, still claimable by name.
FIXTURE_ISSUES = [
    {
        "number": 4242,
        "title": "An issue nobody has taken",
        "labels": [{"name": "ready"}, {"name": "p2"}],
        "assignees": [],
        "milestone": None,
        "blockedBy": {"nodes": []},
    },
    {
        "number": 4243,
        "title": "An issue another session is working",
        "labels": [{"name": "ready"}, {"name": "p2"}, {"name": "in-progress"}],
        "assignees": [{"login": "dlapiduz"}],
        "milestone": None,
        "blockedBy": {"nodes": []},
    },
    {
        "number": 4244,
        "title": "An issue waiting on a design decision",
        "labels": [{"name": "needs-architecture"}, {"name": "p2"}],
        "assignees": [],
        "milestone": None,
        "blockedBy": {"nodes": []},
    },
]

GH_STUB = """#!/bin/bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then cat "$STUB_DIR/issues.json"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then echo "OPEN ready,p2"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then echo "[]"; exit 0; fi
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


def claim(repo: Path, base: Path, stub_dir: Path, *args: str):
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_WORKTREES"] = str(base / "worktrees")
    environment["POSTIO_CLAIMS"] = str(base / "claims")
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["POSTIO_CLAIM_SEED"] = "0"
    return subprocess.run(
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
        claims = base / "claims"

        # ── naming an issue does not walk through somebody's claim ────────
        # #4243 is assigned and `in-progress` with no lock: a rider claim, or
        # a session on another machine. The queue path already skips it.
        result = claim(repo, base, stub_dir, "4243")
        if result.returncode == 0:
            fail("claimed", "took an issue another session is working", result)
        elif (worktrees / "issue-4243").exists():
            fail("claimed", "created a worktree for it anyway", result)
        elif "4243" not in (result.stdout + result.stderr):
            fail("claimed", "refused without naming the issue", result)

        # ── ...and does not quietly release it either ─────────────────────
        # The refusal above is what stops the damage that followed on #1142:
        # a second claim, then a release that stripped the first session's
        # labels. Nothing here should have touched the lock directory.
        if (claims / "issue-4243").exists():
            FAILURES.append(
                "claimed: a refused claim left a lock behind, which would make "
                "the issue unclaimable until the next stale sweep"
            )

        # ── a named claim still bypasses the label filters ────────────────
        # `needs-architecture` is not queue work and is claimable by name:
        # that is how the architect takes one to write its ADR.
        result = claim(repo, base, stub_dir, "4244")
        if result.returncode != 0:
            fail("by-name", "refused a needs-architecture issue named explicitly", result)
        elif not (worktrees / "issue-4244").is_dir():
            fail("by-name", "took it without giving it a tree", result)

        # ── the lock says who holds it ────────────────────────────────────
        owner = claims / "issue-4244.owner"
        if not owner.is_file():
            FAILURES.append(
                "owner: the claim lock records nothing about who took it, so a "
                "refusal cannot say which tree is holding the issue"
            )
        elif str(worktrees / "issue-4244") not in owner.read_text(encoding="utf-8"):
            FAILURES.append(
                f"owner: the lock names {owner.read_text(encoding='utf-8')!r}, "
                f"not the worktree it created"
            )

        # ── ...beside the lock, not inside it ─────────────────────────────
        # Every worktree carries its own copy of these scripts at the commit
        # it was cut from, and the main checkout is pulled when somebody
        # remembers, so a lock's format is something several versions have to
        # agree on at once. `rmdir` is what every older copy drops a lock
        # with, and it refuses a directory with a file in it — which left
        # #1216 unclaimable within an hour of #1218 landing.
        lock = claims / "issue-4244"
        try:
            lock.rmdir()
        except OSError as error:
            FAILURES.append(
                f"format: a script that has never heard of the owner file "
                f"cannot drop this lock ({error}), so the issue stays claimed "
                f"by a session that has gone"
            )
        else:
            lock.mkdir()

        # ── --resume takes the lock, or refuses ───────────────────────────
        # A branch on origin for 4242, and its lock already held by another
        # session on this machine. `mkdir ... || true` walked through this.
        git("checkout", "-q", "-b", "issue-4242-red-pr", cwd=repo)
        (repo / "work.txt").write_text("landed on the branch\n", encoding="utf-8")
        git("add", "-A", cwd=repo)
        git("commit", "-q", "-m", "feat: the work", cwd=repo)
        git("push", "-q", "-u", "origin", "issue-4242-red-pr", cwd=repo)
        git("checkout", "-q", "main", cwd=repo)
        git("branch", "-q", "-D", "issue-4242-red-pr", cwd=repo)
        held = claims / "issue-4242"
        held.mkdir(parents=True)
        holder = worktrees / "somebody-elses-tree"
        # The holder's tree exists, because that is what a live claim looks
        # like: `--stale` and this both read a worktree as the evidence a
        # session is still there. A lock whose tree is gone is the ordinary
        # leftover of a session that landed and moved on, and resuming over
        # that one is the flow `--resume` exists for.
        holder.mkdir(parents=True)
        held_owner = claims / "issue-4242.owner"
        held_owner.write_text(str(holder) + "\n", encoding="utf-8")

        result = claim(repo, base, stub_dir, "--resume", "4242")
        if result.returncode == 0:
            fail("resume", "resumed an issue another session holds the lock on", result)
        elif (worktrees / "issue-4242").exists():
            fail("resume", "created a worktree for it anyway", result)
        elif "somebody-elses-tree" not in (result.stdout + result.stderr):
            fail("resume", "refused without naming who is holding it", result)
        elif not held_owner.is_file():
            fail("resume", "a refused resume removed the holder's lock", result)

        # ── ...but a lock the holder has left behind is not a claim ───────
        # The ordinary loop lands, claims the next issue into the same tree,
        # and leaves this issue's lock where it was. Refusing on the lock
        # alone would break coming back to a branch whose PR went red, which
        # is the whole of what `--resume` is for.
        shutil.rmtree(holder)
        result = claim(repo, base, stub_dir, "--resume", "4242")
        if result.returncode != 0:
            fail("resume-leftover", "refused a lock whose session is gone", result)
        elif not (worktrees / "issue-4242").is_dir():
            fail("resume-leftover", "took it without giving it a tree", result)
        elif str(worktrees / "issue-4242") not in held_owner.read_text(encoding="utf-8"):
            fail("resume-leftover", "took the lock without recording the new owner", result)

    if FAILURES:
        for failure in FAILURES:
            print(failure, file=sys.stderr)
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-claim exclusivity self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
