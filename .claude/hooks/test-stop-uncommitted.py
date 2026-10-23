#!/usr/bin/env python3
"""Prove the Stop hook holds a session that leaves its own worktree with
uncommitted work -- once -- and never anyone else's."""
import json
import os
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HOOK = str(Path(__file__).resolve().parent / "stop-uncommitted.py")
MINE, THEIRS = "session-mine", "session-theirs"
failures = 0


def git(cwd: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(cwd), "-c", "user.name=Ada", "-c", "user.email=ada@example.com", *args],
        check=True, capture_output=True, text=True,
        env={**os.environ, "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1"},
    ).stdout.strip()


def claim(tree: Path, session: str, since: float | None = None) -> None:
    record = {"session": session, "at": 0}
    if since is not None:
        record["since"] = since
    Path(git(tree, "rev-parse", "--absolute-git-dir"), "postio-claim").write_text(
        json.dumps(record))


def run(cwd: Path, root: Path, session: str = MINE, active: bool = False):
    payload = {"hook_event_name": "Stop", "session_id": session, "cwd": str(cwd),
               "stop_hook_active": active}
    env = {**os.environ, "POSTIO_WORKTREES": str(root)}
    return subprocess.run([sys.executable, HOOK], input=json.dumps(payload),
                          capture_output=True, text=True, env=env, timeout=30)


def blocked(result) -> bool:
    return result.stdout.strip() != "" and json.loads(result.stdout).get("decision") == "block"


def case(label: str, ok: bool, result) -> None:
    global failures
    print(f"  {'ok  ' if ok else 'FAIL'} {label}")
    if not ok:
        failures += 1
        print(f"    exit {result.returncode} stdout {result.stdout!r} stderr {result.stderr!r}")


with tempfile.TemporaryDirectory() as tmp:
    scratch = Path(tmp)
    shared, root = scratch / "postio", scratch / "postio-worktrees"
    subprocess.run(["git", "init", "-q", "-b", "main", str(shared)], check=True)
    (shared / "a.txt").write_text("a\n")
    git(shared, "add", "a.txt")
    git(shared, "commit", "-qm", "start")
    mine, theirs = root / "issue-1-mine", root / "issue-2-theirs"
    git(shared, "worktree", "add", "-q", "-b", "issue-1-mine", str(mine))
    git(shared, "worktree", "add", "-q", "-b", "issue-2-theirs", str(theirs))
    claim(mine, MINE)
    claim(theirs, THEIRS)

    r = run(shared, root)
    case("clean worktrees let the session stop", r.returncode == 0 and not blocked(r), r)

    (theirs / "a.txt").write_text("their edit\n")
    r = run(shared, root)
    case("another session's loose work is not this session's to commit",
         r.returncode == 0 and not blocked(r), r)

    (mine / "a.txt").write_text("my edit\n")
    r = run(shared, root)
    case("my uncommitted edit holds the stop, naming the tree and the fix",
         blocked(r) and "issue-1-mine" in json.loads(r.stdout)["reason"]
         and "commit" in json.loads(r.stdout)["reason"], r)

    r = run(mine, root)
    case("from inside the worktree too", blocked(r), r)

    r = run(shared, root, active=True)
    case("only once: a second stop is let through", r.returncode == 0 and not blocked(r), r)

    git(mine, "commit", "-qam", "wip")
    (mine / "new.txt").write_text("untracked\n")
    r = run(shared, root)
    case("an untracked file counts as loose work", blocked(r), r)

    (mine / ".gitignore").write_text("new.txt\n.gitignore\n")
    r = run(shared, root)
    case("an ignored file does not", r.returncode == 0 and not blocked(r), r)

    r = run(scratch, root)
    case("outside a repository it stays out of the way", r.returncode == 0 and not blocked(r), r)

print(f"\n{'all passed' if not failures else f'{failures} failed'}")
sys.exit(1 if failures else 0)
