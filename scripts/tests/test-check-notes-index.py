#!/usr/bin/env python3
"""Self-test for scripts/checks/check-notes-index.py (#1130).

The dated engineering-notes entries live one per file under `docs/notes/`,
and `docs/engineering-notes.md` lists them. Two ways for that to rot, both
silent: a note nobody listed (written, never found) and a listing that
names a file that is not there (a rename, a typo). The check refuses both
and names the fix. It runs against a fixture tree, not the real one, so it
proves the direction the check fails in rather than that today's tree
happens to be tidy.

Usage: scripts/tests/test-check-notes-index.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

CHECK = Path(__file__).resolve().parent.parent / "checks" / "check-notes-index.py"
FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    print(f"{'ok   ' if condition else 'FAIL '} {name}")
    if not condition:
        FAILURES.append(f"{name}: {detail}")


def run(root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["python3", str(CHECK), "--root", str(root)],
        capture_output=True, text=True, timeout=30,
    )


def tree(root: Path, index_lines: list[str], notes: dict[str, str]) -> None:
    (root / "docs" / "notes").mkdir(parents=True, exist_ok=True)
    (root / "docs" / "engineering-notes.md").write_text(
        "# Notes\n\n## Dated entries, one file each\n\n" + "\n".join(index_lines) + "\n",
        encoding="utf-8",
    )
    for name, body in notes.items():
        (root / "docs" / "notes" / name).write_text(body, encoding="utf-8")


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw)
        good = "2026-09-04-a-thing-that-happened.md"
        tree(root, [f"- 2026-09-04 — [A thing that happened](notes/{good})"],
             {good: "# A thing that happened\n\nbody\n"})
        r = run(root)
        case("a listed note with a title passes", r.returncode == 0, r.stdout + r.stderr)

        (root / "docs" / "notes" / "2026-09-05-unlisted.md").write_text("# Unlisted\n\nx\n", encoding="utf-8")
        r = run(root)
        case("an unlisted note fails", r.returncode != 0, "passed with a note missing from the index")
        case("...and is named", "2026-09-05-unlisted.md" in r.stdout + r.stderr, r.stdout + r.stderr)
        (root / "docs" / "notes" / "2026-09-05-unlisted.md").unlink()

        tree(root, [f"- 2026-09-04 — [A thing that happened](notes/{good})",
                    "- 2026-09-06 — [Gone](notes/2026-09-06-gone.md)"],
             {good: "# A thing that happened\n\nbody\n"})
        r = run(root)
        case("a listing that names no file fails", r.returncode != 0, "passed with a dangling index line")
        case("...and is named", "2026-09-06-gone.md" in r.stdout + r.stderr, r.stdout + r.stderr)

        tree(root, [f"- 2026-09-04 — [A thing that happened](notes/{good})"],
             {good: "no heading here\n"})
        r = run(root)
        case("a note without a title fails", r.returncode != 0, "passed a note with no `# ` title")

        tree(root, [f"- 2026-09-04 — [A thing that happened](notes/{good})"],
             {good: "# A thing that happened\n", "not-dated.md": "# x\n"})
        r = run(root)
        case("a note not named by date fails", r.returncode != 0, "passed a note whose name has no date")

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("check-notes-index self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
