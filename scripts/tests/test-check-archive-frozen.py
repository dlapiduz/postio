#!/usr/bin/env python3
"""Self-test for scripts/checks/check-archive-frozen.py.

docs/archive/ holds what no longer describes the system. A change may move
a document into it, take one out, or update its README; it may not edit an
archived document, which is what a rebase does silently when a branch edits
a file that main has since moved there (git follows the rename). The check
compares the branch with main; this builds throwaway repositories to prove
it refuses the edit and allows the rest.

Usage: scripts/tests/test-check-archive-frozen.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

CHECK = Path(__file__).resolve().parent.parent / "checks" / "check-archive-frozen.py"
FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str = "") -> None:
    print(f"{'ok   ' if condition else 'FAIL '} {name}")
    if not condition:
        FAILURES.append(f"{name}: {detail}")


def git(root: Path, *args: str) -> None:
    subprocess.run(["git", *args], cwd=root, check=True, capture_output=True)


def repository(root: Path) -> None:
    git(root, "init", "-q", "-b", "main")
    git(root, "config", "user.email", "test@example.test")
    git(root, "config", "user.name", "Test")
    (root / "docs" / "archive").mkdir(parents=True)
    (root / "docs" / "notes").mkdir(parents=True)
    (root / "docs" / "archive" / "README.md").write_text("# Archive\n")
    (root / "docs" / "archive" / "old.md").write_text("# Old\n\nkept as it was\n")
    (root / "docs" / "notes" / "live.md").write_text("# Live\n\nstill true\n")
    git(root, "add", "-A")
    git(root, "commit", "-q", "-m", "base")
    git(root, "update-ref", "refs/remotes/origin/main", "HEAD")


def run(root: Path) -> subprocess.CompletedProcess[str]:
    environment = {k: v for k, v in os.environ.items() if not k.startswith("GITHUB_")}
    return subprocess.run(
        ["python3", str(CHECK), "--root", str(root)],
        capture_output=True, text=True, env=environment,
    )


def scenario(change) -> subprocess.CompletedProcess[str]:
    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw)
        repository(root)
        change(root)
        return run(root)


def main() -> int:
    r = scenario(lambda root: None)
    case("an untouched archive passes", r.returncode == 0, r.stdout + r.stderr)

    def edit(root: Path) -> None:
        (root / "docs" / "archive" / "old.md").write_text("# Old\n\nkept as it was\n- a new line\n")
    r = scenario(edit)
    case("an edit to an archived document fails", r.returncode != 0, r.stdout + r.stderr)
    case("...and names it", "docs/archive/old.md" in r.stdout + r.stderr, r.stdout + r.stderr)

    def committed_edit(root: Path) -> None:
        edit(root)
        git(root, "commit", "-qam", "edit")
    r = scenario(committed_edit)
    case("a committed edit fails too", r.returncode != 0, r.stdout + r.stderr)

    def move_in(root: Path) -> None:
        git(root, "mv", "docs/notes/live.md", "docs/archive/live.md")
        git(root, "commit", "-qm", "archive it")
    r = scenario(move_in)
    case("moving a document into the archive passes", r.returncode == 0, r.stdout + r.stderr)

    def readme(root: Path) -> None:
        (root / "docs" / "archive" / "README.md").write_text("# Archive\n\n- old.md\n")
    r = scenario(readme)
    case("the archive's README may change", r.returncode == 0, r.stdout + r.stderr)

    def remove(root: Path) -> None:
        git(root, "rm", "-q", "docs/archive/old.md")
    r = scenario(remove)
    case("deleting an archived document passes", r.returncode == 0, r.stdout + r.stderr)

    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw)
        git(root, "init", "-q", "-b", "main")
        (root / "docs" / "archive").mkdir(parents=True)
        r = run(root)
        case("with no main to compare against it skips, saying so",
             r.returncode == 0 and "skip" in (r.stdout + r.stderr).lower(), r.stdout + r.stderr)

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("check-archive-frozen self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
