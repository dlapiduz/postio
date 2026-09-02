#!/usr/bin/env python3
"""Self-test: a merge GitHub calls MERGED is waited for, not given up on.

#406 taught this script to retry the "did it land?" check instead of asking
once, and #418 scaled the window with the number of commits. Both are guesses
at how long replication takes, and a guess can be short: #749's landing spent
its 120s, printed

    MERGE DID NOT LAND. gh reported success and origin/main does not
    carry the commits above.

and the commits were on `main` a minute later. The session then had to verify
by hand the very thing this loop exists to answer, and the exit status said
"failed" for work that had succeeded.

The PR's own state is the fact that settles it. MERGED means GitHub completed
the merge and the only thing outstanding is the new tip becoming visible here,
so that is a state to keep waiting in rather than to fail in.

**This does not weaken #312**, the guard the whole block exists for -- a
`gh pr merge` that exits 0 having done nothing. Success is still only ever
declared by seeing the subjects actually arrive on the base branch; the extra
patience is spent only where the PR itself says the merge happened, and a PR
that claims MERGED while its commits never appear still fails, just later.

Two cases:

1. the ref arrives *after* the first deadline, PR says MERGED -> the script
   waits it out and reports success;
2. the ref never arrives at all, PR says MERGED -> the script still fails,
   so the patience above cannot be mistaken for trusting `gh`.

Usage: scripts/tests/test-issue-land-merged-lag.py
Exit status: 0 both cases behaved, 1 otherwise.
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

# The merge succeeds and the PR reports MERGED; the ref shows up well after
# the first deadline. `disown` so the push outlives the `gh` process, the way
# GitHub's replication outlives the API call.
GH_STUB_LATE = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
printf '%s\\n' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    printf '%s' "$*" | grep -q -- "--json state" && { echo "MERGED"; exit 0; }
    printf '%s' "$*" | grep -q -- "--json url" && { echo "https://example.com/pull/1"; exit 0; }
    printf '%s' "$*" | grep -q -- "--json number" && { echo "1"; exit 0; }
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "checks" ]; then echo "[]"; exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
    ( sleep "$PUSH_DELAY"; git push -q "$ORIGIN" "HEAD:refs/heads/main" ) >/dev/null 2>&1 &
    disown
    echo "Merged"
    exit 0
fi
exit 0
"""

# Says MERGED and never pushes: the #312 shape, which must still fail.
GH_STUB_NEVER = GH_STUB_LATE.replace(
    '( sleep "$PUSH_DELAY"; git push -q "$ORIGIN" "HEAD:refs/heads/main" ) >/dev/null 2>&1 &\n    disown\n',
    "",
)


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
        cwd=cwd,
        check=True,
        capture_output=True,
    )


def run_landing(
    base: Path, name: str, channel: str, stub: str, push_delay: str, merged_timeout: str
) -> subprocess.CompletedProcess[str]:
    """One landing against its own repo, remote and `gh` stub."""
    root = base / name
    origin = base / f"{name}.git"
    stub_dir = base / f"{name}-stub"
    (stub_dir / "bin").mkdir(parents=True)
    (stub_dir / "bin" / "gh").write_text(stub, encoding="utf-8")
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
    git("checkout", "-q", "-b", "issue-749-x", cwd=root)
    (root / "dummy" / "src" / "extra.rs").write_text("// nothing\n", encoding="utf-8")

    environment = dict(os.environ)
    environment.pop("RUSTUP_TOOLCHAIN", None)
    environment["CARGO_TARGET_DIR"] = str(base / "target")
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment["ORIGIN"] = str(origin)
    environment["PUSH_DELAY"] = push_delay
    environment["POSTIO_CHECKS_GRACE"] = "1"
    environment["POSTIO_CHECKS_POLL"] = "1"
    # A first window far too short to catch the push, so the extended wait is
    # what the outcome depends on rather than luck.
    environment["POSTIO_LANDED_TIMEOUT"] = "1"
    environment["POSTIO_LANDED_POLL"] = "1"
    environment["POSTIO_MERGED_TIMEOUT"] = merged_timeout
    return subprocess.run(
        ["bash", "scripts/issue-land.sh", "-m", "feat(dummy): add a file"],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
        timeout=180,
    )


def main() -> int:
    if shutil.which("cargo") is None:
        print("skip: no cargo on PATH", file=sys.stderr)
        return 0
    channel = pinned_channel()

    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        base = Path(directory)

        # ── 1. late, but it does arrive ──────────────────────────────────
        result = run_landing(base, "late", channel, GH_STUB_LATE, "8", "90")
        if result.returncode != 0:
            FAILURES.append(
                "a MERGED pull request whose ref arrived after the first "
                "deadline was reported as a failed landing:\n"
                f"{result.stdout}\n{result.stderr}"
            )
        if "MERGE DID NOT LAND" in result.stderr:
            FAILURES.append(
                "the session was told the merge did not land, for work that "
                f"reached the base branch a few seconds later:\n{result.stderr}"
            )
        if "merged." not in result.stdout:
            FAILURES.append(f"the landing never reported success:\n{result.stdout}")

        # ── 2. MERGED but the commits never appear: still a failure ───────
        #
        # The patience above must not become trust. This is #312's shape --
        # `gh` reporting a merge that put nothing anywhere -- and it has to
        # keep failing, or the guard is gone.
        result = run_landing(base, "never", channel, GH_STUB_NEVER, "0", "5")
        if result.returncode == 0:
            FAILURES.append(
                "a pull request that says MERGED while its commits never "
                "reach the base branch was accepted as a successful landing "
                f"-- #312's guard is gone:\n{result.stdout}\n{result.stderr}"
            )
        if "MERGE DID NOT LAND" not in result.stderr:
            FAILURES.append(
                "a merge that genuinely did not land should still say so:\n"
                f"{result.stderr}"
            )

    for failure in FAILURES:
        print(f"FAIL {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-land merged-lag self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
