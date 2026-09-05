#!/usr/bin/env python3
"""Self-test for #1129: `issue-land.sh --detach` hands the landing to a
process the calling tool cannot kill, and `--status` reads its log.

Two backgrounded landings were killed at a tool call's ten-minute cap in one
day, and 79 tool timeouts in the transcript history have the same shape. A
killed run re-pays the push and the CI wait; before #742 it re-paid the
gates too. Detaching under `setsid` (or `nohup` where there is no setsid)
takes the run out of the tool's process group entirely.

Proven without cargo: the sandbox branch is not an issue branch, so the
detached child refuses in its first second -- which is enough to see that
the parent returned at once, the child ran on its own, the log holds the
child's output and exit status, and `--status` reports them.

Usage: scripts/tests/test-issue-land-detach.py
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

HERE = Path(__file__).resolve().parent.parent
ISSUE_LAND = HERE / "issue-land.sh"
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

        started = time.monotonic()
        result = subprocess.run(
            ["bash", "scripts/issue-land.sh", "--detach", "--gates-only"],
            cwd=repo, capture_output=True, text=True, timeout=60,
        )
        elapsed = time.monotonic() - started
        case("--detach returns at once", result.returncode == 0 and elapsed < 10,
             f"exit {result.returncode} after {elapsed:.1f}s\n{result.stdout}\n{result.stderr}")
        log = git_dir / "postio-land.log"
        case("it says where the log is", str(log) in result.stdout,
             f"stdout does not name {log}:\n{result.stdout}")
        case("it says how to check", "--status" in result.stdout, result.stdout)

        # The child is on its own: wait for it to finish rather than for us.
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if log.exists() and "issue-land exit" in log.read_text(encoding="utf-8"):
                break
            time.sleep(0.2)
        text = log.read_text(encoding="utf-8") if log.exists() else ""
        case("the child ran and logged", "not an issue branch" in text,
             f"log:\n{text}")
        case("the log ends with the child's exit status", "issue-land exit 2" in text,
             f"log:\n{text}")

        status = subprocess.run(
            ["bash", "scripts/issue-land.sh", "--status"],
            cwd=repo, capture_output=True, text=True, timeout=30,
        )
        case("--status succeeds", status.returncode == 0,
             f"exit {status.returncode}\n{status.stdout}\n{status.stderr}")
        case("--status reports the outcome", "exit 2" in status.stdout and "finished" in status.stdout,
             status.stdout)

        # Nothing to report is an answer, not an error.
        (log).unlink()
        status = subprocess.run(
            ["bash", "scripts/issue-land.sh", "--status"],
            cwd=repo, capture_output=True, text=True, timeout=30,
        )
        case("--status with no log says so", status.returncode == 0 and "no landing" in status.stdout,
             f"exit {status.returncode}\n{status.stdout}")

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("issue-land --detach self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
