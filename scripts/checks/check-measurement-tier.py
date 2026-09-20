#!/usr/bin/env python3
"""Keep the measurement tier's filter and its markers from drifting apart.

`.config/nextest.toml`'s `profile.default` holds a handful of tests off the
merge path because what they produce is numbers a person reads rather than an
assertion (#1450). Two things have to stay true about that set, and neither is
true by construction:

* **Every test the filter excludes says so in its own docs.** A name in a TOML
  filter is invisible from the file it silences. Without this, a test stops
  running and the only record is a line in a config nobody opens.
* **Every test that says it is a measurement is actually excluded.** This is
  the direction that matters: a marker is a claim about when something runs,
  and a claim nothing enforces is how `header_block_size` and its five
  neighbours came to be `#[ignore]`d for a tier that did not exist.

And one rule the tier must never break:

* **A measurement is never also `#[ignore]`d.** Conflating the two is the bug
  #1450 fixed. `#[ignore]` means *this machine may not have what I need* -- a
  live IMAP server, a system D-Bus, the real desktop -- and `ci.yml` greps the
  workflows to be sure nothing ever opts back into those. If a measurement
  wore `#[ignore]` too, the only way to run it would be the flag that guard
  forbids, which is exactly the trap the six were in.

This reads the filter and the sources and compiles nothing, so it belongs in
`check.sh` beside the other invariants rather than behind a build.

Usage: scripts/checks/check-measurement-tier.py
Exit status: 0 clean, 1 a drift this names.
"""

from __future__ import annotations

import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONFIG = ROOT / ".config" / "nextest.toml"
MARKER = "POSTIO-MEASUREMENT:"

# `binary(name)` and `test(/^module::/)` are the two shapes the filter uses.
BINARY = re.compile(r"binary\(([A-Za-z0-9_]+)\)")
MODULE = re.compile(r"test\(/\^([A-Za-z0-9_]+)::/\)")


def excluded_filter() -> str | None:
    """`profile.default`'s `default-filter`, or None if it has none."""
    parsed = tomllib.loads(CONFIG.read_text())
    return parsed.get("profile", {}).get("default", {}).get("default-filter")


def sources_for(binary: str, module: str | None = None) -> list[Path]:
    """The file(s) a filter term names.

    A `binary(x)` term is `crates/*/tests/x.rs` or a custom-harness directory
    `crates/*/tests/x/`; a `test(/^m::/)` term is a module inside one of those
    harnesses, `crates/*/tests/*/m.rs`.
    """
    if module is None:
        found = list(ROOT.glob(f"crates/*/tests/{binary}.rs"))
        found += [p for p in ROOT.glob(f"crates/*/tests/{binary}/main.rs")]
        return found
    return list(ROOT.glob(f"crates/*/tests/*/{module}.rs"))


def main() -> int:
    problems: list[str] = []

    expression = excluded_filter()
    if expression is None:
        print("no profile.default.default-filter in .config/nextest.toml", file=sys.stderr)
        return 1

    named: dict[str, list[Path]] = {}
    for binary in BINARY.findall(expression):
        named[f"binary({binary})"] = sources_for(binary)
    for module in MODULE.findall(expression):
        named[f"test(/^{module}::/)"] = sources_for(None if False else module, module)

    # 1. Every term names a file that exists and claims to be a measurement.
    for term, paths in named.items():
        if not paths:
            problems.append(f"{term} in the filter names no file under crates/*/tests/")
            continue
        for path in paths:
            if MARKER not in path.read_text():
                problems.append(
                    f"{term} is excluded from profile.default, but "
                    f"{path.relative_to(ROOT)} does not say so "
                    f"(add a `//! {MARKER} ...` line to its module docs)"
                )

    # 2. Every file that claims to be a measurement is named by the filter.
    claimed = {
        path
        for path in ROOT.glob("crates/*/tests/**/*.rs")
        if MARKER in path.read_text()
    }
    covered = {path for paths in named.values() for path in paths}
    for path in sorted(claimed - covered):
        problems.append(
            f"{path.relative_to(ROOT)} carries `{MARKER}` but nothing in "
            f"profile.default's default-filter excludes it, so it still runs "
            f"on every pull request"
        )

    # 3. No `#[ignore]` anywhere may mean "slow". That is the conflation #1450
    #    undid, and it is the one that silences a test completely: `#[ignore]`
    #    is for what this machine may not have, nothing in CI may pass
    #    --run-ignored, so an ignored measurement runs nowhere at all.
    #
    #    Note this is asked of every test file, not only the marked ones. A
    #    file may legitimately hold a measurement *and* a test ignored for a
    #    missing capability -- `smtp_wait_cpu.rs` holds exactly that pair --
    #    so the question is what a reason *says*, never where it sits.
    slow_words = re.compile(r"bench|measurement|slow|minutes|seeds \d|writes two", re.I)
    for path in sorted(ROOT.glob("crates/*/tests/**/*.rs")):
        for reason in re.findall(r'^\s*#\[ignore\s*=\s*"([^"]*)"', path.read_text(), re.MULTILINE):
            if slow_words.search(reason):
                problems.append(
                    f"{path.relative_to(ROOT)} has `#[ignore = \"{reason}\"]`, which "
                    f"is a scheduling reason wearing `#[ignore]`. Nothing in CI may "
                    f"pass --run-ignored, so this test runs nowhere. Drop the "
                    f"attribute and exclude it in .config/nextest.toml's "
                    f"profile.default instead, with a `//! {MARKER} ...` line "
                    f"saying so (#1450)."
                )

    if problems:
        print("measurement-tier check failed:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1

    print(f"measurement-tier check passed ({len(claimed)} measurement file(s)).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
