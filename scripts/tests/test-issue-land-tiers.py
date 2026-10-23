#!/usr/bin/env python3
"""Self-test for the landing's test tier: which tests a landing runs, and
which it leaves to the pull request.

The default tier has been cut twice. #847 cut it from every changed crate's
integration suites to the workspace's unit tests, when the suites had become
minutes each on a shared workstation. 2026-10-23 cut it again, to the unit
tests of the crates the branch changed, when the pull request's checks came
down from a 35-minute median to about five: a landing waits for those checks
anyway, and they run the workspace's units, every crate's test targets under
clippy, the suites a vocabulary change breaks, and the doctests. What a
landing proves locally is what a red check would cost a round trip to learn.

Three directions are pinned here, and the last matters as much as the first:
the default runs the changed crates' units and their doctests; it runs
neither the integration suites nor the whole workspace; and `--full` still
runs the suites. A "fast" default that had quietly stopped running them under
`--full` too would be indistinguishable from this change working.

**This does not make unit tests sufficient.** CI runs the sanity tier on every
pull request and the full suite on `main` after every merge. That is what
makes leaning on the fast tier safe, because the bugs this project ships are
layers that each pass and are not joined up -- the Reader was built, tested
and never mounted -- and unit tests are exactly the tier that cannot see them.

Usage: scripts/tests/test-issue-land-tiers.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# The shared dial (#1249). `scripts/lib`, not beside this file, because CI
# runs every `scripts/tests/*.py` it finds as a self-test.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
REPO_ROOT = HERE.parent
ISSUE_LAND = HERE / "issue-land.sh"
# Sandboxes go under `target/`, which git ignores: inside the worktree because
# the shared-tree guard only lifts its refusals for worktree paths, and not in
# its root because a killed run leaves the sandbox behind and `git add -A` in a
# worktree will commit it. `scripts/checks/check-test-sandboxes.py` says what
# that cost (#1225).
SANDBOXES = REPO_ROOT / "target" / "tmp"
SANDBOXES.mkdir(parents=True, exist_ok=True)

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

MEMBERS = ("postio-gtk", "postio-core", "postio-widgets")

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


def world(base: Path) -> tuple[Path, Path]:
    """A sandbox whose branch changes one crate, `postio-core`."""
    root = base / "repo"
    stub_dir = base / "stub"
    (stub_dir / "bin").mkdir(parents=True)
    (root / "scripts" / "checks").mkdir(parents=True)

    # The repository's own pin, copied rather than written out: a literal
    # here went stale at the 1.99 bump, and on a runner that has only the
    # pinned toolchain the sandbox then had no rustfmt to run.
    (root / "rust-toolchain.toml").write_text(
        (REPO_ROOT / "rust-toolchain.toml").read_text(encoding="utf-8"), encoding="utf-8"
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

    git("checkout", "-q", "-b", "issue-1-tiers", cwd=root)
    (root / "crates" / "postio-core" / "src" / "lib.rs").write_text(
        "// an ordinary change\n", encoding="utf-8"
    )
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "touch postio-core", cwd=root)
    return root, stub_dir


def land(root: Path, stub_dir: Path, *extra: str) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
    environment["STUB_DIR"] = str(stub_dir)
    environment.pop("CARGO_TARGET_DIR", None)
    return patience.run(
        ["bash", "scripts/issue-land.sh", *extra],
        cwd=root,
        env=environment,
        capture_output=True,
        text=True,
        timeout=180,
    )


def test_runs(calls: str) -> list[str]:
    """Every call that runs tests, under whichever runner the host has:
    `cargo test ...` or `cargo nextest run ...`."""
    return [
        line
        for line in calls.splitlines()
        if line.startswith("cargo ")
        and (" test " in f" {line} " or " nextest run " in f" {line} ")
    ]


def unit_runs(calls: str) -> list[str]:
    """The default tier: the changed crates' units, `--lib -p <crate>`."""
    return [
        line for line in test_runs(calls)
        if " --lib " in f" {line} " and " -p " in f" {line} "
    ]


def suite_runs(calls: str) -> list[str]:
    """The integration suites -- the runs that cost binaries to link: a test
    run that is neither `--lib` nor `--doc`, over a crate or the workspace.

    `--doc` is deliberately not one of them. A doctest run compiles the
    examples in a crate's own docs and links no test binary, which is why
    #1440 put it in the default tier; counting it here would make the tier
    look expensive for something that is nearly free, and would make this
    self-test fail on the very change it is meant to describe.
    """
    return [
        line for line in test_runs(calls)
        if " --lib " not in f" {line} " and " --doc " not in f" {line} "
    ]


def workspace_runs(calls: str) -> list[str]:
    """Any test run over the whole workspace, units included. Those are the
    pull request's since 2026-10-23 (`ci.yml`, Tests)."""
    return [line for line in test_runs(calls) if "--workspace" in line]


def doctest_runs(calls: str) -> list[str]:
    return [line for line in test_runs(calls) if " --doc " in f" {line} "]


def main() -> int:
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
        base = Path(directory)

        # -- 1. the default tier: the changed crate's units, nothing wider --
        root, stub_dir = world(base / "default")
        land(root, stub_dir)
        calls = (stub_dir / "calls").read_text(encoding="utf-8")
        units = unit_runs(calls)
        case(
            "a landing runs the unit tests of the crate it changed",
            any("-p postio-core" in line for line in units),
            f"no `--lib -p postio-core` run (cargo test or nextest run) in:\n{calls}",
        )
        case(
            "and only of the crates it changed",
            not any("postio-gtk" in line or "postio-widgets" in line for line in units),
            f"a crate the branch did not touch had its units run:\n{units}",
        )
        case(
            "and does not run the integration suites",
            not suite_runs(calls),
            "a default landing still paid for the integration binaries, which "
            f"is the cost #847 removed:\n{suite_runs(calls)}",
        )
        case(
            "and does not run the whole workspace",
            not workspace_runs(calls),
            "a default landing ran a workspace-wide tier; that is the pull "
            f"request's job since 2026-10-23:\n{workspace_runs(calls)}",
        )

        # The doctests are in the default tier, and that is the whole of
        # #1440: `--lib` does not run them and does not say so, so a doctest
        # that stopped compiling reached `main` and was found by CI's nightly
        # rather than by the landing that broke it. Asserting only "no
        # integration suites" above would pass just as happily if the
        # doctest run had never been added.
        case(
            "a default landing runs the doctests for a crate it changed",
            any("-p postio-core" in line for line in doctest_runs(calls)),
            f"no `cargo test --doc -p postio-core` in:\n{calls}",
        )

        # -- 2. --full still runs the suites ---------------------------------
        #
        # The half that keeps the first half honest: a fast default that had
        # also broken `--full` would pass every assertion above.
        root, stub_dir = world(base / "full")
        land(root, stub_dir, "--full")
        calls = (stub_dir / "calls").read_text(encoding="utf-8")
        case(
            "--full runs the changed crate's integration suites",
            any("-p postio-core" in line for line in suite_runs(calls)),
            f"`--full` ran no per-crate suites at all:\n{calls}",
        )

    for failure in FAILURES:
        print(f"FAIL  {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("issue-land tiers self-test passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
