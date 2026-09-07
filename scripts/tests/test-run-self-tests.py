#!/usr/bin/env python3
"""Self-test for scripts/run-self-tests.sh (#1257).

The runner used to be inline in `ci.yml`, and it decided which failing logs to
print by grepping their *content*:

    if ! grep -qE "passed|ok\\b" "$log"; then ... tail -n 40 "$log"

Every self-test here prints `ok  <case>` per passing case before reporting
failures, so that filter skipped the log of any test that failed after getting
one case right -- which is nearly all of them. The job announced "a tooling
self-test failed; its output follows" and then followed with nothing, twice in
one day, and both times the only way to the cause was reproducing it locally.
The second was a real race, not a flake.

Which tests failed is known from their exit status, so nothing has to be
guessed from their text. That is what this covers.

It also covers the thing that made the bug invisible: a fixture that prints
`ok` and *then* fails is the exact shape the old filter dropped.

Usage: scripts/tests/test-run-self-tests.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import sys
import tempfile
from pathlib import Path

# The shared dial (#1249). `scripts/lib`, not beside this file, because CI
# runs every `scripts/tests/*.py` it finds as a self-test.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
RUNNER = HERE / "run-self-tests.sh"

FAILURES: list[str] = []

# The shape that used to vanish: passes a case, prints `ok`, then fails.
PRINTS_OK_THEN_FAILS = """#!/usr/bin/env python3
import sys
print("ok    the first case behaved")
print("FAIL  the-telltale-line: it did not", file=sys.stderr)
sys.exit(1)
"""

PASSES = """#!/usr/bin/env python3
print("ok    everything behaved")
print("all cases behaved")
"""

CRASHES_IMMEDIATELY = """#!/usr/bin/env python3
raise SystemExit("the-crash-line: nothing ran at all")
"""


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def run(directory: Path, logs: Path):
    return patience.run(
        ["bash", str(RUNNER), "--dir", str(directory), "--logs", str(logs), "--jobs", "2"],
        capture_output=True,
        text=True,
        timeout=120,
    )


def write(directory: Path, name: str, body: str) -> None:
    path = directory / name
    path.write_text(body, encoding="utf-8")
    path.chmod(0o755)


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)

        # -- Everything passes ------------------------------------------------
        good = base / "good"
        good.mkdir()
        write(good, "test-a.py", PASSES)
        write(good, "test-b.py", PASSES)
        result = run(good, base / "logs-good")
        case(
            "a clean run succeeds",
            result.returncode == 0,
            f"exit {result.returncode}: {result.stdout!r} {result.stderr!r}",
        )
        case(
            "and says how many it ran",
            "2" in result.stdout,
            f"the count is missing: {result.stdout!r}",
        )

        # -- The bug: ok, then failure ----------------------------------------
        mixed = base / "mixed"
        mixed.mkdir()
        write(mixed, "test-passes.py", PASSES)
        write(mixed, "test-ok-then-fails.py", PRINTS_OK_THEN_FAILS)
        result = run(mixed, base / "logs-mixed")
        combined = result.stdout + result.stderr

        case(
            "a failing self-test fails the run",
            result.returncode != 0,
            f"exit {result.returncode}: {combined}",
        )
        case(
            "and it names which one failed",
            "test-ok-then-fails" in combined,
            f"the failing test is not named:\n{combined}",
        )
        case(
            "and shows its output even though it printed `ok` first",
            "the-telltale-line" in combined,
            "the failing test's own output was not surfaced -- this is exactly "
            f"#1257, where the job announced a failure and printed nothing:\n{combined}",
        )
        case(
            "and does not bury it under the passing test's output",
            "everything behaved" not in combined,
            f"a passing test's log was printed too:\n{combined}",
        )

        # -- The shape that did work before, which must keep working ----------
        crashing = base / "crashing"
        crashing.mkdir()
        write(crashing, "test-crashes.py", CRASHES_IMMEDIATELY)
        result = run(crashing, base / "logs-crash")
        combined = result.stdout + result.stderr
        case(
            "a test that fails before printing anything is still surfaced",
            result.returncode != 0 and "the-crash-line" in combined,
            f"exit {result.returncode}:\n{combined}",
        )

        # -- A runner that starts nothing must not report success -----------
        #
        # The regression #1151 found: `xargs -a` is a GNU extension, BSD
        # rejects it, and with its stderr discarded no child ever ran. The
        # old success test was "nothing recorded a failure", which is true
        # both when every test passed and when none of them started -- the
        # two answers a suite must never confuse. So this asserts the
        # positive fact instead: a log per test, or it did not run.
        #
        # `PATH` is emptied of a working `xargs` by pointing at a stub that
        # refuses, which is what BSD did on every Mac for the life of this
        # script.
        stub = base / "stub-bin"
        stub.mkdir()
        (stub / "xargs").write_text(
            "#!/bin/sh\necho 'xargs: invalid option -- a' >&2\nexit 1\n"
        )
        (stub / "xargs").chmod(0o755)

        starved = base / "starved"
        starved.mkdir()
        write(starved, "test-a.py", PASSES)
        broken = patience.run(
            [
                "bash",
                str(RUNNER),
                "--dir",
                str(starved),
                "--logs",
                str(base / "logs-starved"),
                "--jobs",
                "2",
            ],
            capture_output=True,
            text=True,
            timeout=120,
            env={**os.environ, "PATH": f"{stub}:{os.environ.get('PATH', '')}"},
        )
        combined = broken.stdout + broken.stderr
        case(
            "a runner that could not start its children says so, not 'passed'",
            broken.returncode != 0 and "every self-test passed" not in combined,
            f"exit {broken.returncode}:\n{combined}",
        )
        case(
            "and says how many of them actually ran",
            "ran 0 of 1" in combined,
            f"it did not say what it managed:\n{combined}",
        )

    for failure in FAILURES:
        print(f"FAIL  {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed", file=sys.stderr)
        return 1
    print("\nrun-self-tests: all cases behaved")
    return 0


if __name__ == "__main__":
    sys.exit(main())
