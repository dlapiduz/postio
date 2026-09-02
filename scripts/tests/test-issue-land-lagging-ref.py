#!/usr/bin/env python3
"""Self-test for issue #406: a merge that lands must never be reported as one
that did not.

`gh pr merge --rebase` returns as soon as GitHub *accepts* the merge. The
`git fetch` on the next line can still be answered before the new tip is
visible, and the verification added for #312 asked once -- so a sub-second
replication lag became:

    MERGE DID NOT LAND. gh reported success and origin/main does not
    carry the commits above.
    ...
    Do not run issue-release.sh. Check the PR, then land onto a branch
    name that is not already spoken for:

which, for work that *did* land, tells the session to open a second PR for
commits already on `main` and to keep the worktree and the claim held. It
happened twice in three landings on 2026-08-26 (#194, #299).

The stub here reproduces the race deterministically: `gh pr merge` reports
success immediately and pushes to the bare remote a couple of seconds later,
from a background subshell. A single-shot check cannot see it; a retried one
can, and #312's own guarantee is untouched because the state that guards
against is permanent rather than late.

Usage: scripts/tests/test-issue-land-lagging-ref.py
Exit status: 0 the lagging merge was accepted, 1 otherwise.
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

FIXTURE_CI_YML = "name: CI\non:\n  workflow_dispatch:\n"

# The merge succeeds; the ref shows up late. `disown` so the push outlives the
# `gh` process the way GitHub's own replication outlives the API call.
GH_STUB_LAGGING = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
printf '%s\\n' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    printf '%s' "$*" | grep -q -- "--json state" && { echo "OPEN"; exit 0; }
    printf '%s' "$*" | grep -q -- "--json url" && { echo "https://example.com/pull/1"; exit 0; }
    printf '%s' "$*" | grep -q -- "--json number" && { echo "1"; exit 0; }
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "checks" ]; then echo "[]"; exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
    ( sleep 4; git push -q "$ORIGIN" "HEAD:refs/heads/main" ) >/dev/null 2>&1 &
    disown
    echo "Merged"
    exit 0
fi
exit 0
"""


def pinned_channel() -> str:
    text = (REPO_ROOT / "rust-toolchain.toml").read_text(encoding="utf-8")
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("channel"):
            return line.split("=", 1)[1].strip().strip('"')
    raise RuntimeError("rust-toolchain.toml names no channel")


def build_sandbox(root: Path, channel: str) -> None:
    (root / "rust-toolchain.toml").write_text(
        f'[toolchain]\nchannel = "{channel}"\nprofile = "minimal"\n', encoding="utf-8"
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
        cwd=cwd, check=True, capture_output=True,
    )


def main() -> int:
    if shutil.which("cargo") is None:
        print("skip: no cargo on PATH", file=sys.stderr)
        return 0
    channel = pinned_channel()

    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        base = Path(directory)
        root = base / "repo"
        origin = base / "origin.git"
        stub_dir = base / "stub"
        (stub_dir / "bin").mkdir(parents=True)
        (stub_dir / "bin" / "gh").write_text(GH_STUB_LAGGING, encoding="utf-8")
        (stub_dir / "bin" / "gh").chmod(0o755)
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
        git("checkout", "-q", "-b", "issue-406-x", cwd=root)
        (root / "dummy" / "src" / "extra.rs").write_text("// nothing\n", encoding="utf-8")

        environment = dict(os.environ)
        environment.pop("RUSTUP_TOOLCHAIN", None)
        environment["CARGO_TARGET_DIR"] = str(base / "target")
        environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
        environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
        environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
        environment["STUB_DIR"] = str(stub_dir)
        environment["ORIGIN"] = str(origin)
        environment["POSTIO_CHECKS_GRACE"] = "1"
        environment["POSTIO_CHECKS_POLL"] = "1"
        result = subprocess.run(
            ["bash", "scripts/issue-land.sh", "-m", "feat(dummy): add a file"],
            cwd=root, env=environment, capture_output=True, text=True, timeout=120,
        )

        # The stub's push is deliberately late, so this waits for it before
        # asking whether it happened -- otherwise a *correct* script and a
        # broken one would both be measured against a ref that had not
        # arrived, and the control below would prove nothing.
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            git("fetch", "-q", "origin", "main", cwd=root)
            landed = git("log", "origin/main", "--format=%s", cwd=root).stdout.decode()
            if "feat(dummy): add a file" in landed:
                break
            time.sleep(1)
        remotes = git("ls-remote", "--heads", "origin", cwd=root).stdout.decode()

        if "feat(dummy): add a file" not in landed:
            FAILURES.append(
                "the stub never landed anything, so this run proves nothing "
                f"about the retry:\n{landed}"
            )
        if result.returncode != 0:
            FAILURES.append(
                "#406: a merge that landed a few seconds late was reported as "
                f"one that did not land:\n{result.stdout}\n{result.stderr}"
            )
        if "MERGE DID NOT LAND" in result.stderr:
            FAILURES.append(
                "#406: the session was told to open a second PR for work that "
                f"is already on main:\n{result.stderr}"
            )
        if "merged." not in result.stdout:
            FAILURES.append(f"the landing never reported success:\n{result.stdout}")
        if "issue-406-x" in remotes:
            FAILURES.append(
                "a landing that succeeded should still delete the remote branch"
            )

    for failure in FAILURES:
        print(f"FAIL {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-land lagging-ref self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
