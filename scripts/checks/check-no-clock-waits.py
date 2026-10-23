#!/usr/bin/env python3
"""No test on the merge path waits on the clock.

A `sleep` of a second or more in a test is a wait every pull request and
every landing pays, and under a loaded runner it is the flake: the host's two
cases of 2026-10-22 waited 5 and 15 s against fixed deadlines and failed one
run in three. CLAUDE.md (Tests) says such a case gets a fake clock, or the
nightly tier. This is what holds that true.

What it reads: every `sleep(` whose argument is a literal of a second or
more, on one line, in `crates/*/tests/**/*.rs` and in `crates/*/src/**/*.rs`
files that hold tests -- including a literal wrapped in
`postio_test_support::scaled(...)`, which scales a wait and does not remove
it. A hit is allowed when:

  * the enclosing test runs on a fake clock, `#[tokio::test(start_paused =
    true)]`, so the sleep costs nothing;
  * the enclosing case, or its file, carries `POSTIO-MEASUREMENT:`, so the
    filter in `.config/nextest.toml` already holds it off the merge path
    (`check-measurement-tier.py` keeps the marker and the filter in step);
  * the line above says `// POSTIO-NOT-A-WAIT: <why>`: the sleep is the
    subject of the test rather than a wait in it, such as a stuck task the
    test must return without.

A product timer run in real time -- a fetch timeout, a retry window -- shows
no `sleep` and is not seen here; the per-test time budget on CI is what
catches it.

Usage: scripts/checks/check-no-clock-waits.py
Exit status: 0 clean, 1 a wait this names.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MARKER = "POSTIO-MEASUREMENT:"
WAIVER = "POSTIO-NOT-A-WAIT:"

SLEEP = re.compile(r"\bsleep(?:_for)?\s*\(")
SECS = re.compile(r"from_secs\s*\(\s*(\d+)")
MILLIS = re.compile(r"from_millis\s*\(\s*(\d+)")
SECS_F = re.compile(r"from_secs_f(?:32|64)\s*\(\s*([\d.]+)")
FN = re.compile(r"^\s*(?:pub(?:\(crate\))?\s+)?(?:async\s+)?fn\s+\w+")
HOLDS_TESTS = re.compile(r"#\[(?:tokio::)?test|#\[cfg\(test\)\]")


def seconds_on(line: str) -> float:
    """The largest literal duration on `line`, in seconds, or 0."""
    found = [float(s) for s in SECS.findall(line)]
    found += [float(ms) / 1000 for ms in MILLIS.findall(line)]
    found += [float(s) for s in SECS_F.findall(line)]
    return max(found, default=0.0)


def test_sources() -> list[Path]:
    sources = sorted(ROOT.glob("crates/*/tests/**/*.rs"))
    for source in sorted(ROOT.glob("crates/*/src/**/*.rs")):
        if HOLDS_TESTS.search(source.read_text()):
            sources.append(source)
    return sources


def allowed(lines: list[str], index: int, file_marked: bool) -> bool:
    if file_marked:
        return True
    if index > 0 and WAIVER in lines[index - 1]:
        return True
    i = index
    while i >= 0 and not FN.match(lines[i]):
        i -= 1
    i -= 1
    while i >= 0 and lines[i].lstrip().startswith(("///", "//", "#[")):
        if MARKER in lines[i] or "start_paused" in lines[i]:
            return True
        i -= 1
    return False


def main() -> int:
    problems: list[str] = []
    for source in test_sources():
        text = source.read_text()
        lines = text.split("\n")
        file_marked = any(MARKER in line and line.lstrip().startswith("//!") for line in lines)
        for index, line in enumerate(lines):
            if not SLEEP.search(line) or seconds_on(line) < 1.0:
                continue
            if allowed(lines, index, file_marked):
                continue
            problems.append(f"{source.relative_to(ROOT)}:{index + 1}: {line.strip()}")
    if problems:
        print("no-clock-waits check FAILED: a test on the merge path waits on the clock.\n", file=sys.stderr)
        for problem in problems:
            print(f"  {problem}", file=sys.stderr)
        print(
            "\n  Give the case a fake clock (`#[tokio::test(start_paused = true)]`), or hold\n"
            "  it off the merge path: `/// POSTIO-MEASUREMENT: <why>` above the case and a\n"
            "  `(package(p) & test(=module::case))` term in .config/nextest.toml's\n"
            "  profile.default default-filter. A sleep that is the test's subject rather\n"
            "  than a wait takes `// POSTIO-NOT-A-WAIT: <why>` on the line above.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
