#!/usr/bin/env python3
"""Prove `check-measurement-tier.py` fails on each drift it claims to catch.

A guard nobody has seen fail is a guard nobody should trust. This is the
measurement tier's, and it matters more than most: what the tier does is stop
tests running, so every way it can go wrong is a way for a test to disappear
quietly.

Each case builds a small tree -- a nextest config and a couple of test files --
breaks exactly one thing, and asserts the check notices and says which.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CHECK = ROOT / "scripts" / "checks" / "check-measurement-tier.py"
MARKER = "POSTIO-MEASUREMENT:"
FAILURES: list[str] = []

FILTER = "not (binary(slow_thing) + (package(p) & test(/^measured::/)))"


def sandbox(tmp: Path, *, config: str, files: dict[str, str]) -> Path:
    (tmp / ".config").mkdir(parents=True)
    (tmp / ".config" / "nextest.toml").write_text(config)
    (tmp / "scripts" / "checks").mkdir(parents=True)
    shutil.copy(CHECK, tmp / "scripts" / "checks" / CHECK.name)
    for relative, body in files.items():
        path = tmp / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)
    return tmp


def run(tmp: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(tmp / "scripts" / "checks" / CHECK.name)],
        capture_output=True, text=True,
    )


def config_with(filter_expression: str) -> str:
    return f'[profile.default]\ndefault-filter = \'{filter_expression}\'\n'


def marked(body: str = "") -> str:
    return f"//! A thing.\n//!\n//! {MARKER} numbers a person reads.\n{body}"


def case(name: str, *, config: str, files: dict[str, str], expect_fail: bool,
         mentions: str = "") -> None:
    with tempfile.TemporaryDirectory() as raw:
        tmp = sandbox(Path(raw), config=config, files=files)
        result = run(tmp)
        failed = result.returncode != 0
        ok = failed == expect_fail and (not mentions or mentions in result.stderr)
        print(f"{'ok   ' if ok else 'FAIL '} {name}")
        if not ok:
            FAILURES.append(
                f"{name}: exit {result.returncode} (wanted "
                f"{'non-zero' if expect_fail else 'zero'}); stderr={result.stderr.strip()[:200]}"
            )


def main() -> int:
    good = {
        "crates/p/tests/slow_thing.rs": marked(),
        "crates/p/tests/suite/measured.rs": marked(),
        "crates/p/tests/suite/ordinary.rs": "//! Nothing special.\n",
    }

    case("a tier that agrees with itself", config=config_with(FILTER),
         files=good, expect_fail=False)

    # The filter silences a file that never says it is silenced.
    quiet = dict(good)
    quiet["crates/p/tests/slow_thing.rs"] = "//! A thing with no marker.\n"
    case("excluded but unmarked", config=config_with(FILTER), files=quiet,
         expect_fail=True, mentions="does not say so")

    # The file claims the tier; the filter has never heard of it.
    orphan = dict(good)
    orphan["crates/p/tests/suite/ordinary.rs"] = marked()
    case("marked but still on the merge path", config=config_with(FILTER),
         files=orphan, expect_fail=True, mentions="still runs on every pull request")

    # A term naming nothing -- a rename that only got as far as the source.
    case("the filter names a file that is gone",
         config=config_with("not (binary(departed) + binary(slow_thing) "
                            "+ (package(p) & test(/^measured::/)))"),
         files=good, expect_fail=True, mentions="names no file")

    # The conflation coming back, in a file that is not marked at all.
    relapse = dict(good)
    relapse["crates/p/tests/suite/ordinary.rs"] = (
        '//! Ordinary.\n#[ignore = "a bench, not a gate"]\nfn x() {}\n'
    )
    case("an #[ignore] that means slow", config=config_with(FILTER), files=relapse,
         expect_fail=True, mentions="scheduling reason wearing")

    # ...and the legitimate neighbour it must not flag: one file holding a
    # measurement and a test ignored for a capability, which is what
    # `smtp_wait_cpu.rs` really is.
    neighbour = dict(good)
    neighbour["crates/p/tests/slow_thing.rs"] = marked(
        '#[ignore = "needs a system D-Bus and a live NetworkManager"]\nfn y() {}\n'
    )
    case("a measurement beside a capability-ignored test",
         config=config_with(FILTER), files=neighbour, expect_fail=False)

    # A module name two suites share, held out of one crate only: the
    # `package(p) &` scope decides which file must say so, and the other
    # crate's module keeps running on every pull request, unmarked.
    shared = dict(good)
    shared["crates/q/tests/other_suite/measured.rs"] = "//! The other crate's, still on every pull request.\n"
    case("a scoped term reaches only its crate", config=config_with(FILTER),
         files=shared, expect_fail=False)
    case("an unscoped term reaches every crate's module",
         config=config_with("not (binary(slow_thing) + test(/^measured::/))"),
         files=shared, expect_fail=True, mentions="crates/q/tests/other_suite/measured.rs")

    # One case held out of a fast file: the marker sits on that case, and a
    # bare `fn` of the same name with nothing above it is the drift.
    percase = dict(good)
    percase["crates/p/tests/suite/mixed.rs"] = (
        "//! Fast cases, and one that waits.\n\n"
        f"/// {MARKER} it sleeps two seconds.\n#[test]\nfn waits() {{}}\n\n#[test]\nfn quick() {{}}\n"
    )
    scoped_case = config_with(FILTER[:-1] + " + (package(p) & test(=mixed::waits)))")
    case("a marked case held out by name passes", config=scoped_case,
         files=percase, expect_fail=False)
    unmarked = dict(percase)
    unmarked["crates/p/tests/suite/mixed.rs"] = "//! Fast cases.\n\n#[test]\nfn waits() {}\n"
    case("a case held out by name without its marker fails", config=scoped_case,
         files=unmarked, expect_fail=True, mentions="mixed.rs:4 does not say so")

    # `profile.ci-full` holds the storyboard passes out because
    # `profile.ci-storyboards` runs them. The two spellings must agree, or a
    # test is held out of one and selected by neither.
    passes = "binary(focus_suite) & test(/^every_command::/)"
    split = (
        config_with(FILTER)
        + "[profile.ci-full]\n"
        + f"default-filter = 'not (binary(cache_pressure) + ({passes}))'\n"
        + "[profile.ci-storyboards]\n"
        + f"default-filter = '{passes}'\n"
    )
    case("the storyboard split agrees with itself", config=split, files=good,
         expect_fail=False)
    drifted = split.replace(
        "default-filter = 'binary(focus_suite) & test(/^every_command::/)'",
        "default-filter = 'binary(focus_suite) & test(/^every_command_renamed::/)'",
    )
    case("ci-full holds out what ci-storyboards does not run", config=drifted,
         files=good, expect_fail=True, mentions="ci-storyboards")

    # No filter at all is a tier that has been deleted, not an empty one.
    case("no default-filter", config="[profile.default]\nretries = 0\n",
         files=good, expect_fail=True)

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("check-measurement-tier self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
