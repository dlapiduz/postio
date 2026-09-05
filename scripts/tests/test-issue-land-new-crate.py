#!/usr/bin/env python3
"""Self-test for #585: adding a workspace member widens the gate.

`cargo test -p postio-session` was red on `main` for two landings, and
neither landing could have seen it. `postio-session/src/logging.rs` keeps a
list of every workspace crate so a bare `POSTIO_LOG=debug` does not hold
Postio's own crates at `warn`, and it has a test that enumerates the
workspace and fails when one is missing:

    these crates would be held at `warn` by a bare POSTIO_LOG level:
        ["postio_ui", "postio_ffi"]

`postio-ui` arrived with #566 and `postio-ffi` with #571, from two different
sessions. **Neither branch changed `postio-session`**, so the per-crate gate
-- which runs clippy and tests over the crates a branch changed, deliberately
and for good reasons -- never compiled the test that was about to start
failing.

The per-crate gate is not the bug. Testing the whole workspace on every
landing is what it was changed away from, and going back would cost every
session minutes to catch something rare.

**Adding a crate is the one edit whose blast radius is definitionally outside
the crates it touches.** Anything that enumerates the workspace -- that list,
`check-lint-floor.py`, `check-crate-boundaries.py` -- can start failing
somewhere nobody looked. So a branch that touches the root `Cargo.toml`'s
`members` runs the workspace tests, and every other branch is untouched.

Asserting on the recorded `cargo` invocations rather than on a banner, for
the reason #419's self-test gives: a banner can claim a run that never
happened.

Usage: scripts/tests/test-issue-land-new-crate.py
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

PKGCONFIG_STUB = """#!/usr/bin/env bash
if [ "$1" = "--exists" ]; then exit 0; fi
exit 0
"""

CARGO_STUB = """#!/usr/bin/env bash
printf 'cargo %s\\n' "$*" >> "$STUB_DIR/calls"
exit 0
"""

RUSTC_STUB = """#!/usr/bin/env bash
echo "rustc 1.98.0 (stub)"
"""

MEMBERS = ("postio-gtk", "postio-core", "postio-app")

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


def workspace_toml(members: tuple[str, ...]) -> str:
    listed = ",\n".join(f'    "crates/{name}"' for name in members)
    return f"[workspace]\nresolver = \"2\"\nmembers = [\n{listed},\n]\n"


def world(base: Path, *, adds_a_crate: bool) -> tuple[Path, Path]:
    """A sandbox whose branch either adds a workspace member or does not."""
    root = base / "repo"
    stub_dir = base / "stub"
    (stub_dir / "bin").mkdir(parents=True)
    (root / "scripts" / "checks").mkdir(parents=True)

    (root / "rust-toolchain.toml").write_text(
        '[toolchain]\nchannel = "1.98.0"\n', encoding="utf-8"
    )
    (root / "Cargo.toml").write_text(workspace_toml(MEMBERS), encoding="utf-8")
    for crate in MEMBERS:
        (root / "crates" / crate / "src").mkdir(parents=True)
        (root / "crates" / crate / "src" / "lib.rs").write_text(
            "// x\n", encoding="utf-8"
        )

    shutil.copy(HERE / "check.sh", root / "scripts" / "check.sh")
    (root / "scripts" / "check.sh").chmod(0o755)
    shutil.copy(ISSUE_LAND, root / "scripts" / "issue-land.sh")
    (root / "scripts" / "issue-land.sh").chmod(0o755)
    shutil.copytree(HERE / "lib", root / "scripts" / "lib")
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

    git("init", "-q", "-b", "main", cwd=root)
    # A lockfile, because the gates `git add -A` before running and one
    # written mid-gates would otherwise land as an untracked stray.
    (root / "Cargo.lock").write_text("# stub\n", encoding="utf-8")
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "init", cwd=root)
    origin = base / "origin.git"
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
    git("remote", "add", "origin", str(origin), cwd=root)
    git("push", "-q", "origin", "main", cwd=root)

    git("checkout", "-q", "-b", "issue-1-new-crate", cwd=root)
    if adds_a_crate:
        # The shape of #585: a new member, and nothing at all touching the
        # crates that enumerate the workspace.
        (root / "crates" / "postio-ffi" / "src").mkdir(parents=True)
        (root / "crates" / "postio-ffi" / "src" / "lib.rs").write_text(
            "// a brand new crate\n", encoding="utf-8"
        )
        (root / "Cargo.toml").write_text(
            workspace_toml(MEMBERS + ("postio-ffi",)), encoding="utf-8"
        )
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "add postio-ffi", cwd=root)
    else:
        (root / "crates" / "postio-core" / "src" / "lib.rs").write_text(
            "// an ordinary change\n", encoding="utf-8"
        )
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "touch postio-core", cwd=root)
    return root, stub_dir


def land(root: Path, stub_dir: Path) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment.pop("CARGO_TARGET_DIR", None)
    return subprocess.run(
        ["bash", "scripts/issue-land.sh"],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
        timeout=180,
    )


def workspace_tests(calls: str) -> list[str]:
    """`cargo test --workspace` invocations, which is what #585 asks for.

    Deliberately not counting `cargo check --workspace --all-targets`: that
    one runs on every landing already (#419) and type-checks rather than
    runs, so it cannot catch a test that compiles and fails.
    """
    return [
        line
        for line in calls.splitlines()
        if line.startswith("cargo ")
        and " test " in f" {line} "
        and "--workspace" in line
        # Not the sanity tier. Since #847 every landing runs `cargo test
        # --workspace --lib`, so "a workspace test ran" no longer
        # distinguishes this gate from the default -- and `--lib` would not
        # catch what this gate is for anyway. The thing that broke in #566
        # and #571 was a test that enumerates the workspace, and the next one
        # of those may well be an integration test.
        and "--lib" not in line
    ]


def main() -> int:
    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        base = Path(directory)

        # ── 1. a branch that adds a member runs the workspace tests ───────
        root, stub_dir = world(base / "adds", adds_a_crate=True)
        result = land(root, stub_dir)
        calls = (stub_dir / "calls").read_text(encoding="utf-8")
        found = workspace_tests(calls)
        case(
            "adding a workspace member runs the workspace tests",
            bool(found),
            "a branch that added a crate ran only the per-crate gates, which "
            "is exactly how postio-ui and postio-ffi each landed a red "
            f"postio-session:\n{calls}\n{result.stdout[-2000:]}",
        )

        # ── 2. an ordinary branch is unchanged ───────────────────────────
        #
        # The other half of the acceptance, and the one that keeps this from
        # quietly becoming "test the whole workspace every time" -- which is
        # the thing the per-crate gate exists to avoid.
        root, stub_dir = world(base / "ordinary", adds_a_crate=False)
        result = land(root, stub_dir)
        calls = (stub_dir / "calls").read_text(encoding="utf-8")
        found = workspace_tests(calls)
        case(
            "an ordinary branch still runs only the per-crate tests",
            not found,
            "a branch that added no crate paid for a whole-workspace test "
            f"run:\n{found}",
        )

    for failure in FAILURES:
        print(f"FAIL  {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-land new-crate self-test passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
