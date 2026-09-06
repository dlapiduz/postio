#!/usr/bin/env python3
"""Self-test for `scripts/checks/check-test-sandboxes.py` (#1225).

The check reads the other self-tests looking for a sandbox built somewhere
git does not ignore. What it has to get right is both directions: a sandbox in
the repository root is the defect, and one under `target/` is the fix — a
check that only ever passes is not a check.

Usage: scripts/tests/test-check-test-sandboxes.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
REPO_ROOT = HERE.parent
CHECK = HERE / "checks" / "check-test-sandboxes.py"
# Sandboxes go under `target/`, which git ignores: inside the worktree because
# the shared-tree guard only lifts its refusals for worktree paths, and not in
# its root because a killed run leaves the sandbox behind and `git add -A` in a
# worktree will commit it. `scripts/checks/check-test-sandboxes.py` says what
# that cost (#1225).
SANDBOXES = REPO_ROOT / "target" / "tmp"
SANDBOXES.mkdir(parents=True, exist_ok=True)

FAILURES: list[str] = []

# Assembled rather than written out, because the check under test reads other
# files textually and cannot tell a fixture from a sandbox. Spelled literally,
# this string would make *this* file an offender — and marking the line to be
# skipped would hide it from the inner check too, which is the one that is
# supposed to catch it.
BAD_CALL = "dir=" + "REPO_ROOT"

OFFENDING = f'''import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent

def main():
    with tempfile.TemporaryDirectory({BAD_CALL}) as directory:
        print(directory)
'''

GOOD_CALL = "dir=" + "SANDBOXES"

FIXED = f'''import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SANDBOXES = REPO_ROOT / "target" / "tmp"
SANDBOXES.mkdir(parents=True, exist_ok=True)

def main():
    with tempfile.TemporaryDirectory({GOOD_CALL}) as directory:
        print(directory)
'''


def run_against(body: str) -> subprocess.CompletedProcess[str]:
    """The check, over a tree holding one self-test with `body` in it."""
    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
        root = Path(directory)
        (root / "scripts" / "checks").mkdir(parents=True)
        (root / "scripts" / "tests").mkdir(parents=True)
        shutil.copy(CHECK, root / "scripts" / "checks" / CHECK.name)
        (root / "scripts" / "tests" / "test-example.py").write_text(body, encoding="utf-8")
        # The check asks git whether `target/` is ignored before it reads
        # anything, so the sandbox needs to be a repository that ignores it.
        subprocess.run(["git", "init", "-q", "-b", "main", str(root)], check=True)
        (root / ".gitignore").write_text("/target\n", encoding="utf-8")
        return subprocess.run(
            [sys.executable, str(root / "scripts" / "checks" / CHECK.name)],
            capture_output=True,
            text=True,
            timeout=60,
        )


def main() -> int:
    offending = run_against(OFFENDING)
    if offending.returncode == 0:
        FAILURES.append(
            "a sandbox in the repository root passed the check:\n"
            f"{offending.stdout}{offending.stderr}"
        )
    elif "test-example.py" not in offending.stderr:
        FAILURES.append(
            f"the refusal did not name the file:\n{offending.stderr}"
        )
    elif "SANDBOXES" not in offending.stderr:
        FAILURES.append(
            f"the refusal did not say what to write instead:\n{offending.stderr}"
        )

    fixed = run_against(FIXED)
    if fixed.returncode != 0:
        FAILURES.append(
            "a sandbox under target/ was refused, so the fix the check "
            f"recommends does not satisfy it:\n{fixed.stdout}{fixed.stderr}"
        )

    if FAILURES:
        for failure in FAILURES:
            print(failure, file=sys.stderr)
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("check-test-sandboxes self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
