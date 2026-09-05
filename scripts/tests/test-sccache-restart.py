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
    return subprocess.run(
        ["bash", str(SCRIPT), *args],
        env=environment,
        capture_output=True,
        text=True,
        timeout=30,
    )


def main() -> int:
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
