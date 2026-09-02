#!/usr/bin/env python3
"""Self-test: landing a branch a second time still pushes.

`issue-land.sh` rebases onto `origin/<base>` before pushing. That makes the
second push of a branch already on the remote necessarily non-fast-forward --
the commits have new hashes -- so a plain `git push` is rejected with

    ! [rejected] issue-N-x -> issue-N-x (non-fast-forward)
    hint: Updates were rejected because the tip of your current branch is
    hint: behind its remote counterpart.

after the gates have already run. The script's header authorises
`--force-with-lease` for exactly this and the code did not use it.

**Why it stayed hidden.** While `ci.yml` was `workflow_dispatch`-only a
landing succeeded on its first attempt: push once, merge, done. Nothing ever
pushed the same branch twice. A CI-gated landing fails on a red check, gets
fixed on the same branch, and lands again -- and *every* one of those second
attempts hit this. Turning CI back on (#781) is what made the path reachable,
which is why the regression test arrives with it.

Leased rather than bare, and that is the part worth keeping: the push must go
through for this script's own rebase and still refuse if the remote moved for
any other reason. Both directions are asserted below.

Each case runs `issue-land.sh --wip`, which stops right after pushing and
never touches `gh`.

Usage: scripts/tests/test-issue-land-repush.py
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


def git(*args: str, cwd: Path) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(
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
        ["bash", "scripts/issue-land.sh", "--wip", *args],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
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
        git("checkout", "-q", "-b", "issue-781-x", cwd=root)

        # ── first landing: pushes the branch ─────────────────────────────
        (root / "dummy" / "src" / "extra.rs").write_text("// one\n", encoding="utf-8")
        first = land(root, target, "-m", "feat(dummy): add a file")
        if first.returncode != 0:
            FAILURES.append(f"the first landing failed:\n{first.stdout}\n{first.stderr}")
            return report()

        # ── main moves, exactly as it does while a PR sits in review ─────
        #
        # This is what forces the rebase on the next run, and so what makes
        # the second push non-fast-forward. Committed on a detached worktree
        # of the bare remote's main so the branch under test is untouched.
        elsewhere = base / "elsewhere"
        subprocess.run(
            ["git", "clone", "-q", str(origin), str(elsewhere)], check=True
        )
        (elsewhere / "OTHER.md").write_text("landed meanwhile\n", encoding="utf-8")
        git("add", "-A", cwd=elsewhere)
        git("commit", "-q", "-m", "docs: something else landed", cwd=elsewhere)
        git("push", "-q", "origin", "main", cwd=elsewhere)

        # ── second landing of the same branch, the CI-failure shape ──────
        (root / "dummy" / "src" / "extra.rs").write_text("// two\n", encoding="utf-8")
        second = land(root, target, "-m", "fix(dummy): correct the file")
        if second.returncode != 0:
            FAILURES.append(
                "landing the same branch a second time failed. This is the "
                "path a CI-gated landing takes every time a check goes red: "
                "the rebase gives the commits new hashes, so the push is "
                "non-fast-forward and needs --force-with-lease.\n"
                f"{second.stdout}\n{second.stderr}"
            )
        elif "non-fast-forward" in second.stderr:
            FAILURES.append(f"the push was rejected:\n{second.stderr}")

        # The remote must now carry the second landing's work.
        remote_log = subprocess.run(
            ["git", "log", "origin/issue-781-x", "--format=%s"],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
        ).stdout
        if "fix(dummy): correct the file" not in remote_log:
            FAILURES.append(
                f"the remote branch does not carry the second commit:\n{remote_log}"
            )

        # ── the lease still refuses a remote somebody else moved ─────────
        #
        # The point of the leased form over a bare --force: our own rebase
        # goes through, and this must not.
        (elsewhere / "sneak.md").write_text("not ours\n", encoding="utf-8")
        git("add", "-A", cwd=elsewhere)
        git("commit", "-q", "-m", "docs: a commit this worktree never saw", cwd=elsewhere)
        git("push", "-q", "-f", "origin", "HEAD:refs/heads/issue-781-x", cwd=elsewhere)

        (root / "dummy" / "src" / "extra.rs").write_text("// three\n", encoding="utf-8")
        third = land(root, target, "-m", "fix(dummy): a third change")
        if third.returncode == 0:
            FAILURES.append(
                "the push overwrote a remote branch that had moved underneath "
                "this worktree -- --force-with-lease must refuse that, or it "
                "is a bare --force wearing a longer name.\n"
                f"{third.stdout}\n{third.stderr}"
            )

    return report()


def report() -> int:
    for failure in FAILURES:
        print(f"FAIL {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-land re-push self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
