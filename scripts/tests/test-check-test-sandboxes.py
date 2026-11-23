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

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
REPO_ROOT = HERE.parent
CHECK = HERE / "checks" / "check-test-sandboxes.py"
SANDBOXES = REPO_ROOT / "target" / "tmp"
SANDBOXES.mkdir(parents=True, exist_ok=True)

FAILURES: list[str] = []

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
        subprocess.run(["git", "init", "-q", "-b", "main", str(root)], check=True)
        (root / ".gitignore").write_text("/target\n", encoding="utf-8")
        return patience.run(
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
