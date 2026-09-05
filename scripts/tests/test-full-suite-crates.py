#!/usr/bin/env python3
"""Self-test for scripts/full-suite-crates.sh.

The predicate decides which changed crates get their *integration* suites run
by `issue-land.sh`, on top of the sanity `--lib` tier. It exists because
several tests **enumerate** the command vocabulary rather than compiling
against it -- the golden binding table, `docs/keybindings.md`, `[keys]`,
`docs/config.md` -- so a change that builds cleanly fails CI ten minutes later
on an assertion about a table (#1047, and #1003 paid for it twice).

Both directions matter. Too narrow and the class of bug this exists for slips
through to CI; too wide and every landing that touches `postio-gtk` waits
minutes for a suite it did not need.

Usage: scripts/tests/test-full-suite-crates.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
REPO = HERE.parent
SCRIPT = HERE / "full-suite-crates.sh"

FAILURES: list[str] = []


def chosen(crates: list[str]) -> list[str]:
    result = subprocess.run(
        ["bash", str(SCRIPT)],
        input="".join(f"{name}\n" for name in crates),
        capture_output=True,
        text=True,
        timeout=30,
    )
    if result.returncode != 0:
        return [f"exit {result.returncode}: {result.stderr.strip()}"]
    return result.stdout.split()


def case(name: str, crates: list[str], expected: list[str]) -> None:
    got = chosen(crates)
    if got != expected:
        FAILURES.append(f"{name}\n    expected {expected}, got {got}")


def main() -> int:
    if not SCRIPT.exists():
        print(f"missing {SCRIPT}", file=sys.stderr)
        return 1

    # ---- the crates this exists for --------------------------------------

    case(
        "postio-core carries the golden binding table, so it runs",
        crates=["postio-core"],
        expected=["postio-core"],
    )
    case(
        "and postio-config carries [keys] and the config reference",
        crates=["postio-config"],
        expected=["postio-config"],
    )
    case(
        "a command change usually touches both",
        crates=["postio-config", "postio-core"],
        expected=["postio-config", "postio-core"],
    )

    # ---- and the cost exception ------------------------------------------

    case(
        "postio-gtk's suites are minutes, so a landing does not wait for them",
        crates=["postio-gtk"],
        expected=[],
    )
    case(
        "nor postio-app's",
        crates=["postio-app"],
        expected=[],
    )
    case(
        "the expensive ones are dropped and the rest are kept",
        crates=["postio-app", "postio-core", "postio-gtk", "postio-search"],
        expected=["postio-core", "postio-search"],
    )

    # ---- the property that makes the shape right -------------------------

    case(
        "a crate nobody has classified is *included*, not skipped",
        # The direction the whole design turns on. This is a deny-list of
        # crates that are too slow, so a crate nobody has thought about runs
        # its suites -- and a stale list costs seconds somebody notices. An
        # allow-list of "crates worth testing" would fail the other way: a
        # new golden table in a new crate, unchecked, invisibly.
        crates=["postio-newthing"],
        expected=["postio-newthing"],
    )
    case(
        "nothing changed, nothing to run",
        crates=[],
        expected=[],
    )

    # ---- and it stays honest about this repository ------------------------

    # Every crate the deny-list names has to exist, or it is silently
    # protecting nothing -- a rename would leave the slow suite running on
    # every landing and the list claiming otherwise.
    denied = subprocess.run(
        ["bash", str(SCRIPT), "--slow"],
        capture_output=True,
        text=True,
        timeout=30,
    ).stdout.split()
    if not denied:
        FAILURES.append("--slow listed no crates, so the exception is empty")
    for crate in denied:
        if not (REPO / "crates" / crate).is_dir():
            FAILURES.append(f"the slow list names {crate}, which is not a crate here")

    for failure in FAILURES:
        print(f"FAIL: {failure}")
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed")
        return 1
    print("full-suite-crates self-test passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
