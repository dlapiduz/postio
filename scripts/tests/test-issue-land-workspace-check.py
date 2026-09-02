#!/usr/bin/env python3
"""Self-test for #419: the gate compiles every crate's *test* targets.

`main` went red twice in one day, both times invisibly to the process that
admitted it, and both times the same shape: a shared type gained a field and
another crate's **test targets** stopped compiling. `Event::BackfillProgress`
gained `footprint`; six call sites in `postio-gtk`'s tests still built it by
literal. The libraries compiled, so `cargo build` was green and
`cargo check -p postio-core` was green.

The gate could not have caught it. It runs clippy and tests over *the crates
you changed*, and whoever added the field changed `postio-core`, not
`postio-gtk` — so nothing in their chain had any reason to compile
`postio-gtk`'s tests. Green meant "the things I named still work"; the things
nobody named were checked by luck, and the next session to touch that crate
paid for it.

So the chain also type-checks the whole workspace, test targets included.
`cargo check`, not `build` or `test`: no codegen, no linking, no execution —
the cheapest thing that can answer "does everyone's code still compile against
what I just changed", which is exactly the blast radius a changed-crate list
does not describe.

Asserting on the recorded `cargo` invocations rather than on the banner: a
banner can claim a check that never ran, and #419's whole complaint is about
gates that look like they cover something and do not.

Usage: scripts/tests/test-issue-land-workspace-check.py
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

# Records every invocation, and fails the ones the case asks it to: a gate
# that runs a check and ignores its verdict is the same bug one layer along.
CARGO_STUB = """#!/usr/bin/env bash
printf 'cargo %s\\n' "$*" >> "$STUB_DIR/calls"
if [ -f "$STUB_DIR/fail-workspace-check" ] \\
   && printf '%s' "$*" | grep -q -- "--workspace" \\
   && printf '%s' "$*" | grep -q -- "--all-targets"; then
    echo "error[E0063]: missing field \\`footprint\\` in initializer" >&2
    exit 101
fi
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


def world(base: Path, *, fail_workspace_check: bool) -> tuple[Path, Path]:
    """A sandbox whose branch changes one crate that others depend on."""
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
    if fail_workspace_check:
        (stub_dir / "fail-workspace-check").write_text("", encoding="utf-8")

    git("init", "-q", "-b", "main", cwd=root)
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "init", cwd=root)
    origin = base / "origin.git"
    subprocess.run(["git", "init", "-q", "--bare", str(origin)], check=True)
    git("remote", "add", "origin", str(origin), cwd=root)
    git("push", "-q", "origin", "main", cwd=root)

    # The shape of the bug: a change to a crate *other* crates' tests build
    # against. Nothing here touches `postio-gtk`, whose tests broke.
    git("checkout", "-q", "-b", "issue-1-workspace-check", cwd=root)
    (root / "crates" / "postio-core" / "src" / "lib.rs").write_text(
        "// a shared type gains a field\n", encoding="utf-8"
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


def workspace_checks(calls: str) -> list[str]:
    return [
        line
        for line in calls.splitlines()
        if line.startswith("cargo ")
        and "--workspace" in line
        and "--all-targets" in line
    ]


def main() -> int:
    # 1. The check runs at all, over the workspace and its test targets.
    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        root, stub_dir = world(Path(directory), fail_workspace_check=False)
        result = land(root, stub_dir)
        out = result.stdout + result.stderr
        calls = (stub_dir / "calls").read_text(encoding="utf-8")
        found = workspace_checks(calls)

        case(
            "the gate type-checks the whole workspace's test targets",
            bool(found),
            "no `cargo check --workspace --all-targets` was run, so a crate "
            "nobody named can still stop compiling unnoticed. Calls were:\n"
            f"{calls}",
        )
        case(
            "it is a check, not a build or a test run",
            all(line.split()[1] == "check" for line in found),
            f"the workspace pass should be `cargo check`: {found}",
        )
        case(
            "the branch still lands when everything compiles",
            result.returncode == 0,
            f"exit {result.returncode}; output:\n{out}",
        )

    # 2. And its verdict is acted on. A gate that runs a check and ignores
    #    the answer is #419 again, one layer along.
    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        root, stub_dir = world(Path(directory), fail_workspace_check=True)
        result = land(root, stub_dir)
        out = result.stdout + result.stderr
        calls = (stub_dir / "calls").read_text(encoding="utf-8")

        case(
            "a workspace test target that stops compiling stops the landing",
            result.returncode != 0,
            f"exit {result.returncode}; output:\n{out}",
        )
        case(
            "nothing was pushed",
            "pr create" not in calls,
            f"a PR was opened despite the broken test target:\n{calls}",
        )

    for failure in FAILURES:
        print(f"FAIL  {failure}")
    return 1 if FAILURES else 0


if __name__ == "__main__":
    sys.exit(main())
