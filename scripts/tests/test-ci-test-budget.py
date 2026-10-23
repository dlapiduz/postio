#!/usr/bin/env python3
"""Prove `ci-test-budget.py` refuses a case over budget and nothing else.

The shape it exists for: a case that waits on a product timer, which no
static check can see and which shows only as its duration in nextest's
report. A skipped case has no duration to judge, and a missing report is a
configuration error, not a pass.

A guard that has never been seen to fail is a guard nobody should trust.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

# The shared dial (#1249). `scripts/lib`, not beside this file, because CI
# runs every `scripts/tests/*.py` it finds as a self-test.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "ci-test-budget.py"

REPORT = """\
<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="4" failures="1">
  <testsuite name="postio-host" tests="4">
    <testcase name="tests::quick" classname="postio-host" time="0.031"/>
    <testcase name="tests::waits_on_a_timer" classname="postio-host" time="{slow}"/>
    <testcase name="tests::died_at_its_deadline" classname="postio-host" time="{dead}">
      <failure message="the event arrived: Elapsed"/>
    </testcase>
    <testcase name="tests::held_out" classname="postio-host" time="0">
      <skipped/>
    </testcase>
  </testsuite>
</testsuites>
"""


def run(report_text: str, budget: str, where: Path) -> subprocess.CompletedProcess[str]:
    report = where / "junit.xml"
    report.write_text(report_text)
    return patience.run([sys.executable, str(SCRIPT), str(report), budget],
                          capture_output=True, text=True, timeout=60)


def main() -> int:
    failures = 0

    def case(name: str, ok: bool, detail: str) -> None:
        nonlocal failures
        print(f"{'ok   ' if ok else 'FAIL '} {name}")
        if not ok:
            failures += 1
            print(f"       {detail}", file=sys.stderr)

    with tempfile.TemporaryDirectory() as raw:
        tmp = Path(raw)
        r = run(REPORT.format(slow="14.2", dead="20.0"), "10", tmp)
        case("a case over budget fails and is named",
             r.returncode == 1 and "14.2 s  postio-host tests::waits_on_a_timer" in r.stderr,
             f"exit {r.returncode}: {r.stderr[:200]}")
        case("a case that died at its deadline is counted with its time",
             "20.0 s  postio-host tests::died_at_its_deadline" in r.stderr, r.stderr[:200])
        r = run(REPORT.format(slow="4.0", dead="2.0"), "10", tmp)
        case("every case inside the budget passes and the slowest is reported",
             r.returncode == 0 and "slowest 4.0 s" in r.stdout, f"exit {r.returncode}: {r.stdout} {r.stderr[:120]}")
        r = run(REPORT.format(slow="0.1", dead="0.1"), "0.05", tmp)
        case("a skipped case is never judged",
             "held_out" not in r.stderr, r.stderr[:200])
        r = patience.run([sys.executable, str(SCRIPT), str(tmp / "missing.xml"), "10"],
                           capture_output=True, text=True, timeout=60)
        case("a missing report is a configuration error, not a pass",
             r.returncode == 2 and "no report" in r.stderr, f"exit {r.returncode}")

    if failures:
        print(f"\n{failures} case(s) failed.", file=sys.stderr)
        return 1
    print("ci-test-budget self-test passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
