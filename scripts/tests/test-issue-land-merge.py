#!/usr/bin/env python3
"""Self-test for issue #167: issue-land.sh's merge step and the local worktree.

`gh pr merge --rebase --delete-branch` deletes the *local* branch too, which
makes `gh` switch the current worktree off it first -- and in this
repository's layout `main` (the PR's base) is permanently checked out in the
shared checkout, so git refuses the checkout and `gh` reports failure even
though the merge already went through on GitHub. Confirmed on #159 and #163.

The fix drops `--delete-branch` and has the script delete the *remote*
branch itself afterward, since `issue-release.sh` already deletes the local
one once the worktree it belongs to is removed.

`gh` is stubbed on PATH and logs every call it is given: the stub fails
loudly, with the real bug's own error text, if it is ever asked to
`--delete-branch` -- which is what makes this a regression test rather than
a demonstration. `git` is real throughout, including the final
`push origin --delete`, so the branch's actual removal from a real (bare,
local) remote is what this checks, not a stubbed opinion about it.

Usage: scripts/tests/test-issue-land-merge.py
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
REPO_ROOT = HERE.parent
ISSUE_LAND = HERE / "issue-land.sh"
WAIT_FOR_CHECKS = HERE / "wait-for-checks.sh"
CI_EXPECTED_WORKFLOWS = HERE / "checks" / "ci-expected-workflows.py"

STUB_CHECKS = [
    "check-crate-boundaries.py",
    "check-no-personal-data.py",
    "check-no-silent-tracking.py",
    "check-toolchain-pinned.py",
    "check-no-gtk-init-in-unit-tests.py",
    "check-runtime-crossings.py",
]

FAILURES: list[str] = []

# The real bug's own message, so a regression reads exactly like the incident
# it would be.
DELETE_BRANCH_ERROR = (
    "failed to run git: fatal: 'main' is already used by worktree at "
    "'/home/user/src/postio'"
)

# Every call is appended to $STUB_DIR/calls, one line each, so the test can
# assert on what was actually asked for -- not just on the exit code.
GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
printf '%s\\n' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    if printf '%s' "$*" | grep -q -- "--json number"; then
        exit 1
    fi
    if printf '%s' "$*" | grep -q -- "--json url"; then
        echo "https://example.com/pull/1"
        exit 0
    fi
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "checks" ]; then
    if printf '%s' "$*" | grep -q -- "--json name,bucket"; then
        echo "[]"
        exit 0
    fi
    if printf '%s' "$*" | grep -q -- "--json name"; then
        echo "[]"
        exit 0
    fi
    exit 1
fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
    for arg in "$@"; do
        case "$arg" in
            --delete-branch|-d)
                echo "STUB_DELETE_BRANCH_ERROR" >&2
                exit 1
                ;;
        esac
    done
    # Actually move the base, as a real rebase-merge does. #312 taught this
    # stub to stop lying: the script now verifies that the work reached
    # `main` before reporting success, so a stub that says "Merged" without
    # merging fails the very check that exists to catch a merge which did
    # not happen.
    git push -q "$ORIGIN" "HEAD:refs/heads/main" || exit 1
    echo "Merged"
    exit 0
fi
exit 0
"""

# A workflow with no `pull_request` trigger at all, so ci-expected-workflows.py
# predicts nothing is scheduled and wait-for-checks.sh takes its short GRACE
# path rather than the long REGISTER_TIMEOUT one. A fixture of our own for
# the reason test-wait-for-checks.py's own FIXTURE_CI_YML gives: this must
# not start testing the wrong branch of the logic the day ci.yml's own
# triggers change for operational reasons.
FIXTURE_CI_YML = "name: CI\non:\n  workflow_dispatch:\n"


def pinned_channel() -> str:
    text = (REPO_ROOT / "rust-toolchain.toml").read_text(encoding="utf-8")
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("channel"):
            return line.split("=", 1)[1].strip().strip('"')
    raise RuntimeError("rust-toolchain.toml names no channel")


