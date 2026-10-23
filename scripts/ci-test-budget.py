#!/usr/bin/env python3
"""Every test on the merge path finishes inside its budget.

CLAUDE.md (Tests): no test on the merge path waits on the clock.
`check-no-clock-waits.py` sees a literal `sleep`; it cannot see a product
timer run in real time -- a fetch timeout, a retry window, a debounce -- and
those show only as duration. This reads what nextest recorded for a run and
refuses any case over the tier's budget, naming it.

    scripts/ci-test-budget.py target/nextest/ci/junit.xml 10

The budgets, chosen from the 2026-10-23 data: 10 s for a unit or integration
case, where the slowest ordinary cases are the host's search fixtures at
6-7 s; 20 s for a case that opens a GTK window, where the mean is 4 s and
the slowest kept case 14 s. A case over its budget is not failed for being
slow; it is failed for what slow means here. The remedies, in order: a fake
clock (`#[tokio::test(start_paused = true)]`), a fixture shared across the
binary, or the nightly tier with `POSTIO-MEASUREMENT:` on the case.

Skipped cases are not counted. A failed case is, with its time: a case that
dies at a deadline has spent that deadline.

Exit status: 0 every case inside the budget, 1 otherwise, 2 no report.
"""

from __future__ import annotations

import sys
import xml.etree.ElementTree as ET
from pathlib import Path


def cases(report: Path) -> list[tuple[float, str]]:
    found = []
    for case in ET.parse(report).iter("testcase"):
        if case.find("skipped") is not None:
            continue
        name = f"{case.get('classname', '')} {case.get('name', '')}".strip()
        found.append((float(case.get("time", "0")), name))
    return found


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print(__doc__.strip().splitlines()[0], file=sys.stderr)
        print("usage: ci-test-budget.py <junit.xml> <budget-seconds>", file=sys.stderr)
        return 2
    report, budget = Path(argv[1]), float(argv[2])
    if not report.is_file():
        print(f"test budget: no report at {report}; nextest writes one when "
              f"`[profile.<name>.junit]` names it in .config/nextest.toml", file=sys.stderr)
        return 2
    timed = sorted(cases(report), reverse=True)
    if not timed:
        print(f"test budget: {report} holds no cases")
        return 0
    over = [(t, n) for t, n in timed if t > budget]
    if over:
        print(f"test budget FAILED: {len(over)} case(s) over {budget:g} s on the merge path.\n", file=sys.stderr)
        for t, n in over:
            print(f"  {t:7.1f} s  {n}", file=sys.stderr)
        print(
            "\n  No test on the merge path waits on the clock (CLAUDE.md, Tests). Give the\n"
            "  case a fake clock, share its fixture across the binary, or hold it out:\n"
            "  `/// POSTIO-MEASUREMENT: <why>` above it and a `(package(p) & test(=module::case))`\n"
            "  term in .config/nextest.toml's profile.default default-filter.",
            file=sys.stderr,
        )
        return 1
    slowest, name = timed[0]
    print(f"test budget: {len(timed)} cases inside {budget:g} s; slowest {slowest:.1f} s, {name}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
