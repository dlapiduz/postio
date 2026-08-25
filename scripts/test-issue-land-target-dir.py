#!/usr/bin/env python3
"""Self-test for issue #253: the gates must not build in a shared target dir.

#178 established that two worktrees sharing one `CARGO_TARGET_DIR` can compile
one worktree's crate against another's -- a type error in a file that is
visibly correct, and a test binary linked against a sibling's library. The fix
gave every worktree its own `target/` and moved third-party caching to
`RUSTC_WRAPPER=sccache`, and CLAUDE.md stopped telling sessions to export
`CARGO_TARGET_DIR` at all.

`scripts/issue-land.sh` kept a default of `$MAIN_CHECKOUT/target`. With nobody
exporting the variable any more, that default fired on every landing, from
every worktree -- so the one cargo run that decides whether a branch is safe to
merge was the one still sharing artifacts with whatever else was landing at the
same instant. This proves the default is gone, and that an explicitly set
`CARGO_TARGET_DIR` is still honoured for the sessions that want one.

The proof is where the artifacts land, not what the script prints: a real
(tiny) crate is really built by `--gates-only`, and the directories are then
inspected. A banner line can be wrong in either direction; a `target/` full of
object files cannot.

The sandbox lives *inside* the current worktree rather than under the system
temp directory, for the same reason `test-rustup-toolchain-cleared.py` does:
`.claude/hooks/guard-shared-tree.py` only lifts its `cargo fmt --all` refusal
for paths under `~/src/postio-worktrees`.

Nothing here reaches the network -- the fake `origin` is a bare repo next door,
not GitHub -- and nothing here writes to the real main checkout: the fake one
is a directory in the sandbox, which is exactly what makes "did anything appear
under it" a usable assertion.

Usage: scripts/test-issue-land-target-dir.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO_ROOT = HERE.parent
ISSUE_LAND = HERE / "issue-land.sh"

# The six invariant checks `issue-land.sh` runs unconditionally. Stubbed here
# because this test is about the target directory, not about them -- they are
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
    """A minimal git repo: one crate under `crates/`, the real pin, the script.

    The crate has to live at `crates/<name>/` because that is the shape
    `issue-land.sh` greps for when deciding which crates changed -- and a run
    with no changed crate never invokes `cargo clippy`/`cargo test`, so it
    never creates a target directory and could not tell the two behaviours
    apart.
    """
    (root / "rust-toolchain.toml").write_text(
        f'[toolchain]\nchannel = "{channel}"\nprofile = "minimal"\n',
        encoding="utf-8",
    )
    (root / "Cargo.toml").write_text(
        '[workspace]\nmembers = ["crates/dummy"]\nresolver = "2"\n', encoding="utf-8"
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
    shutil.copy(ISSUE_LAND, scripts / ISSUE_LAND.name)
    (scripts / ISSUE_LAND.name).chmod(0o755)
    for name in STUB_CHECKS:
        (scripts / name).write_text(
            "#!/usr/bin/env python3\nraise SystemExit(0)\n", encoding="utf-8"
        )
        (scripts / name).chmod(0o755)


def git(*args: str, cwd: Path) -> None:
    subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
    )


def make_repo(base: Path, channel: str) -> Path:
    """A sandbox worktree on an issue branch whose diff touches a crate."""
    root = base / "repo"
    origin = base / "origin.git"
    root.mkdir()

    build_sandbox(root, channel)
    git("init", "-q", "-b", "main", cwd=root)
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "init", cwd=root)

    subprocess.run(["git", "init", "-q", "--bare", str(origin)], check=True)
    git("remote", "add", "origin", str(origin), cwd=root)
    git("push", "-q", "origin", "main", cwd=root)

    git("checkout", "-q", "-b", "issue-1-target-dir-test", cwd=root)
    # A committed change under `crates/` is what makes `issue-land.sh` run
    # clippy and test, which is what makes a target directory appear.
    (root / "crates" / "dummy" / "src" / "lib.rs").write_text(
        "pub fn x() {}\npub fn y() {}\n", encoding="utf-8"
    )
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "touch the crate", cwd=root)
    return root


def gates_only(
    root: Path, fake_main: Path, *, target: Path | None
) -> subprocess.CompletedProcess[str]:
    """Run the sandbox's `issue-land.sh --gates-only`.

    `POSTIO_MAIN_CHECKOUT` points at a directory in the sandbox, so the
    behaviour under test -- defaulting to `$MAIN_CHECKOUT/target` -- would be
    visible as artifacts appearing there, and the real checkout is never
    touched either way.
    """
    environment = dict(os.environ)
    environment.pop("CARGO_TARGET_DIR", None)
    if target is not None:
        environment["CARGO_TARGET_DIR"] = str(target)
    environment["POSTIO_MAIN_CHECKOUT"] = str(fake_main)
    return subprocess.run(
        ["bash", "scripts/issue-land.sh", "--gates-only"],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
    )


def built(directory: Path) -> bool:
    """Did cargo actually build into this directory?

    An empty `target/` is not proof of anything -- cargo creates one and then
    fails -- so this looks for the profile directory a real compile produces.
    """
    return (directory / "debug").is_dir()


def check(name: str, result: subprocess.CompletedProcess[str]) -> None:
    if result.returncode != 0:
        FAILURES.append(
            f"{name}: expected exit 0, got {result.returncode}\n"
            f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}"
        )


def test_no_default_to_the_shared_checkout(channel: str) -> None:
    """With nothing exported, the gates build in the worktree's own target."""
    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        base = Path(directory)
        fake_main = base / "main-checkout"
        fake_main.mkdir()
        root = make_repo(base, channel)

        name = "issue-land.sh: an unset CARGO_TARGET_DIR does not reach the shared checkout"
        result = gates_only(root, fake_main, target=None)
        check(name, result)

        if built(fake_main / "target"):
            FAILURES.append(
                f"{name}: the gates built into the main checkout's target "
                f"directory, which is the #178 hazard this issue is about"
            )
        if not built(root / "target"):
            FAILURES.append(
                f"{name}: nothing was built into the worktree's own target "
                f"directory, so the gates either did not run or went "
                f"somewhere else entirely\n"
                f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}"
            )


def test_an_explicit_setting_is_still_honoured(channel: str) -> None:
    """A caller staking a merge on a private target directory still gets one.

    `docs/engineering-notes.md` advises exactly this for a result you are
    staking a merge on, so dropping the default must not also drop the
    caller's ability to choose.
    """
    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        base = Path(directory)
        fake_main = base / "main-checkout"
        fake_main.mkdir()
        chosen = base / "chosen-target"
        root = make_repo(base, channel)

        name = "issue-land.sh: an explicit CARGO_TARGET_DIR is honoured"
        result = gates_only(root, fake_main, target=chosen)
        check(name, result)

        if not built(chosen):
            FAILURES.append(
                f"{name}: nothing was built into the directory the caller "
                f"named\n--- stdout ---\n{result.stdout}\n"
                f"--- stderr ---\n{result.stderr}"
            )
        if built(root / "target"):
            FAILURES.append(
                f"{name}: the gates built into the worktree's own target "
                f"directory as well, ignoring the caller's choice"
            )


def main() -> int:
    if shutil.which("cargo") is None:
        print("skip: no cargo on PATH", file=sys.stderr)
        return 0

    channel = pinned_channel()
    test_no_default_to_the_shared_checkout(channel)
    test_an_explicit_setting_is_still_honoured(channel)

    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed:\n", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}\n", file=sys.stderr)
        return 1
    print("target-directory check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
