#!/usr/bin/env python3
"""Self-test for scripts/ci-tooling-needed.sh.

The predicate decides whether CI runs the tooling self-tests, which are eight
of the nine minutes of the `Crate boundaries` job (#996). A wrong answer in
one direction costs eight minutes of a runner on the critical path; in the
other it merges a change to `scripts/` that nothing checked. Both directions
are exercised here.

It lived as fifteen lines of inline `bash` in `ci.yml` until this, where it
could not be tested at all -- and the self-tests are exactly the thing that
was supposed to keep the tooling honest. A rule about when to run the tests
that is itself untested is the shape #996's own option (4) is about.

Usage: scripts/tests/test-ci-tooling-needed.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "ci-tooling-needed.sh"

FAILURES: list[str] = []


def decide(files: list[str] | None, event: str = "pull_request") -> str:
    """What the predicate answers for this diff. `None` and `[]` both send no
    input, which the script reads as "cannot prove anything" -- see the case
    about it below."""
    text = "" if files is None else "".join(f"{name}\n" for name in files)
    result = subprocess.run(
        ["bash", str(SCRIPT), event],
        input=text,
        capture_output=True,
        text=True,
        timeout=30,
    )
    if result.returncode != 0:
        return f"exit {result.returncode}: {result.stderr.strip()}"
    return result.stdout.strip()


def case(name: str, expected: str, **kwargs) -> None:
    got = decide(**kwargs)
    if got != expected:
        FAILURES.append(f"{name}\n    expected {expected!r}, got {got!r}")


def main() -> int:
    if not SCRIPT.exists():
        print(f"missing {SCRIPT}", file=sys.stderr)
        return 1

    # ---- it fails safe, in all four directions that matter ---------------

    case(
        "a push to main runs them whatever changed",
        files=["crates/postio-core/src/lib.rs"],
        event="push",
        expected="yes",
    )
    case(
        "a diff that could not be read runs them",
        files=None,
        expected="yes",
    )
    case(
        "a change to the tooling itself runs them",
        files=["scripts/issue-land.sh"],
        expected="yes",
    )
    case(
        "and so does one to a directory nobody has classified yet",
        files=["packaging/flatpak.yml"],
        expected="yes",
        # The property the allow-list shape exists for: a new top-level
        # directory is unknown, and unknown means run. A deny-list of "risky"
        # paths would silently skip it.
    )

    # ---- and skips only what it can prove cannot matter -------------------

    case(
        "a Rust-only change cannot affect scripts/, so it skips",
        files=["crates/postio-gtk/src/list.rs", "crates/postio-app/src/lib.rs"],
        expected="no",
    )
    case(
        "and neither can a documentation-only one",
        # The measured case: three of five landings in one session paid the
        # eight minutes for a docs edit. Every self-test builds its own
        # sandbox repository -- `test-mutants-gate.py` says so at length,
        # because it is the one that would otherwise read the real
        # `docs/mutants-baseline.txt` -- so nothing under `docs/` can change
        # what they report.
        files=["docs/PRODUCT.md", "docs/config.md"],
        expected="no",
    )
    case(
        "docs and crates together still skip",
        files=["docs/config.md", "crates/postio-config/src/rules.rs"],
        expected="no",
    )
    case(
        "but one tooling file among them is enough to run them",
        files=["docs/config.md", "crates/postio-config/src/rules.rs", "scripts/check.sh"],
        expected="yes",
    )
    case(
        "including a check's own data, which a check reads",
        # `uncalled-pub-fn-baseline.txt` is data rather than code, and it
        # would be tempting to treat it as inert. It is not: the check reads
        # it, and a self-test of that check reads the check.
        files=["scripts/checks/uncalled-pub-fn-baseline.txt"],
        expected="yes",
    )
    case(
        "a root file is not classified as safe",
        files=["Cargo.toml"],
        expected="yes",
    )
    case(
        "nor is a workflow",
        files=[".github/workflows/ci.yml"],
        expected="yes",
    )
    case(
        "an empty diff runs them, because it reads the same as no answer",
        # Deliberately conflated. A pull request with genuinely no files is a
        # degenerate case worth nothing, and the alternative -- a second
        # channel telling the script whether the API answered -- is a way for
        # the two to disagree. Empty means "cannot prove anything", and that
        # means run.
        files=[],
        expected="yes",
    )

    for failure in FAILURES:
        print(f"FAIL: {failure}")
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed")
        return 1
    print("ci-tooling-needed self-test passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
