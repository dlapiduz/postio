#!/usr/bin/env python3
"""Self-test for #1428 and #1460: a full disk is reported as a full disk.

Two landings died on a disk at 100% and reported a compile error and a
SIGBUS -- both true, neither the cause, and the tell (`os error 28`) was
several lines up in a log that mostly scrolls past. So `issue-land.sh`
refuses below a floor of free space before a gate can run out of it, and
`--status` and the detached child's own log name the disk when a run has
already met the failure.

No cargo: the floor is driven through POSTIO_LAND_DISK_FLOOR_GB, and the
diagnosis reads logs this test writes.

Usage: scripts/tests/test-issue-land-disk-full.py
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
ISSUE_LAND = HERE / "issue-land.sh"
FAILURES: list[str] = []

COMPILE_ERROR = """   Compiling postio-app v0.3.0
error: failed to write query cache to .../target/debug/incremental/postio_app-x/query-cache.bin:
  No space left on device (os error 28)
error: could not compile `postio-app` (lib test)
issue-land exit 101
"""

SIGBUS = """     Running tests/app_suite/main.rs
error: test failed, to rerun pass `-p postio-app --test app_suite`
Caused by:
  process didn't exit successfully: .../deps/postio_app-4db7c645e7e843fa (signal: 7, SIGBUS: access to undefined memory)
issue-land exit 101
"""

PLAIN_FAILURE = """   Compiling postio-app v0.3.0
error[E0425]: cannot find value `x` in this scope
error: could not compile `postio-app` (lib test)
issue-land exit 101
"""


def case(name: str, condition: bool, detail: str) -> None:
    print(f"{'ok   ' if condition else 'FAIL '} {name}")
    if not condition:
        FAILURES.append(f"{name}: {detail}")


def git(*args: str, cwd: Path) -> str:
    return subprocess.run(
        ["git", "-c", "user.email=t@example.com", "-c", "user.name=T", *args],
        cwd=cwd, check=True, capture_output=True, text=True,
    ).stdout.strip()


def status(repo: Path) -> subprocess.CompletedProcess:
    return patience.run(
        ["bash", "scripts/issue-land.sh", "--status"],
        cwd=repo, capture_output=True, text=True, timeout=30,
    )


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        repo = Path(raw) / "repo"
        (repo / "scripts").mkdir(parents=True)
        shutil.copy(ISSUE_LAND, repo / "scripts" / "issue-land.sh")
        git("init", "-q", "-b", "main", cwd=repo)
        (repo / "README.md").write_text("x\n", encoding="utf-8")
        git("add", "-A", cwd=repo)
        git("commit", "-q", "-m", "init", cwd=repo)
        git("checkout", "-q", "-b", "not-an-issue-branch", cwd=repo)
        git_dir = Path(git("rev-parse", "--absolute-git-dir", cwd=repo))
        log = git_dir / "postio-land.log"

        # Below the floor, nothing starts. The floor is set past any disk
        # this could run on, so the refusal is the only thing that can
        # happen -- before the branch check, which is what a sandbox branch
        # would otherwise hit first.
        starved = patience.run(
            ["bash", "scripts/issue-land.sh", "--gates-only"],
            cwd=repo, capture_output=True, text=True, timeout=30,
            env=dict(os.environ, POSTIO_LAND_DISK_FLOOR_GB="9999999"),
        )
        case("below the floor it refuses", starved.returncode == 2,
             f"exit {starved.returncode}\n{starved.stdout}\n{starved.stderr}")
        case("and says the disk is why", "Refusing to land with" in starved.stderr
             and "GB free" in starved.stderr, starved.stderr)
        case("and names the reaper", "worktree-reap.sh" in starved.stderr, starved.stderr)
        case("not the branch", "not an issue branch" not in starved.stderr, starved.stderr)

        # Above the floor, the ordinary refusal for this sandbox branch --
        # so the floor is a floor, not a wall.
        fed = patience.run(
            ["bash", "scripts/issue-land.sh", "--gates-only"],
            cwd=repo, capture_output=True, text=True, timeout=30,
            env=dict(os.environ, POSTIO_LAND_DISK_FLOOR_GB="0"),
        )
        case("above the floor the run goes on to its usual checks",
             "not an issue branch" in fed.stderr and "Refusing to land with" not in fed.stderr,
             f"{fed.stdout}\n{fed.stderr}")

        # A run that met the failure: --status names the disk.
        for name, text in (("a compile error on a full disk", COMPILE_ERROR),
                           ("a SIGBUS on a full disk", SIGBUS)):
            log.write_text(text, encoding="utf-8")
            result = status(repo)
            case(f"--status says {name} is the disk",
                 result.returncode == 0 and "disk was full" in result.stdout,
                 f"exit {result.returncode}\n{result.stdout}\n{result.stderr}")
            case("and says what is free now", "free now:" in result.stdout, result.stdout)

        log.write_text(PLAIN_FAILURE, encoding="utf-8")
        result = status(repo)
        case("a failure that is not the disk is not blamed on it",
             result.returncode == 0 and "disk" not in result.stdout,
             f"exit {result.returncode}\n{result.stdout}")

        # The detached child writes the same conclusion at the end of its own
        # log, where whoever tails the log will see it without --status.
        log.unlink()
        detached = patience.run(
            ["bash", "scripts/issue-land.sh", "--detach", "--gates-only"],
            cwd=repo, capture_output=True, text=True, timeout=60,
            env=dict(os.environ, POSTIO_LAND_DISK_FLOOR_GB="9999999"),
        )
        case("--detach still returns at once", detached.returncode == 0,
             f"exit {detached.returncode}\n{detached.stdout}\n{detached.stderr}")
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if log.exists() and "issue-land exit" in log.read_text(encoding="utf-8"):
                break
            time.sleep(0.2)
        text = log.read_text(encoding="utf-8") if log.exists() else ""
        case("the child refused below the floor", "Refusing to land with" in text
             and "issue-land exit 2" in text, text)
        case("a refusal is not called a full disk", "disk full:" not in text, text)

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("issue-land disk-full self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
