#!/usr/bin/env python3
"""Self-test for scripts/worktree-reap.sh (#1428, #1460).

Fifty-two worktrees once filled a disk, and a landing on it died reporting a
compile error. The reaper's three rules are the whole of what makes it safe
to run on a machine where other sessions' work lives in these trees, so
each is proven in the direction that matters: what is never touched, before
what goes.

Everything happens in a sandbox: a bare origin, a clone of it standing in
for the main checkout, and worktrees under a POSTIO_WORKTREES of its own.
No cargo, no network.

Usage: scripts/tests/test-worktree-reap.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

# The shared dial (#1249). `scripts/lib`, not beside this file, because CI
# runs every `scripts/tests/*.py` it finds as a self-test.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
REAP = HERE / "worktree-reap.sh"
FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    print(f"{'ok   ' if condition else 'FAIL '} {name}")
    if not condition:
        FAILURES.append(f"{name}: {detail}")


def git(*args: str, cwd: Path) -> str:
    return subprocess.run(
        ["git", "-c", "user.email=t@example.com", "-c", "user.name=T", *args],
        cwd=cwd, check=True, capture_output=True, text=True,
    ).stdout.strip()


def gitdir(tree: Path) -> Path:
    return Path(git("rev-parse", "--absolute-git-dir", cwd=tree))


def quiet_for(tree: Path, days: int) -> None:
    """Make `tree` look untouched for `days`: the reaper reads these stamps."""
    stamp = time.time() - days * 86400
    # ORIG_HEAD too: `git worktree add` writes one, and the reaper reads it as
    # a sign of life, since reset, rebase and merge all rewrite it.
    for file in (gitdir(tree) / "index", gitdir(tree) / "HEAD", gitdir(tree) / "ORIG_HEAD", tree / ".git"):
        if file.exists():
            os.utime(file, (stamp, stamp))


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        box = Path(raw)
        origin = box / "origin.git"
        subprocess.run(["git", "init", "-q", "--bare", str(origin)], check=True)
        repo = box / "repo"
        repo.mkdir()
        git("init", "-q", "-b", "main", cwd=repo)
        git("remote", "add", "origin", str(origin), cwd=repo)
        (repo / "scripts").mkdir()
        shutil.copy(REAP, repo / "scripts" / "worktree-reap.sh")
        (repo / ".gitignore").write_text("target/\n", encoding="utf-8")
        (repo / "README.md").write_text("x\n", encoding="utf-8")
        git("add", "-A", cwd=repo)
        git("commit", "-q", "-m", "init", cwd=repo)
        git("push", "-q", "-u", "origin", "main", cwd=repo)

        worktrees = box / "worktrees"
        worktrees.mkdir()
        claims = box / "claims"
        env = dict(os.environ, POSTIO_WORKTREES=str(worktrees), POSTIO_CLAIMS=str(claims))

        def tree(name: str, start: str = "main") -> Path:
            git("worktree", "add", "-q", str(worktrees / name), "-b", name, start, cwd=repo)
            return worktrees / name

        def commit(at: Path, filename: str, message: str) -> str:
            (at / filename).write_text(f"{message}\n", encoding="utf-8")
            git("add", "-A", cwd=at)
            git("commit", "-q", "-m", message, cwd=at)
            return git("rev-parse", "HEAD", cwd=at)

        def with_target(at: Path) -> None:
            (at / "target").mkdir()
            (at / "target" / "junk").write_text("x" * 64, encoding="utf-8")

        # Landed: its commit reached origin/main the way issue-land.sh lands
        # one -- by rebase, so a different sha carrying the same patch.
        landed = tree("issue-1-landed")
        landed_sha = commit(landed, "a.txt", "a")
        git("cherry-pick", "-x", landed_sha, cwd=repo)
        git("push", "-q", "origin", "main", cwd=repo)
        with_target(landed)
        (claims / "issue-1").mkdir(parents=True)
        quiet_for(landed, 3)

        # Unlanded: a commit origin has never seen.
        unlanded = tree("issue-2-unlanded")
        unlanded_sha = commit(unlanded, "b.txt", "b")
        with_target(unlanded)
        quiet_for(unlanded, 3)

        # Dirty: an uncommitted change, on a tree that is otherwise landed.
        dirty = tree("issue-3-dirty")
        (dirty / "README.md").write_text("changed\n", encoding="utf-8")
        with_target(dirty)
        quiet_for(dirty, 3)

        # Fresh: landed and clean, touched today.
        fresh = tree("issue-4-fresh")
        with_target(fresh)

        # An initiative tree: cut from feature/x and landed onto it, which
        # main has never seen. Judged against the base it recorded.
        git("branch", "-q", "feature/x", "main", cwd=repo)
        git("push", "-q", "origin", "feature/x", cwd=repo)
        initiative = tree("issue-5-initiative", "feature/x")
        commit(initiative, "c.txt", "c")
        git("push", "-q", "origin", "HEAD:feature/x", cwd=initiative)
        (gitdir(initiative) / "postio-base").write_text("feature/x\n", encoding="utf-8")
        quiet_for(initiative, 3)

        # A base origin no longer has: nothing can be proven landed.
        orphan = tree("issue-6-orphan")
        (gitdir(orphan) / "postio-base").write_text("feature/gone\n", encoding="utf-8")
        quiet_for(orphan, 3)

        everything = [landed, unlanded, dirty, fresh, initiative, orphan]

        report = patience.run(
            ["bash", "scripts/worktree-reap.sh"],
            cwd=repo, env=env, capture_output=True, text=True, timeout=60,
        )
        out = report.stdout
        case("the report succeeds", report.returncode == 0,
             f"exit {report.returncode}\n{out}\n{report.stderr}")
        case("a landed, clean, quiet tree would be removed whole",
             "remove  issue-1-landed" in out, out)
        case("commits origin has not seen keep the tree and lose only target/",
             "target  issue-2-unlanded" in out and "1 commit(s) not on origin/main" in out, out)
        case("uncommitted changes are never touched",
             "keep    issue-3-dirty: uncommitted changes" in out, out)
        case("a tree touched today is left alone",
             "keep    issue-4-fresh: touched 0d ago" in out, out)
        case("an initiative tree is judged against its own base",
             "remove  issue-5-initiative" in out and "origin/feature/x" in out, out)
        case("a base origin no longer has keeps the tree",
             "keep    issue-6-orphan: cut from 'feature/gone'" in out, out)
        case("the report says how to act on it", "--reap" in out, out)
        case("the report removes nothing",
             all(t.exists() for t in everything) and (landed / "target" / "junk").exists(),
             out)

        again = patience.run(
            ["bash", "scripts/worktree-reap.sh"],
            cwd=repo, env=env, capture_output=True, text=True, timeout=60,
        )
        case("reading a tree does not count as touching it",
             "remove  issue-1-landed" in again.stdout, again.stdout)

        inside = patience.run(
            ["bash", "scripts/worktree-reap.sh"],
            cwd=landed, env=env, capture_output=True, text=True, timeout=60,
        )
        case("the tree it runs from is never a candidate",
             "keep    issue-1-landed: this is where you are" in inside.stdout,
             f"{inside.stdout}\n{inside.stderr}")

        eager = patience.run(
            ["bash", "scripts/worktree-reap.sh", "--days", "0"],
            cwd=repo, env=env, capture_output=True, text=True, timeout=60,
        )
        case("--days 0 counts nothing as too fresh",
             "remove  issue-4-fresh" in eager.stdout, eager.stdout)

        reap = patience.run(
            ["bash", "scripts/worktree-reap.sh", "--reap"],
            cwd=repo, env=env, capture_output=True, text=True, timeout=60,
        )
        out = reap.stdout
        case("--reap succeeds", reap.returncode == 0,
             f"exit {reap.returncode}\n{out}\n{reap.stderr}")
        listed = git("worktree", "list", cwd=repo)
        case("the landed tree is gone, worktree and branch",
             not landed.exists() and "issue-1-landed" not in listed
             and git("branch", "--list", "issue-1-landed", cwd=repo) == "",
             f"{out}\n{listed}")
        case("and its claim lock with it", not (claims / "issue-1").exists(), out)
        case("the unlanded tree keeps its commit",
             unlanded.exists() and git("rev-parse", "HEAD", cwd=unlanded) == unlanded_sha, out)
        case("and loses its target/", not (unlanded / "target").exists(), out)
        case("the dirty tree is untouched",
             (dirty / "README.md").read_text(encoding="utf-8") == "changed\n"
             and (dirty / "target" / "junk").exists(),
             out)
        case("the fresh tree is untouched", (fresh / "target" / "junk").exists(), out)
        case("the initiative tree is gone", not initiative.exists(), out)
        case("the orphan is kept", orphan.exists(), out)
        case("it says what it did", "reaped 3, kept 3" in out, out)

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("worktree-reap self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
