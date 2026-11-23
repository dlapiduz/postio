#!/usr/bin/env python3
"""Self-test for issue-land.sh's log rotation (#710).

A gate failure's diagnosis often lives in the *whole* run's output rather
than in the failing test's own block -- a C library printing straight to
stderr as `fprintf`, for one, is output `cargo test`'s per-test capture
never holds (SQLCipher was the live example when this was written).

The obvious response to a flaked landing is to land again, and that used to
truncate the log -- so the evidence survived exactly as long as it took to
notice it was needed. Two occurrences of #710 in one session were destroyed
that way, by the session that had just been asked to keep them.

So a detach moves the previous log aside before starting a new one, one
deep.

This reads the script rather than running it: the behaviour lives in the
`--detach` arm, and exercising that arm means actually landing something.
What it can still hold is the shape the rotation has to have -- before the
truncation, one deep, tolerant of there being nothing to move, and named by
`--status` so somebody is told where the evidence went.

Usage: scripts/tests/test-issue-land-log-rotation.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "issue-land.sh"
FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    print(f"{'ok   ' if condition else 'FAIL '} {name}")
    if not condition:
        FAILURES.append(f"{name}: {detail}")


source = SCRIPT.read_text()

case(
    "the previous log is named",
    '"$LAND_LOG.1"' in source,
    "issue-land.sh never names a `.1` log, so nothing is kept",
)

truncate = source.find(': > "$LAND_LOG"')
rotate = source.find('"$LAND_LOG.1"')
case(
    "the log is still emptied for a fresh run",
    truncate != -1,
    "the truncation is gone; a new landing would append to the old one",
)
case(
    "rotation happens before the truncation",
    rotate != -1 and rotate < truncate,
    f"rotate at {rotate}, truncate at {truncate} -- the move must come first "
    "or it moves an already-emptied file",
)

case(
    "only one previous log is kept",
    ".log.2" not in source,
    "a second level of rotation would accumulate in every worktree",
)

status_block = source[source.find('*" --status "*') : source.find('*" --detach "*')]
case(
    "--status points at the kept log",
    "LAND_LOG.1" in status_block or "log.1" in status_block,
    "--status never mentions the previous log, so a flake's evidence is kept "
    "somewhere nobody is told about",
)

rotation_line = ""
for line in source.splitlines():
    if '"$LAND_LOG.1"' in line and "mv" in line:
        rotation_line = line
        break
case(
    "rotating tolerates having nothing to rotate",
    bool(rotation_line) and re.search(r"2>\s*/dev/null|\|\|\s*true|-f ", rotation_line) is not None,
    f"the move is unguarded ({rotation_line.strip()!r}); the first landing in "
    "a worktree has no previous log and must not fail on it",
)

if FAILURES:
    print("\nissue-land log rotation self-test FAILED", file=sys.stderr)
    for failure in FAILURES:
        print(f"  {failure}", file=sys.stderr)
    sys.exit(1)
print(f"\nissue-land log rotation: {6 - len(FAILURES)} cases passed")
