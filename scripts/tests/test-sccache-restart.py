#!/usr/bin/env python3
"""Self-test for scripts/sccache-restart.sh (#1184).

The wedge signature is **two** facts together: the daemon's
`Compile requests executed` counter frozen, *and* `rustc` processes sitting
for minutes. Either alone is ordinary -- a frozen counter is an idle machine,
which is most of the time, and a long-running compile is a large crate. A
check that fired on one of them would cry wolf on an idle box and be ignored,
which is the same as not having one.

`sccache` and `ps` are both stubbed on PATH, so this runs anywhere, in
milliseconds, and never touches the real shared daemon -- which matters more
here than usual, because the failure this script exists to prevent is caused
by starting one carelessly.

Usage: scripts/tests/test-sccache-restart.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "sccache-restart.sh"

FAILURES: list[str] = []

# How long the stubbed script gets before the deadline is treated as a hang.
#
# It guards nothing this file asserts. The script's own waiting is already
# neutralised by the fixture (`POSTIO_SCCACHE_WINDOW=0`), so every case here is
# a few shell invocations against stubs and finishes in milliseconds -- the
# deadline is only here so a genuine hang fails rather than blocking the job
# for ever.
#
# It failed once anyway (#1243, run 34043679568): the Crate boundaries job runs
# 81 self-tests four at a time on a shared runner, and 30 seconds of wall clock
# is generous on an idle box and not obviously generous on that one. Nothing on
# the branch touched sccache.
BASE_TIMEOUT = 120.0

# The dial the Rust suite already has, reaching this file too.
#
# `postio_test_support::patience` multiplies every deadline in the Rust suite
# by `POSTIO_TEST_PATIENCE`, so a loaded machine is one environment variable
# rather than a pull request that enlarges a constant and slows every run
# afterwards. That dial stopped at the crate boundary; a deadline written by
# hand in Python could not hear it. Now it can, and reads the same variable,
# because two dials that mean the same thing is one dial nobody sets.
PATIENCE_ENV = "POSTIO_TEST_PATIENCE"


def patience() -> float:
    """The multiplier `POSTIO_TEST_PATIENCE` asks for, or 1.

    An unparseable or non-positive value is ignored rather than honoured, the
    same as the Rust side: a typo in a workflow should not quietly set every
    deadline to zero and turn every wait into an instant failure.
    """
    raw = os.environ.get(PATIENCE_ENV)
    if not raw:
        return 1.0
    try:
        value = float(raw)
    except ValueError:
        return 1.0
    return value if value > 0 else 1.0


def timeout_seconds() -> float:
    return BASE_TIMEOUT * patience()


class ScriptHung(Exception):
    """The script under test ran out of wall clock rather than misbehaving.

    Its own exception because the two findings are not the same and used to
    look identical: a `TimeoutExpired` surfaced as a plain FAILED, so a run
    that lost a race with a loaded runner read exactly like the script getting
    the answer wrong. #1243 is one of those, and it cost a session the time to
    establish that nothing on the branch touched sccache.
    """

# A daemon whose counter reads from a file, so a case can decide whether the
# second reading differs from the first.
SCCACHE_STUB = """#!/usr/bin/env bash
if [ "${1:-}" = "--show-stats" ]; then
    count="$(cat "$COUNTER_FILE")"
    printf 'Compile requests executed %s\\n' "$count"
    printf 'Max cache size                       %s\\n' "$MAX_SIZE"
    if [ "${COUNTER_MOVES:-0}" = "1" ]; then
        printf '%s' "$((count + 7))" > "$COUNTER_FILE"
    fi
    exit 0
