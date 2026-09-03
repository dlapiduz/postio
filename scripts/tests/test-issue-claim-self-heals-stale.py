#!/usr/bin/env python3
"""Self-test for #924: issue-claim.sh must self-heal a stale claim before
reporting "nothing ready".

A session that dies mid-work (crashes, runs out of context, gets
interrupted) leaves its claim lock and its GitHub assignee/`in-progress`
label exactly where they were -- nothing else ever runs
`issue-release.sh --stale`, so the ready queue only shrinks. This happened
for real: four issues sat unclaimable for over a day before a session
noticed by accident.

Cases:
  * a dead claim (lock exists, no matching worktree, issue still carries
    `in-progress` and an assignee GitHub-side) is swept and the issue
    becomes claimable in the same `issue-claim.sh` invocation that first
    found nothing;
  * a lock whose worktree still exists is never touched by the sweep, and
    the issue stays reported as "already claimed" -- the self-heal must not
    let a live session's work be reclaimed out from under it;
  * `--dry-run` never triggers the sweep at all: a dry run's contract
    elsewhere in this script is that it previews and does not act, and the
    sweep is a real GitHub mutation.

Usage: scripts/tests/test-issue-claim-self-heals-stale.py
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
        "number": 50,
        "title": "claimed by a session that died before releasing it",
        "labels": [{"name": "ready"}, {"name": "p2"}, {"name": "in-progress"}],
        "assignees": [{"login": "dead-session"}],
        "milestone": None,
        "blockedBy": {"nodes": []},
    }
]

# `gh issue edit ... --remove-assignee ... --remove-label in-progress` is
# what `issue-release.sh --stale` runs to actually release an abandoned
# claim. The stub mutates the same fixture file `gh issue list` reads, so a
# `fetch_candidates` retry after the sweep sees the issue as GitHub would --
# unassigned and unlabelled -- the way the real self-heal depends on.
GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "list" ]; then
    cat "$STUB_DIR/issues.json"
    exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then
    # Still `in-progress` GitHub-side: the orphaned-lock branch of --stale
    # must NOT fire (that one is for a label that went missing while a
    # session worked); this has to reach the age-gated abandoned-claim
    # branch instead, which is the one #924 is about.
    echo "OPEN ready,p2,in-progress"
    exit 0
fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then
    if printf '%s\\n' "$*" | grep -q -- "--remove-assignee"; then
        python3 -c '
import json, os
path = os.environ["STUB_DIR"] + "/issues.json"
data = json.load(open(path))
for issue in data:
    issue["assignees"] = []
    issue["labels"] = [l for l in issue["labels"] if l["name"] != "in-progress"]
json.dump(data, open(path, "w"))
'
    fi
    exit 0
fi
if [ "$1" = "api" ]; then echo "null"; exit 0; fi
exit 1
"""

FAILURES: list[str] = []


def git(*args: str, cwd: Path) -> None:
    subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
    )


def world(base: Path) -> tuple[Path, Path]:
    """A fixture repo with a local bare origin, a stubbed gh, and both
    scripts (issue-claim.sh calls issue-release.sh by relative path, so it
    has to actually be there)."""
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
    git("add", "-A", cwd=repo)
    git("commit", "-q", "-m", "init", cwd=repo)
    origin = base / "origin.git"
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
    git("remote", "add", "origin", str(origin), cwd=repo)
    git("push", "-q", "origin", "main", cwd=repo)
    return repo, stub_dir


def run_claim(repo: Path, stub_dir: Path, base: Path, *args: str):
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["POSTIO_MAIN_CHECKOUT"] = str(repo)
    environment["POSTIO_WORKTREES"] = str(base / "worktrees")
    environment["POSTIO_CLAIMS"] = str(base / "claims")
    return subprocess.run(
        ["bash", str(repo / "scripts" / "issue-claim.sh"), *args],
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


def reset_fixture(stub_dir: Path) -> None:
    (stub_dir / "issues.json").write_text(json.dumps(FIXTURE_ISSUES), encoding="utf-8")


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)

        # ── case 1: a dead claim self-heals ──────────────────────────────
        repo, stub_dir = world(base)
        dead_lock = base / "claims" / "issue-50"
        dead_lock.mkdir(parents=True)

        result = run_claim(repo, stub_dir, base)
        out = result.stdout + result.stderr
        case(
            "a dead claim is swept and the issue is claimed in the same run",
            result.returncode == 0 and "claimed #50" in result.stdout,
            f"exit {result.returncode}:\n{out}",
        )
        case(
            "the sweep's own report reaches the caller",
            "released abandoned claim on #50" in out,
            f"the self-heal ran silently, or not at all:\n{out}",
        )
        case(
            "the issue got a real worktree of its own",
            (base / "worktrees" / "issue-50").is_dir(),
            f"no worktree was created:\n{out}",
        )

    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)

        # ── case 2: a live worktree is never touched ─────────────────────
        repo, stub_dir = world(base)
        live_lock = base / "claims" / "issue-50"
        live_lock.mkdir(parents=True)
        live_tree = base / "worktrees" / "issue-50"
        live_tree.mkdir(parents=True)
        (live_tree / "half-done.rs").write_text("// mid-work\n", encoding="utf-8")

        result = run_claim(repo, stub_dir, base)
        out = result.stdout + result.stderr
        case(
            "a live worktree's claim is left reported as already claimed",
            result.returncode != 0
            and "No `ready`" in result.stdout
            and "claimed #50" not in result.stdout,
            f"exit {result.returncode}:\n{out}",
        )
        case(
            "the live lock survives",
            live_lock.exists(),
            "the sweep cleared a claim whose worktree exists",
        )
        case(
            "the live worktree's file survives untouched",
            (live_tree / "half-done.rs").exists(),
            "the pre-existing worktree was modified",
        )

    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)

        # ── case 3: --dry-run never mutates anything ─────────────────────
        repo, stub_dir = world(base)
        dead_lock = base / "claims" / "issue-50"
        dead_lock.mkdir(parents=True)

        result = run_claim(repo, stub_dir, base, "--dry-run")
        out = result.stdout + result.stderr
        case(
            "--dry-run reports nothing ready without sweeping",
            result.returncode != 0 and "No `ready`" in result.stdout,
            f"exit {result.returncode}:\n{out}",
        )
        case(
            "--dry-run never calls the sweep at all",
            "released abandoned claim" not in out
            and "cleared orphaned lock" not in out,
            f"a dry run mutated GitHub-side state:\n{out}",
        )
        case(
            "the lock is untouched by a dry run",
            dead_lock.exists(),
            "a dry run removed a claim lock",
        )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("issue-claim self-heal check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
