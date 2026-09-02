#!/usr/bin/env python3
"""Self-test for issue #120: the commit guard in scripts/issue-land.sh.

The script used to demand `-m` even when there was nothing left to commit --
a clean tree with commits already on the branch, exactly the state CLAUDE.md's
"commit each piece of work as you finish it" rule leaves a session in, was
refused with "Nothing committed" until a redundant `-m` was supplied. The fix
moves the guard so it only fires when there is something to commit, and adds
a companion guard for the branch having nothing to land at all -- a state the
first fix would otherwise let sail through to a push and an empty PR.

Each case runs `issue-land.sh --wip`, which stops right after pushing and
never touches `gh` -- so this needs no stub for it, just a real git repo with
a fake `origin` next door (a bare repo, not GitHub) and a minimal Rust
workspace for the gates to run against for real.

Usage: scripts/tests/test-issue-land-commit-guard.py
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

# The six invariant checks `issue-land.sh` runs unconditionally. Stubbed here
# because this test is about the commit guard, not about them -- they are
# exercised by their own self-tests.
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
    """The version this repository actually pins, so the sandbox's own pin
    names a toolchain that is genuinely installed on this machine."""
    text = (REPO_ROOT / "rust-toolchain.toml").read_text(encoding="utf-8")
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("channel"):
            return line.split("=", 1)[1].strip().strip('"')
    raise RuntimeError("rust-toolchain.toml names no channel")


def build_sandbox(root: Path, channel: str) -> None:
    """A minimal git repo: one crate, the real pin, the fixed script."""
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
    # No global identity: a CI runner has none, and the repo-local one set in
    # `fresh_branch` is the thing under test -- this must not pass by
    # accident on a machine that happens to have one of its own configured.
    environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
    environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
    return subprocess.run(
        ["bash", "scripts/issue-land.sh", "--wip", *args],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
    )


def case(
    name: str,
    result: subprocess.CompletedProcess[str],
    *,
    expected_status: int,
    expect_output: str = "",
) -> None:
    if result.returncode != expected_status:
        FAILURES.append(
            f"{name}: expected exit {expected_status}, got {result.returncode}\n"
            f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}"
        )
        return
    output = result.stdout + result.stderr
    if expect_output and expect_output not in output:
        FAILURES.append(f"{name}: expected {expect_output!r} in the output\n{output}")


def main() -> int:
    if shutil.which("cargo") is None:
        print("skip: no cargo on PATH", file=sys.stderr)
        return 0

    channel = pinned_channel()

    # This test is about the sandbox in isolation from #112's fix; run it
    # inside the current worktree for the same reason that test does --
    # .claude/hooks/guard-shared-tree.py only lifts its `cargo fmt --all`
    # refusal for paths under ~/src/postio-worktrees.
    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        base = Path(directory)
        target = base / "target"

        def fresh_branch(name: str) -> Path:
            """A repo with one commit on main, checked out onto its own
            issue branch -- the state issue-claim.sh leaves a session in.

            Its own bare `origin` too: each case pushes independently, and a
            shared one would reject the second case's unrelated history as a
            non-fast-forward.
            """
            root = base / name
            root.mkdir()
            origin = base / f"{name}.git"
            subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
            build_sandbox(root, channel)
            git("init", "-q", "-b", "main", cwd=root)
            # Local, not just the `-c` flags this helper passes on its own
            # calls: issue-land.sh runs `git commit` directly, with no
            # identity of its own, and a CI runner has no global one either.
            git("config", "user.email", "test@example.com", cwd=root)
            git("config", "user.name", "Test", cwd=root)
            git("add", "-A", cwd=root)
            git("commit", "-q", "-m", "init", cwd=root)
            git("remote", "add", "origin", str(origin), cwd=root)
            git("push", "-q", "origin", "main", cwd=root)
            git("checkout", "-q", "-b", "issue-1-x", cwd=root)
            return root

        # ── a clean tree with a prior commit lands with no -m ─────────────
        root = fresh_branch("already-committed")
        (root / "dummy" / "src" / "extra.rs").write_text("// nothing\n", encoding="utf-8")
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "feat(dummy): add a file", cwd=root)
        case(
            "a clean tree with a prior commit lands with no -m",
            land(root, target),
            expected_status=0,
            expect_output="pushed issue-1-x",
        )

        # ── a dirty tree with no -m still refuses, saying the tree is dirty ─
        root = fresh_branch("dirty-no-message")
        (root / "dummy" / "src" / "extra.rs").write_text("// nothing\n", encoding="utf-8")
        case(
            "a dirty tree with no -m refuses, naming the dirty tree",
            land(root, target),
            expected_status=2,
            expect_output="Uncommitted changes",
        )

        # ── a dirty tree with -m still commits and proceeds, as before ─────
        root = fresh_branch("dirty-with-message")
        (root / "dummy" / "src" / "extra.rs").write_text("// nothing\n", encoding="utf-8")
        result = land(root, target, "-m", "feat(dummy): add a file")
        case(
            "a dirty tree with -m commits and proceeds",
            result,
            expected_status=0,
            expect_output="pushed issue-1-x",
        )
        log = subprocess.run(
            ["git", "log", "-1", "--format=%s"],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        if log != "feat(dummy): add a file":
            FAILURES.append(
                f"a dirty tree with -m commits and proceeds: expected the "
                f"message to land as the commit subject, got {log!r}"
            )

        # ── a clean tree with nothing to land at all refuses too ───────────
        root = fresh_branch("nothing-to-land")
        case(
            "a clean branch with no commits beyond main refuses",
            land(root, target),
            expected_status=2,
            expect_output="Nothing to land",
        )

        # ── rustfmt's own output is amended, not asked about ───────────────
        #
        # The session committed everything and then this script reformatted
        # the tree underneath it. Demanding `-m` there asks for a message for
        # changes the session did not make, about work it already committed --
        # and it used to ask *after* the gates, so the ten minutes were spent
        # before the question. The formatter's output belongs to the commit it
        # reformats.
        root = fresh_branch("rustfmt-amends")
        (root / "dummy" / "src" / "extra.rs").write_text(
            "pub fn  y ( ) {}\n", encoding="utf-8"
        )
        # Declared, or rustfmt never sees it: cargo fmt walks `mod` from the
        # crate root and skips orphan files entirely.
        (root / "dummy" / "src" / "lib.rs").write_text(
            "pub mod extra;\npub fn x() {}\n", encoding="utf-8"
        )
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "feat(dummy): add a badly formatted file", cwd=root)
        before = subprocess.run(
            ["git", "rev-list", "--count", "HEAD"],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        case(
            "rustfmt's changes to a committed tree are amended, not refused",
            land(root, target),
            expected_status=0,
            expect_output="amending",
        )
        after = subprocess.run(
            ["git", "rev-list", "--count", "HEAD"],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        if after != before:
            FAILURES.append(
                f"rustfmt's changes should be amended into the existing commit, "
                f"not added as a new one: {before} commit(s) became {after}"
            )
        subject = subprocess.run(
            ["git", "log", "-1", "--format=%s"],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        if subject != "feat(dummy): add a badly formatted file":
            FAILURES.append(
                f"amending must keep the commit's own subject, got {subject!r}"
            )
        dirty = subprocess.run(
            ["git", "status", "--porcelain"],
            cwd=root,
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
        if dirty:
            FAILURES.append(
                f"the tree should be clean after the amend, still dirty:\n{dirty}"
            )

        # ── but a clean branch with no commits of its own is not amendable ──
        #
        # There is nothing of this session's to amend into, and HEAD is main's
        # own commit -- amending it would rewrite somebody else's history to
        # carry a whitespace fix.
        # The badly formatted file has to be *committed on main*, so the
        # branch is clean, has nothing of its own, and rustfmt still finds
        # something to rewrite -- the one state where the amend above would
        # reach past this session's work into somebody else's commit.
        root = fresh_branch("rustfmt-nothing-to-amend")
        git("checkout", "-q", "main", cwd=root)
        (root / "dummy" / "src" / "untidy.rs").write_text(
            "pub fn  y ( ) {}\n", encoding="utf-8"
        )
        (root / "dummy" / "src" / "lib.rs").write_text(
            "pub mod untidy;\npub fn x() {}\n", encoding="utf-8"
        )
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "chore(dummy): untidy file", cwd=root)
        git("push", "-q", "origin", "main", cwd=root)
        git("checkout", "-q", "issue-1-x", cwd=root)
        git("reset", "-q", "--hard", "main", cwd=root)
        case(
            "rustfmt's changes with no commit of our own still refuse",
            land(root, target),
            expected_status=2,
            expect_output="Uncommitted changes",
        )

    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed:\n", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}\n", file=sys.stderr)
        return 1
    print(f"issue-land commit-guard check passed ({channel}).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
