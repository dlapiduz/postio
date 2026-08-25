#!/usr/bin/env python3
"""Self-test for issue #112: RUSTUP_TOOLCHAIN must not reach the gates.

`RUSTUP_TOOLCHAIN` in the environment beats `rust-toolchain.toml` -- this
project's own workstation exports it from `~/.config/mise/config.toml` -- so
a session could build, lint and test on a different compiler than CI while
`scripts/issue-land.sh` and `scripts/test-headless.sh` only warned about it,
or said nothing at all. The fix clears the variable before either one runs a
gate; this proves it rather than arguing it.

The proof does not need a fake compiler: a `RUSTUP_TOOLCHAIN` naming a version
rustup has never installed makes `rustc`/`cargo` refuse outright ("override
toolchain '...' is not installed"). So the case that matters for
`issue-land.sh` is exactly the one `--gates-only` would hit first --
`cargo fmt --all` -- and a real, minimal workspace is enough to exercise it
for real, with no cargo stubbed out. For `test-headless.sh` the wrapped
command is `rustc --version` itself. Before the fix both failed with
rustup's own error; after it, both run on the pinned compiler regardless.

The `issue-land.sh` sandbox lives *inside* the current worktree rather than
under the system temp directory: `.claude/hooks/guard-shared-tree.py` only
lifts its `cargo fmt --all` refusal for paths under `~/src/postio-worktrees`,
and a plain `mktemp -d` elsewhere is the shared checkout as far as that hook
can tell.

Nothing here reaches the network -- the fake `origin` is a bare repo next
door, not GitHub. `test-headless.sh`'s compositor check is satisfied with a
real Unix socket bound at the path it looks for, rather than a real mutter,
so this runs the same whether or not one is installed.

Usage: scripts/tests/test-rustup-toolchain-cleared.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import shutil
import socket
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
REPO_ROOT = HERE.parent
ISSUE_LAND = HERE / "issue-land.sh"
TEST_HEADLESS = HERE / "test-headless.sh"

# Not installed anywhere, ever -- that is the point. rustup refuses an
# override naming a toolchain it has never fetched rather than fetching one
# on the spot, so this is a reliable trigger rather than a guess about what
# happens to be absent on this machine.
BOGUS_TOOLCHAIN = "1.0.0-does-not-exist"

# The six invariant checks `issue-land.sh` runs unconditionally. Stubbed here
# because this test is about the toolchain, not about them -- they are
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


def gates_only(root: Path, target: Path, *, rustup_toolchain: str | None) -> subprocess.CompletedProcess[str]:
    """Run the sandbox's `issue-land.sh --gates-only`."""
    environment = dict(os.environ)
    environment.pop("RUSTUP_TOOLCHAIN", None)
    if rustup_toolchain is not None:
        environment["RUSTUP_TOOLCHAIN"] = rustup_toolchain
    environment["CARGO_TARGET_DIR"] = str(target)
    return subprocess.run(
        ["bash", "scripts/issue-land.sh", "--gates-only"],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
    )


def case(name: str, root: Path, target: Path, *, rustup_toolchain: str | None) -> subprocess.CompletedProcess[str]:
    result = gates_only(root, target, rustup_toolchain=rustup_toolchain)
    if result.returncode != 0:
        FAILURES.append(
            f"{name}: expected exit 0, got {result.returncode}\n"
            f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}"
        )
    return result


def test_issue_land_clears_the_override() -> str:
    """Returns the pinned channel, for the success message."""
    channel = pinned_channel()

    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        base = Path(directory)
        root = base / "repo"
        origin = base / "origin.git"
        target = base / "target"
        root.mkdir()

        build_sandbox(root, channel)
        git("init", "-q", "-b", "main", cwd=root)
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "init", cwd=root)

        subprocess.run(["git", "init", "-q", "--bare", str(origin)], check=True)
        git("remote", "add", "origin", str(origin), cwd=root)
        git("push", "-q", "origin", "main", cwd=root)
        git("checkout", "-q", "-b", "issue-1-toolchain-test", cwd=root)

        # ── the case this issue is about ─────────────────────────────────
        result = case(
            "issue-land.sh: a bogus RUSTUP_TOOLCHAIN does not reach the gates",
            root,
            target,
            rustup_toolchain=BOGUS_TOOLCHAIN,
        )
        if BOGUS_TOOLCHAIN not in result.stdout:
            FAILURES.append(
                "issue-land.sh: a bogus RUSTUP_TOOLCHAIN does not reach the "
                "gates: the diagnostic never named the override it cleared, "
                "so this proves nothing about what actually happened"
            )
        if "is not installed" in result.stderr or "is not installed" in result.stdout:
            FAILURES.append(
                "issue-land.sh: a bogus RUSTUP_TOOLCHAIN does not reach the "
                "gates: rustup's own 'is not installed' error surfaced, "
                "meaning some gate still saw the override"
            )

        # ── the baseline: nothing exported, nothing to clear or report ────
        result = case(
            "issue-land.sh: an unset RUSTUP_TOOLCHAIN changes nothing",
            root,
            target,
            rustup_toolchain=None,
        )
        if "RUSTUP_TOOLCHAIN" in result.stdout:
            FAILURES.append(
                "issue-land.sh: an unset RUSTUP_TOOLCHAIN changes nothing: "
                "the diagnostic mentioned it anyway"
            )

    return channel


def test_headless_sh_clears_the_override() -> None:
    """`test-headless.sh` fronts every ad-hoc GTK test run, so the same
    override has to be cleared before its `exec "$@"` too."""
    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        runtime_dir = Path(directory)
        display = "postio-headless-test"
        # `running()` in test-headless.sh only checks for a socket at this
        # path -- a real one, bound but never accepted from, satisfies it
        # without needing mutter installed, which CI does not have.
        server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        server.bind(str(runtime_dir / display))
        server.listen(1)
        try:
            environment = dict(os.environ)
            environment["XDG_RUNTIME_DIR"] = str(runtime_dir)
            environment["POSTIO_TEST_DISPLAY"] = display
            environment["RUSTUP_TOOLCHAIN"] = BOGUS_TOOLCHAIN
            result = subprocess.run(
                ["bash", str(TEST_HEADLESS), "rustc", "--version"],
                cwd=REPO_ROOT,
                env=environment,
                capture_output=True,
                text=True,
            )
        finally:
            server.close()

    if result.returncode != 0:
        FAILURES.append(
            "test-headless.sh: a bogus RUSTUP_TOOLCHAIN made the wrapped "
            f"command fail: expected exit 0, got {result.returncode}\n"
            f"--- stdout ---\n{result.stdout}\n--- stderr ---\n{result.stderr}"
        )
    if "rustc" not in result.stdout:
        FAILURES.append(
            "test-headless.sh: the wrapped `rustc --version` printed no "
            "version, so this proves nothing about what actually happened"
        )


def main() -> int:
    if shutil.which("cargo") is None:
        print("skip: no cargo on PATH", file=sys.stderr)
        return 0

    channel = test_issue_land_clears_the_override()
    test_headless_sh_clears_the_override()

    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed:\n", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}\n", file=sys.stderr)
        return 1
    print(f"toolchain-clearing check passed ({channel}).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