def build_sandbox(root: Path, channel: str) -> None:
    (root / "rust-toolchain.toml").write_text(
        f'[toolchain]\nchannel = "{channel}"\nprofile = "minimal"\n',
        encoding="utf-8",
    )
    (root / "Cargo.toml").write_text(
        '[workspace]\nmembers = ["dummy"]\nresolver = "2"\n', encoding="utf-8"
    )
    dummy = root / "dummy" / "src"
    dummy.mkdir(parents=True)
    (root / "dummy" / "Cargo.toml").write_text(
        '[package]\nname = "dummy"\nversion = "0.1.0"\nedition = "2021"\n',
        encoding="utf-8",
    )
    (dummy / "lib.rs").write_text("pub fn x() {}\n", encoding="utf-8")

    workflows = root / ".github" / "workflows"
    workflows.mkdir(parents=True)
    (workflows / "ci.yml").write_text(FIXTURE_CI_YML, encoding="utf-8")

    scripts = root / "scripts"
    scripts.mkdir()
    (scripts / "checks").mkdir()
    shutil.copy(HERE / "check.sh", scripts / "check.sh")
    (scripts / "check.sh").chmod(0o755)
    shutil.copytree(HERE / "lib", scripts / "lib")
    for source in (ISSUE_LAND, WAIT_FOR_CHECKS, CI_EXPECTED_WORKFLOWS):
        into = scripts / "checks" if source.parent.name == "checks" else scripts
        shutil.copy(source, into / source.name)
        (into / source.name).chmod(0o755)
    for name in STUB_CHECKS:
        (scripts / "checks" / name).write_text(
            "#!/usr/bin/env python3\nraise SystemExit(0)\n", encoding="utf-8"
        )
        (scripts / "checks" / name).chmod(0o755)


def git(*args: str, cwd: Path) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
    )


def land(
    root: Path, target: Path, stub_dir: Path, origin: Path
) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment.pop("RUSTUP_TOOLCHAIN", None)
    environment["CARGO_TARGET_DIR"] = str(target)
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    # The bare remote the `pr merge` stub pushes into; see GH_STUB.
    environment["ORIGIN"] = str(origin)
    # A real merge, watched briefly: the fixture schedules no workflow, so
    # this settles in POSTIO_CHECKS_GRACE seconds rather than the real
    # default's 30.
    environment["POSTIO_CHECKS_GRACE"] = "1"
    environment["POSTIO_CHECKS_POLL"] = "1"
    return subprocess.run(
        ["bash", "scripts/issue-land.sh", "-m", "feat(dummy): add a file"],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
        timeout=60,
    )


def main() -> int:
    if shutil.which("cargo") is None:
        print("skip: no cargo on PATH", file=sys.stderr)
        return 0

    channel = pinned_channel()

    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        base = Path(directory)
        target = base / "target"
        root = base / "repo"
        origin = base / "origin.git"
        stub_dir = base / "stub"
        (stub_dir / "bin").mkdir(parents=True)
        gh = stub_dir / "bin" / "gh"
        gh.write_text(GH_STUB, encoding="utf-8")
        gh.chmod(0o755)
        (stub_dir / "calls").write_text("", encoding="utf-8")

        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
        root.mkdir()
        build_sandbox(root, channel)
        git("init", "-q", "-b", "main", cwd=root)
        git("config", "user.email", "test@example.com", cwd=root)
        git("config", "user.name", "Test", cwd=root)
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "init", cwd=root)
        git("remote", "add", "origin", str(origin), cwd=root)
        git("push", "-q", "origin", "main", cwd=root)
        git("checkout", "-q", "-b", "issue-1-x", cwd=root)
        (root / "dummy" / "src" / "extra.rs").write_text("// nothing\n", encoding="utf-8")

        result = land(root, target, stub_dir, origin)
        calls = (stub_dir / "calls").read_text(encoding="utf-8")

        if result.returncode != 0:
            FAILURES.append(
                "the merge step failed on a fix that should have avoided the "
                f"worktree conflict entirely:\n--- stdout ---\n{result.stdout}\n"
                f"--- stderr ---\n{result.stderr}\n--- gh calls ---\n{calls}"
            )
        elif DELETE_BRANCH_ERROR in result.stderr or "STUB_DELETE_BRANCH_ERROR" in result.stderr:
            FAILURES.append(
                f"the merge step still hit the worktree-conflict error:\n{result.stderr}"
            )

        if "pr merge --rebase --delete-branch" in calls or " -d" in calls:
            FAILURES.append(
                f"gh pr merge was called with --delete-branch, which reintroduces "
                f"#167:\n{calls}"
            )

        remote_branches = git(
            "ls-remote", "--heads", "origin", cwd=root
        ).stdout.decode()
        if "issue-1-x" in remote_branches:
            FAILURES.append(
                "the remote branch was not deleted after a successful merge: "
                f"{remote_branches!r}"
            )

        if "merged." not in result.stdout:
            FAILURES.append(f"the script did not say it merged: {result.stdout!r}")
        if "remote branch deleted." not in result.stdout:
            FAILURES.append(
                f"the script did not confirm the remote branch was cleaned up: "
                f"{result.stdout!r}"
            )

    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed:\n", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}\n", file=sys.stderr)
        return 1
    print(f"issue-land merge check passed ({channel}).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
