#!/usr/bin/env python3
"""Which open pull requests are stuck, and on what.

A pull request goes stale quietly: main moves and it conflicts, a check goes
red and nobody's next claim is on that branch, or it is green and waiting
for a merge nobody armed. #1781 and #1782 sat for days on CI results that
were already wrong -- #1782 long enough that it had to be ported rather
than rebased. This lists each stuck PR with its reasons, for /steward's
pass or anyone about to wonder why main is not moving.

    scripts/stale-prs.py            # the open, non-draft PRs that are stuck
    scripts/stale-prs.py --days 2   # call a PR quiet after two days (default 3)

Exit status: 0 whatever it finds; it reports, the steward acts.
"""

from __future__ import annotations

import argparse
import json
import subprocess
from datetime import datetime, timezone

QUIET_DAYS = 3
FIELDS = "number,title,isDraft,mergeable,updatedAt,autoMergeRequest,statusCheckRollup,headRefName"


def reasons(pr: dict, now: datetime, quiet_days: int = QUIET_DAYS) -> list[str]:
    """Why `pr` is stuck; empty when it is not."""
    if pr.get("isDraft"):
        return []
    found: list[str] = []
    if pr.get("mergeable") == "CONFLICTING":
        found.append("conflicts with main: rebase it")
    checks = pr.get("statusCheckRollup") or []
    failed = [
        c.get("name") or c.get("context") or "a check"
        for c in checks
        if (c.get("conclusion") or c.get("state")) in ("FAILURE", "ERROR", "TIMED_OUT", "CANCELLED")
    ]
    if failed:
        found.append("red: " + ", ".join(sorted(set(failed))))
    running = any(c.get("status") not in (None, "COMPLETED") for c in checks)
    if not failed and not running and not pr.get("autoMergeRequest") and pr.get("mergeable") != "CONFLICTING":
        found.append("green and waiting: no auto-merge armed (`gh pr merge --auto --squash`)")
    updated = datetime.fromisoformat(pr["updatedAt"].replace("Z", "+00:00"))
    days = (now - updated).days
    if days >= quiet_days:
        found.append(f"quiet for {days} days")
    return found


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--days", type=int, default=QUIET_DAYS)
    arguments = parser.parse_args()
    prs = json.loads(
        subprocess.run(
            ["gh", "pr", "list", "--state", "open", "--limit", "100", "--json", FIELDS],
            check=True, capture_output=True, text=True,
        ).stdout
    )
    now = datetime.now(timezone.utc)
    stuck = [(pr, reasons(pr, now, arguments.days)) for pr in prs]
    stuck = [(pr, why) for pr, why in stuck if why]
    if not stuck:
        print(f"no stuck pull requests ({len(prs)} open).")
        return 0
    for pr, why in sorted(stuck, key=lambda item: item[0]["number"]):
        print(f"#{pr['number']} {pr['title']}  [{pr['headRefName']}]")
        for reason in why:
            print(f"    - {reason}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
