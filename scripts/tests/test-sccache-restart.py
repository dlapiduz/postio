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

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "sccache-restart.sh"

FAILURES: list[str] = []

# The stub advances its counter before printing: `executed()` reads it through `awk '... exit'`,
# which closes the pipe after the first line, so a later increment
# lost the race to SIGPIPE and a live daemon read as WEDGED (#1243, #1254).
SCCACHE_STUB = """#!/usr/bin/env bash
if [ "${1:-}" = "--show-stats" ]; then
    count="$(cat "$COUNTER_FILE")"
    if [ "${COUNTER_MOVES:-0}" = "1" ]; then
        printf '%s' "$((count + 7))" > "$COUNTER_FILE"
    fi
    printf 'Compile requests executed %s\\n' "$count"
    printf 'Max cache size                       %s\\n' "$MAX_SIZE"
    exit 0
fi
exit 0
"""

PS_STUB = """#!/usr/bin/env bash
cat "$PS_FILE"
"""


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


KILL_STUB = """#!/usr/bin/env bash
printf '%s\\n' "$@" >> "$KILLED_FILE"
"""


def run(base: Path, *, waiting: int, moves: bool, args: list[str]) -> subprocess.CompletedProcess:
    stub_dir = base / "stubs"
    counter = base / "counter"
    counter.write_text("5351")
    process_list = base / "ps"
    lines = ["      1       1 /sbin/init"]
    for index in range(waiting):
        lines.append(f"   {900 + index}    {2000 + index} sccache /usr/bin/rustc --crate-name c{index}")
    process_list.write_text("\n".join(lines) + "\n")

    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir}:/usr/bin:/bin"
    environment["COUNTER_FILE"] = str(counter)
    environment["PS_FILE"] = str(process_list)
    environment["COUNTER_MOVES"] = "1" if moves else "0"
    environment["MAX_SIZE"] = "30 GiB"
    environment["POSTIO_SCCACHE_WINDOW"] = "0"
    environment["POSTIO_SCCACHE_STALLED_AFTER"] = "300"
    killed = base / "killed"
    killed.write_text("")
    environment["KILLED_FILE"] = str(killed)
    environment["POSTIO_SCCACHE_KILL"] = str(stub_dir / "kill-recorder")
    return patience.run(
        ["bash", str(SCRIPT), *args],
        env=environment,
        capture_output=True,
        text=True,
    )


def main() -> int:
    try:
        return run_the_cases()
    except patience.ScriptHung as hung:
        print(f"TIMED OUT  {hung}", file=sys.stderr)
        return 1


def run_the_cases() -> int:
    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)
        stub_dir = base / "stubs"
        stub_dir.mkdir()
        for name, body in (
            ("sccache", SCCACHE_STUB),
            ("ps", PS_STUB),
            ("kill-recorder", KILL_STUB),
        ):
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

        reaped = run(base, waiting=3, moves=False, args=["--reap"])
        killed = (base / "killed").read_text().split()
        case(
            "--reap kills the compiles parked on the daemon it replaced",
            sorted(killed) == ["2000", "2001", "2002"],
            "a restart alone leaves them parked for ever, holding cargo's "
            f"locks; got {killed!r}: {reaped.stdout}{reaped.stderr}",
        )
        case(
            "--reap names each one it kills",
            all(pid in reaped.stdout for pid in ("2000", "2001", "2002")),
            "killing someone's build silently is not a thing a script should "
            f"do; got {reaped.stdout!r}",
        )

        nothing = run(base, waiting=0, moves=False, args=["--reap"])
        case(
            "--reap on a clean box kills nothing",
            (base / "killed").read_text().split() == [] and nothing.returncode == 0,
            "there is nothing parked, and a reap that kills a healthy compile "
            f"is worse than the wedge; got {nothing.stdout}{nothing.stderr}",
        )

        plain = run(base, waiting=3, moves=False, args=[])
        case(
            "a plain restart kills nothing",
            (base / "killed").read_text().split() == [],
            "the default must stay non-destructive: someone reaching for a "
            f"restart is not asking for their builds to die; got {plain.stdout}",
        )
        case(
            "but it says what it left behind, and how to clear it",
            "--reap" in plain.stdout + plain.stderr,
            "the gap is invisible otherwise -- the daemon looks fixed and the "
            f"box is still serialised; got {plain.stdout}{plain.stderr}",
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
