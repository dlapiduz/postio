#!/usr/bin/env python3
"""Self-test for #552: issue-claim.sh must be able to claim from another queue.

Sessions run concurrently on several different machines. The claim locks under
`$POSTIO_CLAIMS` are per-machine and give no cross-machine protection, so the
label on the issue is the only thing keeping two hosts off the same work.

The macOS frontend initiative (#15) is the first body of work that must not be
picked up by an ordinary Linux session -- most of it cannot even be built
there. Its issues are labelled `ready-mac` and deliberately *not* `ready`, so a
plain `issue-claim.sh` skips them for free. This is the other half: a macOS
session needs a way to ask for that queue, without any change to what a plain
claim does.

Cases:
  * default: `ready` is claimed, `ready-mac` is skipped -- unchanged behaviour,
    which is the constraint that actually matters;
  * `--ready-label ready-mac`: the reverse;
  * the skip reason names the label being filtered on, not a hardcoded "ready",
    so a session that passed the wrong queue can tell;
  * `POSTIO_READY_LABEL` in the environment does the same as the flag, and the
    flag wins over it;
  * the never-claimable labels (`epic`, `icebox`, ...) still apply in the other
    queue -- a second queue must not become a way around them.

Usage: scripts/tests/test-issue-claim-ready-label.py
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

SCRIPTS = Path(__file__).resolve().parent.parent
ISSUE_CLAIM = SCRIPTS / "issue-claim.sh"

FIXTURE_ISSUES = [
    {
        "number": 10,
        "title": "an ordinary ready issue",
        "labels": [{"name": "ready"}, {"name": "p1"}],
        "assignees": [],
        "milestone": None,
        "blockedBy": {"nodes": []},
    },
    {
        "number": 11,
        "title": "a macOS issue, higher priority, wrong queue",
        "labels": [{"name": "ready-mac"}, {"name": "p0"}],
        "assignees": [],
        "milestone": None,
        "blockedBy": {"nodes": []},
    },
    {
        "number": 12,
        "title": "a macOS issue that is also an epic",
        "labels": [{"name": "ready-mac"}, {"name": "epic"}, {"name": "p0"}],
        "assignees": [],
        "milestone": None,
        "blockedBy": {"nodes": []},
    },
]

GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
    cat "$STUB_DIR/issues.json"
    exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then echo "stub issue"; exit 0; fi
exit 1
"""

FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def git(*args: str, cwd: Path) -> None:
    subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
    )


def world(base: Path) -> tuple[Path, Path]:
    repo = base / "repo"
    stub_dir = base / "stub"
    (stub_dir / "bin").mkdir(parents=True)
    (repo / "scripts").mkdir(parents=True)
    shutil.copy(ISSUE_CLAIM, repo / "scripts" / "issue-claim.sh")
    (repo / "scripts" / "issue-claim.sh").chmod(0o755)

    gh = stub_dir / "bin" / "gh"
    gh.write_text(GH_STUB, encoding="utf-8")
    gh.chmod(0o755)
    (stub_dir / "issues.json").write_text(json.dumps(FIXTURE_ISSUES), encoding="utf-8")

    git("init", "-q", "-b", "main", cwd=repo)
    (repo / "README.md").write_text("fixture repo\n", encoding="utf-8")
    git("add", "-A", cwd=repo)
    git("commit", "-q", "-m", "init", cwd=repo)
    origin = base / "origin.git"
    subprocess.run(["git", "init", "-q", "--bare", str(origin)], check=True)
    git("remote", "add", "origin", str(origin), cwd=repo)
    git("push", "-q", "origin", "main", cwd=repo)
    return repo, stub_dir


def run_claim(repo: Path, stub_dir: Path, base: Path, *args: str, env_label: str | None = None):
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_WORKTREES"] = str(base / "worktrees")
    environment["POSTIO_CLAIMS"] = str(base / "claims")
    if env_label is not None:
        environment["POSTIO_READY_LABEL"] = env_label
    else:
        environment.pop("POSTIO_READY_LABEL", None)
    return subprocess.run(
        ["bash", str(repo / "scripts" / "issue-claim.sh"), *args],
        cwd=repo,
        env=environment,
        capture_output=True,
        text=True,
        timeout=60,
    )


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)
        repo, stub_dir = world(base)

        # Default queue. #11 is p0 and #10 only p1, so if the label were being
        # ignored the ranking would hand back #11 -- which makes this a real
        # assertion rather than an accident of ordering.
        result = run_claim(repo, stub_dir, base, "--dry-run")
        case(
            "the default queue offers the ready issue",
            "would claim #10" in result.stdout,
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}",
        )
        case(
            "the default queue does not offer a ready-mac issue",
            "would claim #11" not in result.stdout,
            "a macOS issue was offered to an ordinary claim",
        )

        # The other queue.
        result = run_claim(repo, stub_dir, base, "--ready-label", "ready-mac", "--dry-run")
        case(
            "--ready-label offers the macOS issue",
            "would claim #11" in result.stdout,
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}",
        )
        case(
            "--ready-label does not offer the ordinary issue",
            "would claim #10" not in result.stdout,
            "an ordinary issue was offered to a macOS claim",
        )
        case(
            "--ready-label still refuses an epic",
            "would claim #12" not in result.stdout,
            "the second queue bypassed the never-claimable labels",
        )

        # The skip reason has to name the queue, or a session that passed the
        # wrong one reads "not labelled ready" and goes looking for the wrong
        # problem.
        result = run_claim(repo, stub_dir, base, "--ready-label", "nonexistent-queue", "--dry-run")
        out = result.stdout + result.stderr
        case(
            "an empty queue says which label it looked for",
            "nonexistent-queue" in out,
            f"the label is not named in the output:\n{out}",
        )

        # Environment variable, and the flag winning over it.
        result = run_claim(repo, stub_dir, base, "--dry-run", env_label="ready-mac")
        case(
            "POSTIO_READY_LABEL selects the queue",
            "would claim #11" in result.stdout,
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}",
        )
        result = run_claim(
            repo, stub_dir, base, "--ready-label", "ready", "--dry-run", env_label="ready-mac"
        )
        case(
            "the flag beats the environment variable",
            "would claim #10" in result.stdout,
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}",
        )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("issue-claim ready-label check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
