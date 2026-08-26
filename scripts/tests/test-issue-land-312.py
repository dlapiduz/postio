#!/usr/bin/env python3
"""Self-test for issue #312: issue-land.sh must not report a merge that did
not happen.

The incident, twice in one session on #277. Another session had already landed
work for the same issue on a branch with the same generated name, and its PR
was merged and closed. Then:

  * `gh pr view --json number` succeeds for a *merged* PR -- it resolves the
    most recent PR for the head branch whatever its state -- so the script read
    it as "PR already open for $BRANCH; the push updated it".
  * `gh pr merge --rebase` printed "! Pull request #286 was already merged" and
    exited **0**. Nothing merged.
  * The script printed "merged.", deleted the remote branch, and exited 0 --
    then told the operator to run `issue-release.sh`, which removes the
    worktree holding the only remaining copy.

Two cases here, matching the two halves of the fix:

  * a merged PR on the same head branch must not be adopted: the script opens a
    new one instead;
  * a `gh pr merge` that reports success while merging nothing must fail the
    script, must not print "merged.", and must leave the remote branch alone.

`gh` is stubbed on PATH and `git` is real throughout, including a local bare
remote -- so "did the work reach the base" is answered by the repository rather
than by a stubbed opinion. The success-path stub actually pushes the branch to
`main`, which is what makes the failure-path stub's refusal to do so meaningful.

Usage: scripts/tests/test-issue-land-312.py
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

FIXTURE_CI_YML = "name: CI\non:\n  workflow_dispatch:\n"

# `pr view` answers OPEN, and `pr merge` really does move `main` in the bare
# remote. This is the shape of a working day, and the control for the two
# failure stubs below.
GH_STUB_HEALTHY = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    printf '%s' "$*" | grep -q -- "--json state" && { echo "OPEN"; exit 0; }
    printf '%s' "$*" | grep -q -- "--json url" && { echo "https://example.com/pull/1"; exit 0; }
    printf '%s' "$*" | grep -q -- "--json number" && { echo "1"; exit 0; }
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "checks" ]; then echo "[]"; exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
    # What a real rebase-merge does to the base, near enough: the commits
    # arrive with new hashes and the same subjects.
    git push -q "$ORIGIN" "HEAD:refs/heads/main" || exit 1
    echo "Merged"
    exit 0
fi
exit 0
"""

# The incident: a merged PR for this head branch, and a merge that no-ops.
GH_STUB_ALREADY_MERGED = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    printf '%s' "$*" | grep -q -- "--json state" && { echo "MERGED"; exit 0; }
    printf '%s' "$*" | grep -q -- "--json url" && { echo "https://example.com/pull/286"; exit 0; }
    printf '%s' "$*" | grep -q -- "--json number" && { echo "286"; exit 0; }
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "checks" ]; then echo "[]"; exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
    echo "! Pull request #286 was already merged" >&2
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


def land(root: Path, target: Path, stub_dir: Path, origin: Path):
    environment = dict(os.environ)
    environment.pop("RUSTUP_TOOLCHAIN", None)
    environment["CARGO_TARGET_DIR"] = str(target)
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["ORIGIN"] = str(origin)
    environment["POSTIO_CHECKS_GRACE"] = "1"
    environment["POSTIO_CHECKS_POLL"] = "1"
    # The state this test is about is permanent, not late: a merge that never
    # happened will not appear however long the check waits. Shortening the
    # window keeps that case fast; the lag case has a test of its own
    # (`test-issue-land-lagging-ref.py`, #406).
    environment["POSTIO_LANDED_TIMEOUT"] = "3"
    environment["POSTIO_LANDED_POLL"] = "1"
    return subprocess.run(
        ["bash", "scripts/issue-land.sh", "-m", "feat(dummy): add a file"],
        cwd=root, env=environment, capture_output=True, text=True, timeout=90,
    )


def scenario(base: Path, channel: str, stub: str, name: str):
    """One landing attempt in a repository of its own."""
    root = base / f"repo-{name}"
    origin = base / f"origin-{name}.git"
    stub_dir = base / f"stub-{name}"
    (stub_dir / "bin").mkdir(parents=True)
    (stub_dir / "bin" / "gh").write_text(stub, encoding="utf-8")
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
    git("checkout", "-q", "-b", "issue-312-x", cwd=root)
    (root / "dummy" / "src" / "extra.rs").write_text("// nothing\n", encoding="utf-8")

    result = land(root, base / "target", stub_dir, origin)
    remotes = git("ls-remote", "--heads", "origin", cwd=root).stdout.decode()
    landed = git("log", "origin/main", "--format=%s", cwd=root).stdout.decode()
    return result, remotes, landed, (stub_dir / "calls").read_text(encoding="utf-8")


def main() -> int:
    if shutil.which("cargo") is None:
        print("skip: no cargo on PATH", file=sys.stderr)
        return 0
    channel = pinned_channel()

    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        base = Path(directory)

        # ── the control: a merge that really merges ──────────────────────
        result, remotes, landed, _ = scenario(base, channel, GH_STUB_HEALTHY, "ok")
        if result.returncode != 0:
            FAILURES.append(
                "a genuine merge was rejected -- the verification is too "
                f"strict:\n{result.stdout}\n{result.stderr}"
            )
        if "merged." not in result.stdout:
            FAILURES.append(f"a genuine merge did not report success:\n{result.stdout}")
        if "feat(dummy): add a file" not in landed:
            FAILURES.append(f"the control did not actually land:\n{landed}")
        if "issue-312-x" in remotes:
            FAILURES.append("a successful merge should still delete the remote branch")

        # ── the incident ────────────────────────────────────────────────
        result, remotes, landed, calls = scenario(
            base, channel, GH_STUB_ALREADY_MERGED, "stale"
        )
        if result.returncode == 0:
            FAILURES.append(
                "#312: the script succeeded while nothing was merged:\n"
                f"{result.stdout}\n{result.stderr}"
            )
        if "merged." in result.stdout:
            FAILURES.append(
                f'#312: the script printed "merged." having merged nothing:\n{result.stdout}'
            )
        if "feat(dummy): add a file" in landed:
            FAILURES.append("the stale-PR stub was supposed to merge nothing")
        if "issue-312-x" not in remotes:
            FAILURES.append(
                "#312: the remote branch was deleted after a merge that did not "
                "happen -- that branch may be the only copy of the work"
            )
        if "pr create" not in calls:
            FAILURES.append(
                "#312: a MERGED PR on this head branch must not be adopted; the "
                f"script should open a new one:\n{calls}"
            )

    for failure in FAILURES:
        print(f"FAIL {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-land merge-verification self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
