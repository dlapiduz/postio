#!/usr/bin/env python3
"""Prove the SessionStart hook fetches, says where the tree stands, and
clears RUSTUP_TOOLCHAIN -- and that it never fails a session.

Every case runs against a sandbox: a bare "origin", a clone standing in for
the shared checkout, a linked worktree standing in for a private one, and a
second clone that lands a commit the others have not seen.
"""
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HOOK = str(Path(__file__).resolve().parent / "session-start.py")
failures = 0


def git(cwd: str, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", cwd, *args], check=True, capture_output=True, text=True,
        env={**os.environ, "GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1"},
    ).stdout.strip()


def commit(cwd: str, name: str) -> None:
    Path(cwd, name).write_text(name + "\n")
    git(cwd, "add", name)
    git(cwd, "-c", "user.name=Ada", "-c", "user.email=ada@example.com", "commit", "-qm", name)


def run(cwd: str, env_file: str | None, **env: str) -> subprocess.CompletedProcess[str]:
    full = {k: v for k, v in os.environ.items() if k != "CLAUDE_ENV_FILE"}
    full.update(GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_NOSYSTEM="1", **env)
    if env_file:
        full["CLAUDE_ENV_FILE"] = env_file
    payload = json.dumps({"hook_event_name": "SessionStart", "source": "startup", "cwd": cwd})
    return subprocess.run(
        [sys.executable, HOOK], input=payload, capture_output=True, text=True, env=full,
        cwd=cwd, timeout=60,
    )


def check(label: str, ok: bool, result: subprocess.CompletedProcess[str]) -> None:
    global failures
    if ok:
        print(f"  ok   {label}")
    else:
        failures += 1
        print(f"  FAIL {label}\n    exit {result.returncode}\n    stdout {result.stdout!r}\n"
              f"    stderr {result.stderr!r}")


with tempfile.TemporaryDirectory() as scratch:
    origin = os.path.join(scratch, "origin.git")
    shared = os.path.join(scratch, "postio")
    lander = os.path.join(scratch, "lander")
    private = os.path.join(scratch, "postio-worktrees", "issue-7-x")
    env_file = os.path.join(scratch, "env.sh")

    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", origin], check=True)
    git(scratch, "clone", "-q", origin, shared)
    commit(shared, "first")
    git(shared, "push", "-q", "origin", "main")
    git(shared, "worktree", "add", "-q", "-b", "issue-7-x", private, "origin/main")
    git(scratch, "clone", "-q", origin, lander)
    commit(lander, "landed-elsewhere")
    git(lander, "push", "-q", "origin", "main")

    # --- the private worktree: told, never moved ---------------------------
    head = git(private, "rev-parse", "HEAD")
    result = run(private, env_file)
    check("a private worktree is told it is behind origin/main",
          result.returncode == 0 and "1 commit behind origin/main" in result.stdout, result)
    check("a private worktree is never moved", git(private, "rev-parse", "HEAD") == head, result)
    check("the toolchain override is cleared for the session",
          Path(env_file).read_text().count("unset RUSTUP_TOOLCHAIN") == 1, result)

    # --- the shared checkout: fast-forwarded when that is safe -------------
    result = run(shared, env_file)
    check("the shared checkout fast-forwards to origin/main",
          result.returncode == 0
          and git(shared, "rev-parse", "HEAD") == git(shared, "rev-parse", "origin/main")
          and "fast-forwarded" in result.stdout, result)
    check("the env file is written once, however many sessions start",
          Path(env_file).read_text().count("unset RUSTUP_TOOLCHAIN") == 1, result)

    # --- the shared checkout with a tracked edit: left alone ---------------
    commit(lander, "landed-again")
    git(lander, "push", "-q", "origin", "main")
    Path(shared, "first").write_text("someone's edit\n")
    head = git(shared, "rev-parse", "HEAD")
    result = run(shared, env_file)
    check("a shared checkout with tracked edits is not moved",
          result.returncode == 0 and git(shared, "rev-parse", "HEAD") == head
          and "1 commit behind origin/main" in result.stdout, result)
    check("the edit survives", Path(shared, "first").read_text() == "someone's edit\n", result)
    Path(shared, "first").write_text("first\n")

    # --- the shared checkout on another branch: left alone -----------------
    git(shared, "checkout", "-q", "-b", "elsewhere")
    head = git(shared, "rev-parse", "HEAD")
    result = run(shared, env_file)
    check("a shared checkout off main is not moved",
          result.returncode == 0 and git(shared, "rev-parse", "HEAD") == head, result)
    git(shared, "checkout", "-q", "main")

    # --- up to date: nothing to say beyond that ----------------------------
    git(shared, "merge", "-q", "--ff-only", "origin/main")
    result = run(shared, env_file)
    check("an up-to-date tree says so",
          result.returncode == 0 and "up to date with origin/main" in result.stdout, result)

    # --- failures never fail the session -----------------------------------
    git(private, "remote", "set-url", "origin", os.path.join(scratch, "gone.git"))
    result = run(private, env_file)
    check("a fetch that fails is reported, not fatal",
          result.returncode == 0 and "fetch failed" in result.stdout, result)
    git(private, "remote", "set-url", "origin", origin)

    result = run(scratch, env_file)
    check("outside a repository it stays quiet",
          result.returncode == 0 and result.stdout.strip() == "", result)

    result = run(private, None)
    check("without CLAUDE_ENV_FILE it still reports", result.returncode == 0
          and "origin/main" in result.stdout, result)

    result = run(private, env_file, POSTIO_SESSION_FETCH="0")
    check("POSTIO_SESSION_FETCH=0 skips the network",
          result.returncode == 0 and "fetch skipped" in result.stdout, result)

print(f"\n{'all passed' if not failures else f'{failures} failed'}")
sys.exit(1 if failures else 0)
