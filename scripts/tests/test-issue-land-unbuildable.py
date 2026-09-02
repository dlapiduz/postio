#!/usr/bin/env python3
"""Self-test for #555: a crate this host cannot build must not be landed.

`issue-land.sh` runs the gate chain over the crates a branch changed. On a host
that cannot build one of them, that chain is not a weaker gate -- it is no gate
at all, and the work lands anyway.

This is live: a macOS session cannot build `postio-gtk` or `postio-app` (no
gtk4/libadwaita pkg-config, and webkitgtk has no arm64 bottle and no supported
upstream macOS backend). Without this guard such a session passes its gates and
pushes something that does not compile on Linux. CI would catch it on the pull
request, but a gate that silently proved nothing is worth refusing where it
runs, on a repository where several agents work concurrently on different
machines.

Two behaviours, because the two failure modes are different:

  * a changed crate the host cannot build is a **hard stop**, before anything
    is committed or pushed. There is no useful partial answer here;
  * a changed crate the unbuildable ones *depend on* still lands -- refusing
    would stop a macOS session doing any work at all -- but the PR is labelled
    `needs-linux-verify` so the gap is on the record rather than in someone's
    memory. `postio-app` depends on every other workspace crate, directly or
    transitively, so on a GTK-less host that is every crate.

The third case is the one that actually matters: on a host that *can* build
GTK, none of this fires and the script behaves exactly as it did. The whole
initiative is conditional on Linux not regressing.

Usage: scripts/tests/test-issue-land-unbuildable.py
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

STUB_CHECKS = [
    "check-crate-boundaries.py",
    "check-no-personal-data.py",
    "check-no-silent-tracking.py",
    "check-toolchain-pinned.py",
    "check-no-gtk-init-in-unit-tests.py",
    "check-runtime-crossings.py",
]

# Records every call so the assertions can ask what the script *did*, not what
# it printed. A banner can be wrong in either direction; an absent `pr create`
# cannot.
GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
printf '%s\\n' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "issue" ] && [ "$2" = "view" ]; then echo '{"title":"stub"}'; exit 0; fi
if [ "$1" = "issue" ] && [ "$2" = "edit" ]; then exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    if printf '%s' "$*" | grep -q -- "--json number"; then exit 1; fi
    if printf '%s' "$*" | grep -q -- "--json url"; then
        echo "https://example.com/pull/1"; exit 0
    fi
    exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "create" ]; then exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "checks" ]; then echo "[]"; exit 0; fi
if [ "$1" = "pr" ] && [ "$2" = "merge" ]; then
    BASE=$(cat "$(git rev-parse --git-dir)/postio-base" 2>/dev/null || echo main)
    git push -q origin "HEAD:refs/heads/$BASE" || exit 1
    echo "Merged"
    exit 0
fi
exit 0
"""

# The host's GTK availability, switchable per case. `$STUB_DIR/have-gtk`
# standing in for the real libraries is the whole point: the guard must key on
# what the host can build, and a test that keyed on the OS name would pass on a
# Linux box with no GTK installed while proving nothing.
PKGCONFIG_STUB = """#!/usr/bin/env bash
if [ "$1" = "--exists" ]; then
    [ -f "$STUB_DIR/have-gtk" ] && exit 0
    exit 1
fi
exit 0
"""

# `cargo` and `rustc` are stubbed: this test is about which crates the gates
# run over, not about the gates. Building a real crate three times would make
# it slow and prove nothing extra.
CARGO_STUB = """#!/usr/bin/env bash
printf 'cargo %s\\n' "$*" >> "$STUB_DIR/calls"
exit 0
"""
RUSTC_STUB = """#!/usr/bin/env bash
echo "rustc 1.98.0 (stub)"
"""

FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def git(*args: str, cwd: Path) -> None:
    subprocess.run(
        ["git", "-c", "user.email=test@example.com", "-c", "user.name=Test", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
    )


def world(base: Path, *, have_gtk: bool, touch: str) -> tuple[Path, Path]:
    """A sandbox worktree whose branch changes exactly one crate."""
    root = base / "repo"
    stub_dir = base / "stub"
    (stub_dir / "bin").mkdir(parents=True)
    (root / "scripts" / "checks").mkdir(parents=True)

    (root / "rust-toolchain.toml").write_text(
        '[toolchain]\nchannel = "1.98.0"\n', encoding="utf-8"
    )
    (root / "Cargo.toml").write_text("[workspace]\n", encoding="utf-8")
    for crate in ("postio-gtk", "postio-core", "postio-app"):
        (root / "crates" / crate / "src").mkdir(parents=True)
        (root / "crates" / crate / "src" / "lib.rs").write_text("// x\n", encoding="utf-8")

    shutil.copy(HERE / "check.sh", root / "scripts" / "check.sh")
    (root / "scripts" / "check.sh").chmod(0o755)
    shutil.copy(ISSUE_LAND, root / "scripts" / "issue-land.sh")
    (root / "scripts" / "issue-land.sh").chmod(0o755)
    shutil.copytree(HERE / "lib", root / "scripts" / "lib")
    # The land script shells out to this after opening the PR. Stubbed rather
    # than copied: the real one polls `gh pr checks`, which is its own test's
    # business, and its absence would make every case here exit non-zero --
    # which is how the first draft of this file reported a passing hard-stop
    # that had not actually stopped anything.
    wait = root / "scripts" / "wait-for-checks.sh"
    wait.write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
    wait.chmod(0o755)
    for name in STUB_CHECKS:
        p = root / "scripts" / "checks" / name
        p.write_text("#!/usr/bin/env python3\nraise SystemExit(0)\n", encoding="utf-8")
        p.chmod(0o755)

    for name, body in (
        ("gh", GH_STUB),
        ("pkg-config", PKGCONFIG_STUB),
        ("cargo", CARGO_STUB),
        ("rustc", RUSTC_STUB),
    ):
        p = stub_dir / "bin" / name
        p.write_text(body, encoding="utf-8")
        p.chmod(0o755)
    (stub_dir / "calls").write_text("", encoding="utf-8")
    if have_gtk:
        (stub_dir / "have-gtk").write_text("", encoding="utf-8")

    git("init", "-q", "-b", "main", cwd=root)
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "init", cwd=root)
    origin = base / "origin.git"
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
    git("remote", "add", "origin", str(origin), cwd=root)
    git("push", "-q", "origin", "main", cwd=root)

    git("checkout", "-q", "-b", "issue-1-unbuildable-test", cwd=root)
    (root / "crates" / touch / "src" / "lib.rs").write_text("// y\n", encoding="utf-8")
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", f"touch {touch}", cwd=root)
    return root, stub_dir


def land(root: Path, stub_dir: Path, *args: str) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment.pop("CARGO_TARGET_DIR", None)
    return subprocess.run(
        ["bash", "scripts/issue-land.sh", *args],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
        timeout=180,
    )


def main() -> int:
    # 1. The crate this host cannot build.
    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        base = Path(directory)
        root, stub_dir = world(base, have_gtk=False, touch="postio-gtk")
        result = land(root, stub_dir)
        out = result.stdout + result.stderr
        calls = (stub_dir / "calls").read_text(encoding="utf-8")

        case(
            "a crate the host cannot build stops the landing",
            result.returncode != 0,
            f"exit {result.returncode}; output:\n{out}",
        )
        case(
            "the refusal names the crate",
            "postio-gtk" in out,
            f"the crate is not named:\n{out}",
        )
        case(
            "the refusal names why the host cannot build it",
            "gtk4" in out or "pkg-config" in out,
            f"no reason given, so the reader cannot fix it:\n{out}",
        )
        case(
            "nothing was pushed",
            "pr create" not in calls,
            f"a PR was opened despite the refusal:\n{calls}",
        )

    # 2. A crate the unbuildable ones depend on: lands, but labelled.
    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        base = Path(directory)
        root, stub_dir = world(base, have_gtk=False, touch="postio-core")
        result = land(root, stub_dir)
        out = result.stdout + result.stderr
        calls = (stub_dir / "calls").read_text(encoding="utf-8")

        case(
            "a crate the host can build still lands",
            result.returncode == 0,
            f"exit {result.returncode}; output:\n{out}",
        )
        case(
            "the PR is labelled needs-linux-verify",
            "needs-linux-verify" in calls,
            f"the unverified gap was not recorded anywhere:\n{calls}",
        )

    # 3. A host that can build GTK: nothing fires, nothing changes.
    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        base = Path(directory)
        root, stub_dir = world(base, have_gtk=True, touch="postio-gtk")
        result = land(root, stub_dir)
        out = result.stdout + result.stderr
        calls = (stub_dir / "calls").read_text(encoding="utf-8")

        case(
            "on a host with GTK the same branch lands",
            result.returncode == 0,
            f"exit {result.returncode}; output:\n{out}",
        )
        case(
            "on a host with GTK nothing is labelled",
            "needs-linux-verify" not in calls,
            f"a Linux landing was labelled as unverified:\n{calls}",
        )
        case(
            "on a host with GTK the gates still ran over the crate",
            # Clippy is still per-crate; the tests are the sanity tier since
            # #847, so "the gates ran" is no longer spelled `test -p <crate>`.
            # What this case is really about is unchanged: a host that *can*
            # build the crate must not silently skip its gates.
            "clippy -p postio-gtk" in calls
            and "test --workspace --lib" in calls,
            f"the gates were skipped on a host that can run them:\n{calls}",
        )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("issue-land unbuildable-crate check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
