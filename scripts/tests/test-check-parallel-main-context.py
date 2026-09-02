#!/usr/bin/env python3
"""Self-test for scripts/checks/check-parallel-main-context.py.

The check exists because #841's `logic_suite` was assembled on the rule "does
the file call `adw::init`", which is the obvious question and the wrong one.
`list_model` and `drag_out` initialize nothing; one calls
`MainContext::default().iteration()` and the other `block_on`. *Acquiring*
the default context is as process-global as initializing GTK, two of them on
libtest's thread pool aborted the binary, and it passed locally and failed on
a four-core runner.

The rule has to be narrower than "no main context in a parallel target",
because a target with **one** test has nothing to race: `drag_out` is a
parallel binary with a single case that calls `block_on`, and that is
correct. What is unsafe is more than one test in a parallel target where the
context is acquired at all — so that is what fails.

Usage: scripts/tests/test-check-parallel-main-context.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-parallel-main-context.py"

FAILURES: list[str] = []

USES_CONTEXT = """#[test]
fn one() {
    while glib::MainContext::default().iteration(false) {}
}
"""
SECOND_TEST = """#[test]
fn two() {
    assert!(true);
}
"""
PURE = """#[test]
fn pure_one() { assert!(true); }
#[test]
fn pure_two() { assert!(true); }
"""


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def build(root: Path, *, files: dict[str, str], manifest_extra: str = "") -> None:
    crate = root / "crates" / "postio-thing"
    (crate / "tests").mkdir(parents=True)
    (crate / "src").mkdir(parents=True)
    (crate / "src" / "lib.rs").write_text("// x\n", encoding="utf-8")
    (crate / "Cargo.toml").write_text(
        '[package]\nname = "postio-thing"\nversion = "0.1.0"\n' + manifest_extra,
        encoding="utf-8",
    )
    for name, body in files.items():
        path = crate / "tests" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body, encoding="utf-8")
    subprocess.run(["git", "init", "-q", "-b", "main", str(root)], check=True)
    for args in (["add", "-A"], ["-c", "user.email=t@example.com", "-c",
                                 "user.name=T", "commit", "-qm", "x"]):
        subprocess.run(["git", *args], cwd=root, check=True)


def run(root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(CHECK)], cwd=root, capture_output=True, text=True
    )


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)

        # -- 1. the shape that aborted the binary ---------------------------
        root = base / "racy"
        build(root, files={"suite/main.rs": "mod a;\nmod b;\n",
                           "suite/a.rs": USES_CONTEXT,
                           "suite/b.rs": SECOND_TEST})
        result = run(root)
        case(
            "two tests in a parallel target, one acquiring the context, fails",
            result.returncode == 1,
            f"exit {result.returncode}: this is #841's abort, accepted:\n"
            f"{result.stdout}{result.stderr}",
        )
        case(
            "and the report names the file",
            "a.rs" in result.stderr,
            f"the failure does not say where to look:\n{result.stderr}",
        )

        # -- 2. one test may acquire it: nothing to race --------------------
        #
        # `drag_out` is exactly this and is correct. A check that failed it
        # would push a legitimate file into a sequential suite, which is
        # where it segfaulted in the first place.
        root = base / "single"
        build(root, files={"drag_out.rs": USES_CONTEXT})
        result = run(root)
        case(
            "a single-test parallel target may acquire the context",
            result.returncode == 0,
            f"exit {result.returncode}: a lone test has nothing to race\n"
            f"{result.stdout}{result.stderr}",
        )

        # -- 3. harness = false is allowed to ------------------------------
        root = base / "sequential"
        build(
            root,
            files={"gtk_suite/main.rs": "mod a;\nmod b;\n",
                   "gtk_suite/a.rs": USES_CONTEXT,
                   "gtk_suite/b.rs": SECOND_TEST},
            manifest_extra='\n[[test]]\nname = "gtk_suite"\n'
                           'path = "tests/gtk_suite/main.rs"\nharness = false\n',
        )
        result = run(root)
        case(
            "a harness = false suite may acquire it, running sequentially",
            result.returncode == 0,
            f"exit {result.returncode}: the suite that controls its own "
            f"scheduling was refused:\n{result.stdout}{result.stderr}",
        )

        # -- 4. no context at all is fine, however many tests ---------------
        root = base / "pure"
        build(root, files={"suite/main.rs": "mod a;\n", "suite/a.rs": PURE})
        result = run(root)
        case(
            "a parallel target that never touches the context passes",
            result.returncode == 0,
            f"exit {result.returncode}: {result.stdout}{result.stderr}",
        )

    for failure in FAILURES:
        print(f"FAIL  {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("parallel-main-context self-test passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
