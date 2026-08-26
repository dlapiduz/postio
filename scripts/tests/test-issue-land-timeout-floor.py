#!/usr/bin/env python3
"""Self-test for issue #418: 30s is not enough for the landed-check.

#406 taught `issue-land.sh` to poll `origin/main` for the landed commits
rather than checking once, because `gh pr merge` returns as soon as GitHub
*accepts* the merge and replication can lag a few seconds behind that. The
poll's ceiling, `POSTIO_LANDED_TIMEOUT`, defaulted to 30s -- and recurred on
PR #417: a ten-commit rebase merge took longer than that to replicate, and
the script printed "MERGE DID NOT LAND" for work that was, in fact, landed.

This reproduces the same shape of lag as #406's own test
(`test-issue-land-lagging-ref.py`), but past the *old* 30s default -- 50s --
to prove the default itself, not just the polling mechanism, is now wide
enough. `POSTIO_LANDED_TIMEOUT` is deliberately left unset: this is a test of
what the script does with nothing overridden.

Usage: scripts/tests/test-issue-land-timeout-floor.py
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

# Same shape as #406's stub -- merge succeeds at once, the ref shows up late
# -- except the delay is 50s, past the *old* 30s default and still short of
# a defensible new one.
GH_STUB_LAGGING = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    printf '%s' "$*" | grep -q -- "--json state" && { echo "OPEN"; exit 0; }
    printf '%s' "$*" | grep -q -- "--json url" && { echo "https://example.com/pull/1"; exit 0; }
    printf '%s' "$*" | grep -q -- "--json number" && { echo "1"; exit 0; }
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "checks" ]; then echo "[]"; exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
    ( sleep 50; git push -q "$ORIGIN" "HEAD:refs/heads/main" ) >/dev/null 2>&1 &
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

        subprocess.run(["git", "init", "-q", "--bare", str(origin)], check=True)
        root.mkdir()
        build_sandbox(root, channel)
        git("init", "-q", "-b", "main", cwd=root)
        git("config", "user.email", "test@example.com", cwd=root)
        git("config", "user.name", "Test", cwd=root)
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "init", cwd=root)
        git("remote", "add", "origin", str(origin), cwd=root)
        git("push", "-q", "origin", "main", cwd=root)
        git("checkout", "-q", "-b", "issue-418-x", cwd=root)
        (root / "dummy" / "src" / "extra.rs").write_text("// nothing\n", encoding="utf-8")

        environment = dict(os.environ)
        environment.pop("RUSTUP_TOOLCHAIN", None)
        # The point of this test: nothing overrides the timeout.
        environment.pop("POSTIO_LANDED_TIMEOUT", None)
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
            cwd=root, env=environment, capture_output=True, text=True, timeout=180,
        )

        # The script's own poll only returns 0 once the commits are visible on
        # origin/main, so a successful run already proves the push landed --
        # no separate wait needed here the way #406's own test needed one.
        landed = git("log", "origin/main", "--format=%s", cwd=root).stdout.decode()
        remotes = git("ls-remote", "--heads", "origin", cwd=root).stdout.decode()

        if result.returncode != 0:
            FAILURES.append(
                "#418: a merge that landed 50s late -- past the old 30s "
                f"default -- was reported as one that did not land:\n"
                f"{result.stdout}\n{result.stderr}"
            )
        if "MERGE DID NOT LAND" in result.stderr:
            FAILURES.append(
                "#418: the default landed-check timeout is still too short "
                f"for a merge that replicates in 50s:\n{result.stderr}"
            )
        if "merged." not in result.stdout:
            FAILURES.append(f"the landing never reported success:\n{result.stdout}")
        if "feat(dummy): add a file" not in landed:
            FAILURES.append(
                f"origin/main never got the commit even after the script "
                f"returned:\n{landed}"
            )
        if "issue-418-x" in remotes:
            FAILURES.append(
                "a landing that succeeded should still delete the remote branch"
            )

    for failure in FAILURES:
        print(f"FAIL {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-land timeout-floor self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
