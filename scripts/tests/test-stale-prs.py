#!/usr/bin/env python3
"""Self-test for scripts/stale-prs.py.

The script says which open pull requests are stuck and why: conflicting with
main, red, or waiting with no auto-merge armed, or quiet for days. The
reasons are a pure function of what `gh pr list` returns, so this proves
them against fixtures.

Usage: scripts/tests/test-stale-prs.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import importlib.util
import sys
from datetime import datetime, timezone
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "stale-prs.py"
spec = importlib.util.spec_from_file_location("stale_prs", SCRIPT)
assert spec and spec.loader
stale = importlib.util.module_from_spec(spec)
sys.modules["stale_prs"] = stale
spec.loader.exec_module(stale)

FAILURES: list[str] = []
NOW = datetime(2026, 10, 9, 12, tzinfo=timezone.utc)


def case(name: str, condition: bool, detail: object = "") -> None:
    print(f"{'ok   ' if condition else 'FAIL '} {name}")
    if not condition:
        FAILURES.append(f"{name}: {detail}")


def pr(**fields):
    base = {
        "number": 1,
        "title": "t",
        "isDraft": False,
        "mergeable": "MERGEABLE",
        "updatedAt": "2026-10-09T10:00:00Z",
        "autoMergeRequest": {"mergeMethod": "SQUASH"},
        "statusCheckRollup": [{"conclusion": "SUCCESS", "status": "COMPLETED"}],
    }
    base.update(fields)
    return base


def main() -> int:
    case("a fresh green PR with auto-merge is not stuck", stale.reasons(pr(), NOW) == [])
    case(
        "a conflicting PR needs a rebase",
        any("rebase" in r for r in stale.reasons(pr(mergeable="CONFLICTING"), NOW)),
        stale.reasons(pr(mergeable="CONFLICTING"), NOW),
    )
    red = pr(statusCheckRollup=[{"conclusion": "FAILURE", "status": "COMPLETED", "name": "Tests"}])
    case(
        "a red PR names the failing check",
        any("Tests" in r for r in stale.reasons(red, NOW)),
        stale.reasons(red, NOW),
    )
    waiting = pr(autoMergeRequest=None)
    case(
        "a green PR with no auto-merge is waiting on someone",
        any("auto-merge" in r for r in stale.reasons(waiting, NOW)),
        stale.reasons(waiting, NOW),
    )
    old = pr(updatedAt="2026-10-05T10:00:00Z")
    case(
        "a PR quiet for days says how long",
        any("4 days" in r for r in stale.reasons(old, NOW)),
        stale.reasons(old, NOW),
    )
    case(
        "a running check is not a failure",
        stale.reasons(pr(statusCheckRollup=[{"conclusion": None, "status": "IN_PROGRESS"}]), NOW) == [],
    )
    case("a draft is left alone", stale.reasons(pr(isDraft=True, mergeable="CONFLICTING"), NOW) == [])

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("stale-prs self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
