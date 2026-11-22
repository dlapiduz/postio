#!/usr/bin/env python3
"""Self-test for issue #2022: what `issue-land.sh` titles the PR.

The PR's title is the squash subject `main` records. It was the newest
commit's subject, so a branch that carries the issue's change followed by a
found-work commit (CLAUDE.md: "fix it now, in its own commit") landed on
`main` under the found work's name: #2019's squash is titled after a skill
tweak, with the change it was for in the body.

The claim's purpose comes first and found work comes later, so the title is
the oldest commit on the branch; on a one-commit branch that is HEAD. When
there are several, the landing prints their subjects, so the session sees
what it is about to call the PR. `--title` still overrides either.

`gh` is stubbed on PATH and every call logged verbatim, so the assertions
read the title `pr create` was actually given. `--no-merge` throughout: the
script stops once the PR is open.

Usage: scripts/tests/test-issue-land-title.py
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

# The shared dial (#1249). `scripts/lib`, not beside this file, because CI
# runs every `scripts/tests/*.py` it finds as a self-test.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
REPO_ROOT = HERE.parent
ISSUE_LAND = HERE / "issue-land.sh"
# Under `target/`, which git ignores (#1225).
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

FAILURES: list[str] = []

# NUL-terminated records: `--body` is multi-line.
GH_STUB = """#!/usr/bin/env bash
if [ "$1" = "--version" ]; then echo "gh version 2.98.0 (2026-01-01)"; exit 0; fi
printf '%s\\0' "$*" >> "$STUB_DIR/calls"
if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
    if printf '%s' "$*" | grep -q -- "--json url"; then
        echo "https://example.com/pull/1"
    fi
    exit 0
fi
exit 0
"""

ISSUE_COMMIT = "feat(dummy): the change the issue asked for"
FOUND_COMMIT = "docs(dummy): a found-work fix"


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
    shutil.copytree(HERE / "lib", scripts / "lib")
    shutil.copy(ISSUE_LAND, scripts / "issue-land.sh")
    (scripts / "issue-land.sh").chmod(0o755)
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


def run_case(name: str, *, commits_before: list[str], extra_args: list[str],
             expected_title: str, expect_listing: bool) -> None:
    """Commit `commits_before` by hand, then land one more commit with -m."""
    prefix = f"[{name}] "
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
        base = Path(directory)
        root = base / "repo"
        origin = base / "origin.git"
        stub_dir = base / "stub"
        (stub_dir / "bin").mkdir(parents=True)
        gh = stub_dir / "bin" / "gh"
        gh.write_text(GH_STUB, encoding="utf-8")
        gh.chmod(0o755)
        (stub_dir / "calls").write_text("", encoding="utf-8")

        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", str(origin)], check=True)
        root.mkdir()
        build_sandbox(root, pinned_channel())
        git("init", "-q", "-b", "main", cwd=root)
        git("config", "user.email", "test@example.com", cwd=root)
        git("config", "user.name", "Test", cwd=root)
        git("add", "-A", cwd=root)
        git("commit", "-q", "-m", "init", cwd=root)
        git("remote", "add", "origin", str(origin), cwd=root)
        git("push", "-q", "origin", "main", cwd=root)
        git("checkout", "-q", "-b", "issue-2022-title-check", cwd=root)
        for number, subject in enumerate(commits_before):
            (root / "dummy" / "src" / f"before{number}.rs").write_text("// x\n", encoding="utf-8")
            git("add", "-A", cwd=root)
            git("commit", "-q", "-m", f"{subject}\n\nRefs: #2022", cwd=root)
        (root / "dummy" / "src" / "last.rs").write_text("// x\n", encoding="utf-8")
        last = FOUND_COMMIT if commits_before else ISSUE_COMMIT

        environment = dict(os.environ)
        environment.pop("RUSTUP_TOOLCHAIN", None)
        environment["CARGO_TARGET_DIR"] = str(base / "target")
        environment["GIT_CONFIG_GLOBAL"] = "/dev/null"
        environment["GIT_CONFIG_SYSTEM"] = "/dev/null"
        environment["PATH"] = f"{stub_dir / 'bin'}:{environment['PATH']}"
        environment["STUB_DIR"] = str(stub_dir)
        result = patience.run(
            ["bash", "scripts/issue-land.sh", "-m", last, "--no-merge", *extra_args],
            cwd=root,
            env=environment,
            capture_output=True,
            text=True,
            timeout=60,
        )
        calls = [r.decode("utf-8") for r in (stub_dir / "calls").read_bytes().split(b"\0") if r]
        output = result.stdout + result.stderr
        if result.returncode != 0:
            FAILURES.append(f"{prefix}the landing failed:\n{output}\n--- gh calls ---\n{calls}")
            print(f"  FAILED: {prefix}the landing failed")
            return

        creates = [c for c in calls if c.startswith("pr create")]
        if len(creates) != 1:
            FAILURES.append(f"{prefix}expected one `pr create`, got {len(creates)}:\n{calls}")
            print(f"  FAILED: {prefix}no single pr create")
            return
        match = re.search(r"--title (.*?) --body", creates[0], re.S)
        title = match.group(1) if match else None
        expect(f"{prefix}the PR is titled {expected_title!r}", title == expected_title,
               f"it was titled {title!r}")

        subjects = commits_before + [last]
        listed = all(re.search(rf"^\s+.*{re.escape(s)}$", output, re.M) for s in subjects)
        if expect_listing:
            expect(f"{prefix}the branch's subjects are printed", listed,
                   f"expected each of {subjects} on its own line in:\n{output}")


def expect(case: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"  ok: {case}")
    else:
        FAILURES.append(f"{case}: {detail}")
        print(f"  FAILED: {case} — {detail}")


def main() -> int:
    print("issue-land PR title self-test")
    run_case("one commit", commits_before=[], extra_args=[],
             expected_title=ISSUE_COMMIT, expect_listing=False)
    run_case("found work after the change", commits_before=[ISSUE_COMMIT], extra_args=[],
             expected_title=ISSUE_COMMIT, expect_listing=True)
    run_case("--title overrides", commits_before=[ISSUE_COMMIT],
             extra_args=["--title", "fix(dummy): what the branch does"],
             expected_title="fix(dummy): what the branch does", expect_listing=False)

    print()
    if FAILURES:
        for failure in FAILURES:
            print(failure, file=sys.stderr)
        print(f"{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("all cases behaved.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
