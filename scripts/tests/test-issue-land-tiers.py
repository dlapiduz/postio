#!/usr/bin/env python3
"""Self-test for the landing's gates: what a landing checks locally, and what
it leaves to the pull request.

A landing costs at most five minutes of the agent's time (CLAUDE.md), so it
runs rustfmt, the invariants and one clippy call over the changed crates'
targets, which type-checks their tests too. Tests, doctests and rustdoc are
the pull request's (`ci.yml`: Tests, Rustdoc links); the full suite and the
frontends run on `main` after the merge.

Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
REPO_ROOT = HERE.parent
ISSUE_LAND = HERE / "issue-land.sh"
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
    """A sandbox whose branch changes `postio-core` and `postio-widgets`."""
    root = base / "repo"
    stub_dir = base / "stub"
    (stub_dir / "bin").mkdir(parents=True)
    (root / "scripts" / "checks").mkdir(parents=True)

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
    (root / "Cargo.lock").write_text("# stub\n", encoding="utf-8")
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "init", cwd=root)
    origin = base / "origin.git"
    subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
    git("remote", "add", "origin", str(origin), cwd=root)
    git("push", "-q", "origin", "main", cwd=root)

    git("checkout", "-q", "-b", "issue-1-tiers", cwd=root)
    for crate in ("postio-core", "postio-widgets"):
        (root / "crates" / crate / "src" / "lib.rs").write_text(
            "// an ordinary change\n", encoding="utf-8"
        )
    git("add", "-A", cwd=root)
    git("commit", "-q", "-m", "touch postio-core and postio-widgets", cwd=root)
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


def cargo_runs(calls: str, *verbs: str) -> list[str]:
    return [
        line for line in calls.splitlines()
        if line.startswith("cargo ") and any(f" {verb} " in f" {line} " for verb in verbs)
    ]


def main() -> int:
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
        base = Path(directory)

        root, stub_dir = world(base / "default")
        result = land(root, stub_dir)
        calls = (stub_dir / "calls").read_text(encoding="utf-8")
        clippy = cargo_runs(calls, "clippy")
        case(
            "a landing runs one clippy call",
            len(clippy) == 1,
            f"exit {result.returncode}, clippy calls {clippy} in:\n{calls}",
        )
        call = clippy[0] if clippy else ""
        case(
            "over every crate it changed, their tests included, warnings denied",
            "-p postio-core" in call and "-p postio-widgets" in call
            and "--all-targets" in call and "-D warnings" in call,
            call,
        )
        case(
            "and no crate it left alone",
            "postio-gtk" not in call,
            call,
        )
        case(
            "a landing runs no tests, doctests or rustdoc; the pull request does",
            not cargo_runs(calls, "test", "nextest", "doc"),
            f"{cargo_runs(calls, 'test', 'nextest', 'doc')}",
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
