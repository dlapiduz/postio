#!/usr/bin/env python3
"""Self-test for scripts/checks/check-suite-modules.py.

The check guards the risk #841 introduced. Consolidating 197 integration
binaries into a few suites removed a property the old shape had for free:
cargo compiles every `tests/*.rs` into its own binary, so a file that existed
ran. Inside a suite directory a file runs only if `main.rs` declares it, and
an undeclared file is not an error — it is silence.

A guard that passes on a clean tree passes whether it works or not, so the
failure modes are exercised here: throwaway git repositories, one suite each.

Worth recording why this file exists at all. Writing the check, I "proved" it
caught a deleted `mod` by deleting one and watching it fail — except the
suite directories were untracked at the time, `git ls-files` did not list
them, and the check had silently examined nothing. It reported success. That
is the same shape as the bug it guards against, one level up, and it is why
the first case below asserts the suite was actually *seen*.

Usage: scripts/tests/test-check-suite-modules.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-suite-modules.py"

FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def build(root: Path, main: str, *, files: tuple[str, ...], track: bool = True) -> None:
    suite = root / "crates" / "postio-thing" / "tests" / "thing_suite"
    suite.mkdir(parents=True)
    (suite / "main.rs").write_text(main, encoding="utf-8")
    for name in files:
        (suite / f"{name}.rs").write_text("#[test]\nfn t() {}\n", encoding="utf-8")
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

        # -- 1. a fully declared suite passes, and is actually looked at -----
        root = base / "clean"
        build(root, "mod a;\nmod b;\n", files=("a", "b"))
        result = run(root)
        case(
            "a suite whose files are all declared passes",
            result.returncode == 0,
            f"exit {result.returncode}: {result.stdout}{result.stderr}",
        )
        case(
            "and the suite was actually examined, not skipped",
            "(1 suites)" in result.stdout,
            "the check reported success without finding the suite, which is "
            f"how it lied to me while I was writing it:\n{result.stdout}",
        )

        # -- 2. an undeclared file fails, and is named ----------------------
        root = base / "undeclared"
        build(root, "mod a;\n", files=("a", "forgotten"))
        result = run(root)
        case(
            "an undeclared file fails the check",
            result.returncode == 1,
            f"exit {result.returncode}: a file nobody declared was accepted, "
            "so its tests would have stopped running silently",
        )
        case(
            "and the failure names it",
            "forgotten.rs" in result.stderr,
            f"the report does not say which file:\n{result.stderr}",
        )

        # -- 3. untracked files are not the repository's problem ------------
        root = base / "untracked"
        build(root, "mod a;\n", files=("a", "scratch"), track=False)
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
    print("suite-modules self-test passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
