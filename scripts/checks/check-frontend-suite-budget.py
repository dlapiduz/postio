#!/usr/bin/env python3
"""A frontend suite that has been reviewed does not grow by accident.

ADR 0049: a behaviour's branches are asserted once, at the cheapest layer
that can see them, and a frontend suite asserts composition, one case per
path a person takes on that platform. Spec 012 read every frontend case in a
parity cluster against that rule: 288 cases in 53 files. It kept 205,
proved 16 redundant by mutation and removed them, and left the rest with
their reasons (specs/012-test-suite-value/parity-review.tsv).

Without a check, the next feature re-creates the duplication: the cheapest
place to write a test is beside the last one. So each reviewed file has a
budget, its case count, in scripts/checks/frontend-suite-budget.txt, and a
change that grows a file past it fails here and names where the case
belongs instead. A new composition case -- a path a person takes that no
case in that suite drives -- is welcome: raise the budget in the same
commit and say which path in the body. A file that shrinks passes, and the
check prints the line to lower so the ratchet follows it down.

What a case is: a `#[test]` or `#[tokio::test(...)]` function, or, in a
harness-free GTK or widgets suite, a `pub fn` named in that suite's `CASES`
table in main.rs. Files not in the baseline are not judged.

Usage: scripts/checks/check-frontend-suite-budget.py [--root DIR]
Exit status: 0 within every budget; 1 a file grew past its budget or a listed
file is gone.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BASELINE = Path("scripts/checks/frontend-suite-budget.txt")
TEST_ATTR = re.compile(r"#\[(?:tokio::)?test\b")
PUB_FN = re.compile(r"^pub(?:\(crate\))?\s+(?:async\s+)?fn\s+(\w+)", re.M)


def cases_in(path: Path) -> int:
    text = path.read_text()
    main = path.parent / "main.rs"
    if path.name != "main.rs" and main.exists() and "const CASES" in main.read_text():
        table = main.read_text()
        module = path.stem if path.stem != "mod" else path.parent.name
        return sum(1 for name in PUB_FN.findall(text) if f'"{module}::{name}"' in table)
    return len(TEST_ATTR.findall(text))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--root", default=str(ROOT))
    root = Path(parser.parse_args().root)
    budgets: dict[str, int] = {}
    for line in (root / BASELINE).read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        file, count = line.rsplit(" ", 1)
        budgets[file] = int(count)

    failures, lower = [], []
    for file, budget in sorted(budgets.items()):
        path = root / file
        if not path.exists():
            failures.append(f"{file}: listed but missing. Remove its line from {BASELINE}.")
            continue
        found = cases_in(path)
        if found > budget:
            failures.append(
                f"{file}: {found} cases, budget {budget}.\n"
                "  ADR 0049: a branch belongs in the controller suite "
                "(crates/postio-focus/tests/controller_suite/) or a rule crate's. If this case is\n"
                "  composition -- a path a person takes that no case here drives -- raise the budget in\n"
                f"  {BASELINE} in this commit and name the path in the body."
            )
        elif found < budget:
            lower.append(f"{file} {found}")

    if lower:
        print(f"Fewer cases than budgeted; lower these lines in {BASELINE}:")
        for line in lower:
            print(f"  {line}")
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(f"frontend-suite budget check passed ({len(budgets)} reviewed files).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
