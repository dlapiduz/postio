#!/usr/bin/env python3
"""Self-test for #621: issue-release.sh must strip every queue label, not
just `ready`.

`issue-release.sh <n>`'s post-land path removed `in-progress` and `ready`
unconditionally, so a landed macOS issue (queue label `ready-mac`, #552)
stayed closed but still wearing `ready-mac` -- exactly the "claimable-looking
work on a board that does not check state" #328 already flagged for `ready`
itself.

The fix is one shared list of queue labels (`scripts/lib/ready-labels.sh`),
read by both `issue-claim.sh` (its default queue) and `issue-release.sh`
(every label its post-land path strips) -- so a third queue cannot
desynchronise the two the way `ready-mac` did.

Cases:
  * a `ready-mac` issue released after landing loses `ready-mac` too, not
    only `in-progress`;
  * an ordinary `ready` issue keeps behaving exactly as before;
  * the post-land path's own output names `--abandon` as the other release
    mode, so using the wrong one on unfinished work is easier to notice.

Usage: scripts/tests/test-issue-release-ready-mac.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
ISSUE_RELEASE = HERE / "issue-release.sh"

# Every `gh` invocation is appended to `$STUB_DIR/calls.log`, one line per
# call, so a case can assert on the exact labels a release removed rather
# than only on the script's own exit status.
GH_STUB = """#!/usr/bin/env bash
echo "$@" >> "$STUB_DIR/calls.log"
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "comment" ]; then exit 0; fi
exit 1
"""

FAILURES: list[str] = []


def release(base: Path, repo: Path, stub_dir: Path, num: str, *extra: str):
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_MAIN_CHECKOUT"] = str(repo)
    environment["POSTIO_WORKTREES"] = str(base / "worktrees")
    environment["POSTIO_CLAIMS"] = str(base / "claims")
    return subprocess.run(
        ["bash", str(repo / "scripts" / "issue-release.sh"), num, *extra],
        cwd=repo,
        env=environment,
        capture_output=True,
        text=True,
        timeout=60,
    )


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def calls_log(stub_dir: Path) -> str:
    path = stub_dir / "calls.log"
    return path.read_text(encoding="utf-8") if path.exists() else ""


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)
        repo = base / "repo"
        stub_dir = base / "stub"
        (stub_dir / "bin").mkdir(parents=True)
        (repo / "scripts").mkdir(parents=True)
        shutil.copy(ISSUE_RELEASE, repo / "scripts" / "issue-release.sh")
        (repo / "scripts" / "issue-release.sh").chmod(0o755)
        shutil.copytree(HERE / "lib", repo / "scripts" / "lib")
        gh = stub_dir / "bin" / "gh"
        gh.write_text(GH_STUB, encoding="utf-8")
        gh.chmod(0o755)

        subprocess.run(["git", "init", "-q", "-b", "main", str(repo)], check=True)

        # Case 1: a macOS-queue issue, no worktree and no claim lock left --
        # exactly what a session finds after landing, once it has already
        # run `issue-release.sh` far enough to drop the worktree, or never
        # had one because this call is the whole cleanup.
        (stub_dir / "calls.log").unlink(missing_ok=True)
        result = release(base, repo, stub_dir, "51")
        calls = calls_log(stub_dir)
        case(
            "a landed ready-mac issue has ready-mac stripped",
            "issue edit 51 --remove-label in-progress --remove-label ready-mac"
            in calls
            or "--remove-label ready-mac" in calls,
            f"ready-mac was never removed:\n{calls}",
        )
        case(
            "a landed ready-mac issue still loses in-progress",
            "--remove-label in-progress" in calls,
            f"in-progress was never removed:\n{calls}",
        )
        case(
            "the post-land path also still strips plain ready",
            "--remove-label ready" in calls,
            f"ready was never removed:\n{calls}",
        )
        case(
            "the post-land path names --abandon as the other option",
            "--abandon" in (result.stdout + result.stderr),
            f"no mention of --abandon in the output:\n{result.stdout}{result.stderr}",
        )

        # Case 2: an ordinary queue issue behaves exactly as it always did.
        (stub_dir / "calls.log").unlink(missing_ok=True)
        result = release(base, repo, stub_dir, "52")
        calls = calls_log(stub_dir)
        case(
            "an ordinary ready issue still loses ready",
            "--remove-label ready" in calls,
            f"ready was never removed:\n{calls}",
        )
        case(
            "a plain release still reports success",
            result.returncode == 0 and "#52 cleaned up." in result.stdout,
            f"exit {result.returncode}; stdout:\n{result.stdout}\n{result.stderr}",
        )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("issue-release ready-label check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