fi
exit 0
"""

# `ps -eo etimes,args`, answering with whatever the case set up.
PS_STUB = """#!/usr/bin/env bash
cat "$PS_FILE"
"""


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def run(base: Path, *, waiting: int, moves: bool, args: list[str]) -> subprocess.CompletedProcess:
    stub_dir = base / "stubs"
    counter = base / "counter"
    counter.write_text("5351")
    process_list = base / "ps"
    lines = ["      1 /sbin/init"]
    for index in range(waiting):
        lines.append(f"   {900 + index} /usr/bin/rustc --crate-name c{index}")
    process_list.write_text("\n".join(lines) + "\n")

    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir}:/usr/bin:/bin"
    environment["COUNTER_FILE"] = str(counter)
    environment["PS_FILE"] = str(process_list)
    environment["COUNTER_MOVES"] = "1" if moves else "0"
    environment["MAX_SIZE"] = "30 GiB"
    # So a case that reaches the two-reading path does not actually wait.
    environment["POSTIO_SCCACHE_WINDOW"] = "0"
    environment["POSTIO_SCCACHE_STALLED_AFTER"] = "300"
    limit = timeout_seconds()
    try:
        return subprocess.run(
            ["bash", str(SCRIPT), *args],
            env=environment,
            capture_output=True,
            text=True,
            timeout=limit,
        )
    except subprocess.TimeoutExpired as expired:
        raise ScriptHung(
            f"`sccache-restart.sh {' '.join(args)}` did not finish within "
            f"{limit:g}s. Every case here runs against stubs with the script's "
            f"own waiting switched off, so this is a hang or a runner too "
            f"loaded to finish milliseconds of work -- not the script "
            f"answering wrongly. Raise {PATIENCE_ENV} (currently "
            f"{patience():g}x over a {BASE_TIMEOUT:g}s base) if the machine is "
            f"busy."
        ) from expired


def main() -> int:
    try:
        return run_the_cases()
    except ScriptHung as hung:
        print(f"TIMED OUT  {hung}", file=sys.stderr)
        return 1


def run_the_cases() -> int:
    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)
        stub_dir = base / "stubs"
        stub_dir.mkdir()
        for name, body in (("sccache", SCCACHE_STUB), ("ps", PS_STUB)):
            stub = stub_dir / name
            stub.write_text(body)
            stub.chmod(0o755)

        idle = run(base, waiting=0, moves=False, args=["--check"])
        case(
            "an idle box is not a wedge",
            idle.returncode == 0,
            "the counter is frozen whenever nothing is compiling, which is "
            "most of the time. A check that reports that is one nobody reads; "
            f"got exit {idle.returncode}: {idle.stdout}{idle.stderr}",
        )

        busy = run(base, waiting=4, moves=True, args=["--check"])
        case(
            "compiles waiting on a daemon that is answering is a slow build",
            busy.returncode == 0,
            "a large crate takes minutes and the counter moves the whole "
            f"time; got exit {busy.returncode}: {busy.stdout}{busy.stderr}",
        )

        wedged = run(base, waiting=4, moves=False, args=["--check"])
        case(
            "waiting compiles and a frozen counter together is the wedge",
            wedged.returncode == 3,
            "this is the signature #1184 measured, and reporting it is the "
            f"whole point; got exit {wedged.returncode}: {wedged.stdout}{wedged.stderr}",
        )
        case(
            "and it says how to fix it",
            "sccache-restart.sh" in wedged.stderr,
            "the remedy is not obvious and the obvious one breaks the cache; "
            f"got {wedged.stderr!r}",
        )

        healthy = run(base, waiting=0, moves=False, args=["--if-wedged"])
        case(
            "--if-wedged leaves a healthy daemon alone",
            healthy.returncode == 0 and "restarted" not in healthy.stdout,
            "restarting a working daemon throws away a warm cache for "
            f"nothing; got exit {healthy.returncode}: {healthy.stdout}",
        )

    for failure in FAILURES:
        print(f"FAIL  {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed", file=sys.stderr)
        return 1
    print("\nsccache-restart: all cases behaved")
    return 0


if __name__ == "__main__":
    sys.exit(main())
