#!/usr/bin/env python3
"""Self-test for scripts/worktree-seed.sh, and for which siblings seed.

`issue-claim.sh` seeds a fresh tree's `target/debug` from the newest
sibling's (#1102), but two kinds of tree never got that:

  * a tree made with plain `git worktree add` -- every spec-branch tree
    (CLAUDE.md's "Spec-driven work" recipe) and every parallel lane. On
    2026-09-30 three Focus lanes were compiling from nothing, 15 to 19
    minutes each, at the exact commit of a warm 24 GB sibling (#1717).
  * any sibling not named `issue-*`. The claim's candidates were
    `issue-*/target/debug` and the shared checkout, so `postio-focus` --
    the warmest tree on the box -- was never a seed for anything.

`worktree-seed.sh <tree>` is the claim's seeding, callable on any tree, and
the candidates are every sibling. Its rules are the claim's: newest first,
never the tree itself, never `target/tmp`, and the sibling's own crates
dropped (they carry its path).

Usage: scripts/tests/test-worktree-seed.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
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
SCRIPT = HERE / "worktree-seed.sh"

FAILURES: list[str] = []
GIT_ENV = {"GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_SYSTEM": "/dev/null"}


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def git(*args: str, cwd: Path) -> None:
    subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True,
                   env={**os.environ, **GIT_ENV})


def world(base: Path) -> tuple[Path, Path]:
    repo = base / "repo"
    (repo / "scripts").mkdir(parents=True)
    git("init", "-q", "-b", "main", cwd=repo)
    git("config", "user.email", "dev@example.com", cwd=repo)
    git("config", "user.name", "Dev", cwd=repo)
    (repo / ".gitignore").write_text("target/\n", encoding="utf-8")
    (repo / "README.md").write_text("fixture\n", encoding="utf-8")
    git("add", "-A", cwd=repo)
    git("commit", "-q", "-m", "init", cwd=repo)
    return repo, base / "worktrees"


def tree(repo: Path, worktrees: Path, name: str, artifact: str | None = None) -> Path:
    path = worktrees / name
    worktrees.mkdir(parents=True, exist_ok=True)
    git("worktree", "add", "-q", "-b", f"branch-{name}", str(path), "main", cwd=repo)
    if artifact:
        deps = path / "target" / "debug" / "deps"
        deps.mkdir(parents=True)
        (deps / artifact).write_text("compiled\n", encoding="utf-8")
        (deps / "libpostio_core-4567.rlib").write_text("ours\n", encoding="utf-8")
        (path / "target" / "tmp" / "scratch").mkdir(parents=True)
    return path


def seed(repo: Path, worktrees: Path, target: Path, env: dict | None = None):
    environment = {**os.environ, **GIT_ENV, "POSTIO_WORKTREES": str(worktrees)}
    environment.pop("POSTIO_CLAIM_SEED", None)
    environment.update(env or {})
    return patience.run(
        ["bash", str(SCRIPT), str(target)],
        cwd=target, env=environment, capture_output=True, text=True, timeout=120,
    )


def main() -> int:
    if not SCRIPT.exists():
        print(f"missing {SCRIPT}", file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory() as directory:
        repo, worktrees = world(Path(directory))
        tree(repo, worktrees, "issue-1", "libolder.rlib")
        time.sleep(1.1)  # mtimes at one-second resolution on some filesystems
        focus = tree(repo, worktrees, "postio-focus", "libwarmest.rlib")
        lane = tree(repo, worktrees, "focus-walk5-input")

        r = seed(repo, worktrees, lane)
        deps = lane / "target" / "debug" / "deps"
        out = r.stdout + r.stderr
        case("a lane made with plain git worktree add can be seeded", r.returncode == 0, out)
        case(
            "the seed is the newest sibling, though it is not an issue-* tree",
            (deps / "libwarmest.rlib").is_file() and not (deps / "libolder.rlib").exists(),
            out + str(sorted(p.name for p in deps.iterdir()) if deps.is_dir() else "no deps"),
        )
        case("it says where the seed came from", str(focus) in out, out)
        case(
            "the sibling's own crates are dropped -- they carry its path",
            not (deps / "libpostio_core-4567.rlib").exists(),
            str(sorted(p.name for p in deps.iterdir()) if deps.is_dir() else "no deps"),
        )
        case("the sibling's live target/tmp is not copied", not (lane / "target" / "tmp" / "scratch").exists(), "")
        case("target/tmp exists for the tree's own tests", (lane / "target" / "tmp").is_dir(), "")
        case(
            "the sibling is untouched",
            (focus / "target" / "debug" / "deps" / "libpostio_core-4567.rlib").is_file(),
            "the drop reached into the sibling",
        )

        # A tree that already has a build keeps it: seeding over live
        # artifacts would throw away the tree's own work.
        r = seed(repo, worktrees, lane)
        case("a tree that already has target/debug is left alone", r.returncode == 0 and "already" in r.stdout, r.stdout + r.stderr)

        # The claim's opt-out holds here too.
        other = tree(repo, worktrees, "spec-branch")
        r = seed(repo, worktrees, other, env={"POSTIO_CLAIM_SEED": "0"})
        case(
            "POSTIO_CLAIM_SEED=0 leaves it cold",
            r.returncode == 0 and not (other / "target" / "debug").exists(),
            r.stdout + r.stderr,
        )

    for failure in FAILURES:
        print(f"FAIL  {failure}")
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.")
        return 1
    print("worktree-seed self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
