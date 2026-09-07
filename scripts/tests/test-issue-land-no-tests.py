#!/usr/bin/env python3
"""A crate with no tests must not fail the landing gate (#1308).

`issue-land.sh` runs the per-crate integration suites for every crate a branch
changed. `postio-bench` has no tests and is not supposed to have any — it is
bench targets, and `bench.yml` compiles them nightly and deliberately times
nothing. nextest exits **4** when a run selects nothing, and the gate read that
as a failure:

    --- test (suites): postio-bench ---
        Starting 0 tests across 1 binary
    error: no tests to run
    issue-land exit 4

"No tests" and "tests failed" are different answers and only one of them
should stop a landing. It went unnoticed because nothing had put that crate in
a diff's changed-crate list until a `criterion` bump did (#1304).

The other half is worse and is why this asserts on both runners: `cargo test`
has never behaved this way, so a machine without nextest passed where the
pinned runner failed. A gate whose answer depends on which runner is installed
is not a gate.

Usage: scripts/tests/test-issue-land-no-tests.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# `scripts/lib` is not a package; the CI step that discovers self-tests runs
# every `scripts/tests/*.py` it finds, so the path goes on `sys.path` here the
# way the neighbouring tests do it.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
REPO_ROOT = HERE.parent
ISSUE_LAND = HERE / "issue-land.sh"
# Sandboxes go under `target/`, which git ignores: inside the worktree because
# the shared-tree guard only lifts its refusals for worktree paths, and not in
# its root because a killed run leaves the sandbox behind and `git add -A` in a
# worktree will commit it. `scripts/checks/check-test-sandboxes.py` says what
# that cost (#1225).
SANDBOXES = REPO_ROOT / "target" / "tmp"
SANDBOXES.mkdir(parents=True, exist_ok=True)

FAILURES: list[str] = []

GH_STUB = """#!/bin/bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then echo "https://example.com/pull/1"; exit 0; fi
if [ "$1" = "pr" ]; then echo "[]"; exit 0; fi
if [ "$1" = "issue" ]; then exit 0; fi
if [ "$1" = "api" ]; then echo "null"; exit 0; fi
exit 0
"""


def git(*args: str, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd, check=True, capture_output=True, text=True,
    )


def world(base: Path, *, with_a_failing_test: bool) -> tuple[Path, Path]:
    """A one-crate workspace whose crate has no tests, or one that fails."""
    root = base / "repo"
    stub_dir = base / "stub"
    (stub_dir / "bin").mkdir(parents=True)
    (root / "scripts" / "checks").mkdir(parents=True)
    (root / "rust-toolchain.toml").write_text(
        '[toolchain]\nchannel = "1.98.0"\n', encoding="utf-8"
    )
    (root / "Cargo.toml").write_text(
        '[workspace]\nresolver = "2"\nmembers = ["crates/dummy"]\n', encoding="utf-8"
    )
    (root / "crates" / "dummy" / "src").mkdir(parents=True)
    (root / "crates" / "dummy" / "Cargo.toml").write_text(
        '[package]\nname = "dummy"\nversion = "0.1.0"\nedition = "2021"\n',
        encoding="utf-8",
    )
    # No `#[test]` anywhere: exactly postio-bench's shape.
    body = "// nothing to test here\n"
    if with_a_failing_test:
        body += '#[test]\nfn it_fails() { panic!("this must stop the landing"); }\n'
    (root / "crates" / "dummy" / "src" / "lib.rs").write_text(body, encoding="utf-8")

    for name in ("check.sh",):
        (root / "scripts" / name).write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
        (root / "scripts" / name).chmod(0o755)
    for name in ("wait-for-checks.sh", "install-shims.sh", "jobserver.sh"):
        (root / "scripts" / name).write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
        (root / "scripts" / name).chmod(0o755)
    # The real one: it is the rule that decides which crates get a suite run,
    # and stubbing it would stub the thing under test out of the picture.
    shutil.copy(HERE / "full-suite-crates.sh", root / "scripts" / "full-suite-crates.sh")
    (root / "scripts" / "full-suite-crates.sh").chmod(0o755)
    shutil.copy(ISSUE_LAND, root / "scripts" / "issue-land.sh")
    (root / "scripts" / "issue-land.sh").chmod(0o755)
    shutil.copytree(HERE / "lib", root / "scripts" / "lib")

    gh = stub_dir / "bin" / "gh"
    gh.write_text(GH_STUB, encoding="utf-8")
    gh.chmod(0o755)

    origin = base / "origin.git"
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
    git("init", "-q", "-b", "main", cwd=root)
    # `issue-land.sh` makes its own commit, without the `-c` overrides the
    # helper above passes, so the sandbox needs an identity of its own.
    git("config", "user.email", "test@example.com", cwd=root)
    git("config", "user.name", "Test", cwd=root)
    (root / ".gitignore").write_text("target/\n", encoding="utf-8")
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "init", cwd=root)
    git("remote", "add", "origin", str(origin), cwd=root)
    git("push", "-q", "origin", "main", cwd=root)
    git("checkout", "-q", "-b", "issue-1308-no-tests", cwd=root)
    return root, stub_dir


def land(root: Path, base: Path, stub_dir: Path, runner: str):
    environment = dict(os.environ)
    environment.pop("RUSTUP_TOOLCHAIN", None)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["CARGO_TARGET_DIR"] = str(base / "target")
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["POSTIO_TEST_RUNNER"] = runner
    # `patience.run`, not `subprocess.run`: a hand-rolled deadline measures
    # the machine it runs on, and this one starts a cargo build on a box that
    # may have three other sessions compiling. `POSTIO_TEST_PATIENCE` is the
    # dial that makes 120s mean 120s of this test's own progress (#842).
    return patience.run(
        ["bash", "scripts/issue-land.sh", "-m", "chore(dummy): touch it", "--no-merge"],
        cwd=root, env=environment, capture_output=True, text=True, timeout=120,
    )


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"  ok   {name}")
    else:
        FAILURES.append(f"{name}: {detail}")
        print(f"  FAIL {name} — {detail}")


def main() -> int:
    for runner in ("nextest", "cargo"):
        with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
            base = Path(directory)
            root, stub_dir = world(base, with_a_failing_test=False)
            (root / "crates" / "dummy" / "src" / "extra.rs").write_text("// x\n", encoding="utf-8")
            result = land(root, base, stub_dir, runner)
            case(
                f"[{runner}] a crate with no tests lands",
                result.returncode == 0,
                f"exit {result.returncode}; a crate with no tests ran all of "
                f"them:\n{result.stdout[-2000:]}\n{result.stderr[-2000:]}",
            )

        # The direction it must still fail in: "no tests" being fine must not
        # make "tests failed" fine too.
        with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
            base = Path(directory)
            root, stub_dir = world(base, with_a_failing_test=True)
            (root / "crates" / "dummy" / "src" / "extra.rs").write_text("// x\n", encoding="utf-8")
            result = land(root, base, stub_dir, runner)
            case(
                f"[{runner}] a failing test still stops the landing",
                result.returncode != 0,
                "a landing whose test panics went through",
            )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-land no-tests self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
