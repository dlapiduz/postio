#!/usr/bin/env python3
"""Self-test for scripts/checks/check-tests-share-a-suite.py.

The check stops a crate growing a second integration binary that nobody
decided it needed: every `tests/*.rs` is its own link of the whole
dependency stack, and 57 of them had drifted back in beside the suites
#841 and #1128 made. The failure modes are exercised here, in throwaway git
repositories, because a guard that passes on a clean tree passes whether it
works or not.

Usage: scripts/tests/test-check-tests-share-a-suite.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-tests-share-a-suite.py"

FAILURES: list[str] = []

PLAIN = "#[test]\nfn t() {}\n"
OWN = "//! POSTIO-OWN-BINARY: it sets an environment variable.\n\n#[test]\nfn t() {}\n"
MEASURED = "//! POSTIO-MEASUREMENT: numbers a person reads.\n\n#[test]\nfn t() {}\n"


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def build(root: Path, files: dict[str, str], *, track: bool = True) -> None:
    """`files` maps paths under `crates/postio-thing/tests/` to contents."""
    tests = root / "crates" / "postio-thing" / "tests"
    for relative, text in files.items():
        path = tests / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    subprocess.run(["git", "init", "-q", "-b", "main", str(root)], check=True)
    if track:
        for args in (["add", "-A"], ["-c", "user.email=t@example.com",
                                     "-c", "user.name=T", "commit", "-qm", "x"]):
            subprocess.run(["git", *args], cwd=root, check=True)


def run(root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(CHECK)], cwd=root, capture_output=True, text=True
    )


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)

        root = base / "clean"
        build(root, {
            "thing_suite/main.rs": "mod a;\n",
            "thing_suite/a.rs": PLAIN,
            "own.rs": OWN,
            "measured.rs": MEASURED,
        })
        result = run(root)
        case(
            "a suite beside files that say why they stand alone passes",
            result.returncode == 0,
            f"exit {result.returncode}: {result.stdout}{result.stderr}",
        )
        case(
            "and the crate was actually examined, not skipped",
            "(1 crates)" in result.stdout,
            f"the check passed without finding the crate:\n{result.stdout}",
        )

        root = base / "straggler"
        build(root, {
            "thing_suite/main.rs": "mod a;\n",
            "thing_suite/a.rs": PLAIN,
            "straggler.rs": PLAIN,
        })
        result = run(root)
        case(
            "a file beside a suite with no reason fails",
            result.returncode == 1,
            f"exit {result.returncode}: a second binary nobody asked for was accepted",
        )
        case(
            "and the failure names the file and the suite",
            "straggler.rs" in result.stderr and "thing_suite" in result.stderr,
            f"the report does not say which file or where it goes:\n{result.stderr}",
        )

        root = base / "two-files"
        build(root, {"one.rs": PLAIN, "two.rs": PLAIN})
        result = run(root)
        case(
            "two plain files with no suite fail",
            result.returncode == 1,
            f"exit {result.returncode}: two binaries where one would do",
        )

        root = base / "lone"
        build(root, {"only.rs": PLAIN})
        result = run(root)
        case(
            "a crate's only test file needs no suite",
            result.returncode == 0,
            f"exit {result.returncode}: {result.stdout}{result.stderr}",
        )

        root = base / "marker-in-a-string"
        build(root, {
            "thing_suite/main.rs": "mod a;\n",
            "thing_suite/a.rs": PLAIN,
            "sly.rs": '#[test]\nfn t() { let _ = "//! POSTIO-OWN-BINARY: no"; }\n',
        })
        result = run(root)
        case(
            "a marker only counts as a doc line of its own",
            result.returncode == 1,
            f"exit {result.returncode}: a marker inside a string was believed",
        )

        root = base / "untracked"
        build(root, {
            "thing_suite/main.rs": "mod a;\n",
            "thing_suite/a.rs": PLAIN,
            "scratch.rs": PLAIN,
        }, track=False)
        result = run(root)
        case(
            "an untracked experiment does not fail somebody else's run",
            result.returncode == 0,
            f"exit {result.returncode}: {result.stdout}{result.stderr}",
        )

    for failure in FAILURES:
        print(f"FAIL  {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("tests-share-a-suite self-test passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
