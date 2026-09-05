#!/usr/bin/env python3
"""Self-test for issue #742: green gates are not re-run on an unchanged tree.

Long commands on this workstation get killed sometimes (documented in
docs/engineering-notes.md), and every killed `issue-land.sh` retry used to
re-run the whole gate chain -- clippy and the full per-crate test suite,
minutes each -- against a tree that had not changed a byte since the gates
last went green. Landing #109 paid the postio-app gates three times that way.

So the script records `git write-tree` (the staged tree's content hash --
staging already happens before the gates, #270) plus the crate list after
the gates pass, in the worktree's own git dir. A later run whose staged
tree and crate list match exactly skips clippy and the per-crate tests,
loudly; `check.sh` still runs (it is seconds). Any content change -- an
edit, what `cargo fmt` itself rewrites, a rebase, a toolchain bump
(rust-toolchain.toml is tracked) -- changes the hash and the gates run in
full. The cases below pin both directions, and the timing lines #742 also
asks for.

Usage: scripts/tests/test-issue-land-gate-cache.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
REPO_ROOT = HERE.parent
ISSUE_LAND = HERE / "issue-land.sh"

# Stubbed for the same reason test-issue-land-commit-guard.py stubs them:
# this test is about the gate cache, and the checks have self-tests of
# their own.
STUB_CHECKS = [
    "check-crate-boundaries.py",
    "check-no-personal-data.py",
    "check-no-silent-tracking.py",
    "check-toolchain-pinned.py",
    "check-no-gtk-init-in-unit-tests.py",
    "check-runtime-crossings.py",
]

FAILURES: list[str] = []


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
        '[workspace]\nmembers = ["crates/dummy"]\nresolver = "2"\n',
        encoding="utf-8",
    )
    dummy = root / "crates" / "dummy" / "src"
    dummy.mkdir(parents=True)
    (root / "crates" / "dummy" / "Cargo.toml").write_text(
        '[package]\nname = "dummy"\nversion = "0.1.0"\nedition = "2021"\n',
        encoding="utf-8",
    )
    (dummy / "lib.rs").write_text("pub fn x() {}\n", encoding="utf-8")

    scripts = root / "scripts"
    scripts.mkdir()
    (scripts / "checks").mkdir()
    shutil.copy(HERE / "check.sh", scripts / "check.sh")
    (scripts / "check.sh").chmod(0o755)
    shutil.copy(ISSUE_LAND, scripts / ISSUE_LAND.name)
    (scripts / ISSUE_LAND.name).chmod(0o755)
    for name in STUB_CHECKS:
        (scripts / "checks" / name).write_text(
            "#!/usr/bin/env python3\nraise SystemExit(0)\n", encoding="utf-8"
        )
        (scripts / "checks" / name).chmod(0o755)


def git(*args: str, cwd: Path) -> None:
    subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
    )


def land(root: Path, target: Path, *args: str) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment.pop("RUSTUP_TOOLCHAIN", None)
    environment["CARGO_TARGET_DIR"] = str(target)
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    return subprocess.run(
        ["bash", "scripts/issue-land.sh", *args],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
    )


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def main() -> int:
    if shutil.which("cargo") is None:
        print("skip: no cargo on PATH", file=sys.stderr)
        return 0

    channel = pinned_channel()

    # Inside the current worktree, for the same reason the commit-guard test
    # is: the shared-tree guard only lifts its refusals for worktree paths.
    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        base = Path(directory)
        target = base / "target"

        root = base / "sandbox"
        root.mkdir()
        origin = base / "origin.git"
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
        build_sandbox(root, channel)
        # The real repository commits its Cargo.lock; without one here the
        # first gate run generates it, which changes the tree and makes the
        # second run look like new content -- a sandbox artifact, not the
        # behavior under test.
        environment = dict(os.environ)
        environment.pop("RUSTUP_TOOLCHAIN", None)
        environment["CARGO_TARGET_DIR"] = str(target)
        subprocess.run(
            ["cargo", "generate-lockfile"],
            cwd=root,
            env=environment,
            check=True,
            capture_output=True,
        )
        git("init", "-q", "-b", "main", cwd=root)
        git("config", "user.email", "test@example.com", cwd=root)
        git("config", "user.name", "Test", cwd=root)
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "init", cwd=root)
        git("remote", "add", "origin", str(origin), cwd=root)
        git("push", "-q", "origin", "main", cwd=root)
        git("checkout", "-q", "-b", "issue-1-x", cwd=root)

        # A crate change, uncommitted: --gates-only stages and runs the
        # gates against it, exactly the state a session verifies from.
        (root / "crates" / "dummy" / "src" / "lib.rs").write_text(
            "pub fn x() {}\npub fn y() {}\n", encoding="utf-8"
        )

        # ── the first run runs the gates and records the green tree ───────
        first = land(root, target, "--gates-only")
        output = first.stdout + first.stderr
        case(
            "the first run runs clippy for the changed crate",
            first.returncode == 0 and "clippy: dummy" in output,
            f"exit {first.returncode}\n{output}",
        )
        case(
            "and prints how long each gate phase took",
            re.search(r"^\[timing\] clippy dummy: \d+s$", output, re.MULTILINE)
            is not None
            # The default tier is the workspace's unit tests, not a
            # per-crate run, since #847 -- so the timing line names the tier.
            # `--full` is what still prints `test <crate>`.
            and re.search(r"^\[timing\] sanity tier: \d+s$", output, re.MULTILINE)
            is not None,
            f"no [timing] lines for the crate's gates\n{output}",
        )

        # ── the second run on the identical tree skips clippy and tests ───
        second = land(root, target, "--gates-only")
        output = second.stdout + second.stderr
        case(
            "an unchanged tree does not re-run clippy",
            second.returncode == 0 and "clippy: dummy" not in output,
            f"exit {second.returncode}\n{output}",
        )
        case(
            "and says loudly why it was skipped",
            "already green" in output,
            f"the skip has to be on the record, not silent\n{output}",
        )

        # ── the record survives a commit of the same content ──────────────
        # The kill-retry case commits between runs (a retried full land
        # commits first, then re-runs); the *content* is what the record is
        # keyed on, so a commit alone must not invalidate it.
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "feat(dummy): grow", cwd=root)
        committed = land(root, target, "--gates-only")
        output = committed.stdout + committed.stderr
        case(
            "committing the same content keeps the gates green",
            committed.returncode == 0 and "clippy: dummy" not in output,
            f"exit {committed.returncode}\n{output}",
        )

        # ── any content change runs the gates in full again ───────────────
        (root / "crates" / "dummy" / "src" / "lib.rs").write_text(
            "pub fn x() {}\npub fn y() {}\npub fn z() {}\n", encoding="utf-8"
        )
        changed = land(root, target, "--gates-only")
        output = changed.stdout + changed.stderr
        case(
            "a changed tree re-runs clippy",
            changed.returncode == 0 and "clippy: dummy" in output,
            f"exit {changed.returncode}\n{output}",
        )

        # ── a full land on the recorded tree still pushes ─────────────────
        # --wip stops after the push, before anything needs gh. The gates
        # were just recorded green by the run above; the land must skip them
        # and still do its real job.
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "feat(dummy): grow again", cwd=root)
        wip = land(root, target, "--wip")
        output = wip.stdout + wip.stderr
        case(
            "a full land on the recorded tree skips the gates and pushes",
            wip.returncode == 0
            and "clippy: dummy" not in output
            and "pushed issue-1-x" in output,
            f"exit {wip.returncode}\n{output}",
        )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print(f"issue-land gate-cache check passed ({channel}).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
