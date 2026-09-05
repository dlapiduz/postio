#!/usr/bin/env python3
"""Every note under docs/notes/ is listed in docs/engineering-notes.md, and
every listing names a note that exists (#1130).

The dated engineering-notes entries are one file each, so that two sessions
writing one down in the same day no longer conflict on every rebase. The
price is an index that can rot in silence, in both directions: a note
nobody listed is a note nobody finds, and a listing whose file was renamed
is a link to nothing. This check refuses both and names the fix.

    python3 scripts/checks/check-notes-index.py            # the repository
    python3 scripts/checks/check-notes-index.py --root DIR  # a fixture (self-test)

Exit status: 0 clean, 1 a problem was found and printed.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

NAME = re.compile(r"^\d{4}-\d{2}-\d{2}-[a-z0-9][a-z0-9-]*\.md$")
LINK = re.compile(r"\]\(notes/([^)]+\.md)\)")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=str(Path(__file__).resolve().parent.parent.parent))
    root = Path(parser.parse_args().root)
    index = root / "docs" / "engineering-notes.md"
    notes_dir = root / "docs" / "notes"
    problems: list[str] = []

    listed = set(LINK.findall(index.read_text(encoding="utf-8"))) if index.exists() else set()
    present = {p.name for p in notes_dir.glob("*.md")} if notes_dir.is_dir() else set()

    for name in sorted(present - listed):
        problems.append(
            f"docs/notes/{name} is not listed in docs/engineering-notes.md -- add a line "
            f"under \"Dated entries\": - <date> — [<title>](notes/{name})"
        )
    for name in sorted(listed - present):
        problems.append(
            f"docs/engineering-notes.md links notes/{name}, which does not exist -- "
            "fix the link or restore the file"
        )
    for name in sorted(present):
        if not NAME.match(name):
            problems.append(
                f"docs/notes/{name} is not named <YYYY-MM-DD>-<slug>.md; the date is how "
                "the index stays in order"
            )
        text = (notes_dir / name).read_text(encoding="utf-8")
        if not text.startswith("# "):
            problems.append(f"docs/notes/{name} has no `# ` title on its first line")

    if problems:
        for problem in problems:
            print(f"FAIL: {problem}")
        return 1
    print(f"notes-index check passed ({len(present)} notes, all listed).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
