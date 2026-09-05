#!/usr/bin/env python3
"""Self-test for #1104: one machine-wide cargo jobserver instead of `jobs = 2`.

`.cargo/config.toml` capped every cargo at two jobs because four sessions
once shared this eight-core box. The transcripts say that is the rare case:
while anything was building, one session was building 60% of the time and
two 26%, so six cores sat idle most of the day, and only 3 commands in the
whole history ever raised `-j`.

A GNU-make-style jobserver is the fix cargo already understands: a fifo
holding N tokens, `MAKEFLAGS=--jobserver-auth=fifo:<path>` in the
environment, and every cargo on the machine draws from the same pool. A
lone session gets the whole box; four share the same ceiling as before.
Verified by hand on cargo 1.98: a six-crate build went 13.3 s -> 4.7 s with
seven tokens, overriding `-j2`, and a missing fifo is a warning and a
fallback to `jobs`, not a failure.

What `scripts/jobserver.sh` has to get right, and what this checks:

  * `ensure` creates the fifo with exactly N tokens and a holder process
    that keeps it open -- a fifo nobody holds open drops its buffered tokens
    the moment the last cargo exits;
  * it is idempotent: a second `ensure` keeps the holder it has;
  * it refills. A cargo killed mid-build (79 tool timeouts in the
    transcripts) never returns the tokens it held, so the pool shrinks for
    ever. When no cargo or rustc is running, every token is by definition
    free, and `ensure` resets the pool to N; while one *is* running it must
    not, because a token that is out is somebody's live job;
  * `env` prints the export cargo needs; `stop` takes it all down.

The busy/idle question is answered by `pgrep` for real and by
`POSTIO_JOBSERVER_IDLE=1|0` here, because other sessions on this machine
are compiling while this test runs.

Usage: scripts/tests/test-jobserver.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import stat
import subprocess
import sys
import tempfile
import time
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
JOBSERVER = SCRIPTS / "jobserver.sh"

FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def run(directory: Path, *args: str, idle: str | None = None):
    environment = dict(os.environ)
    environment["POSTIO_JOBSERVER_DIR"] = str(directory)
    environment["POSTIO_JOBSERVER_TOKENS"] = "3"
    environment.pop("MAKEFLAGS", None)
    if idle is not None:
        environment["POSTIO_JOBSERVER_IDLE"] = idle
    else:
        environment.pop("POSTIO_JOBSERVER_IDLE", None)
    return subprocess.run(
        ["bash", str(JOBSERVER), *args],
        env=environment, capture_output=True, text=True, timeout=30,
    )


def tokens(fifo: Path) -> int:
    """Count the free tokens without keeping any: read them all, put them back."""
    fd = os.open(fifo, os.O_RDWR | os.O_NONBLOCK)
    try:
        held = b""
        while True:
            try:
                chunk = os.read(fd, 4096)
            except BlockingIOError:
                break
            if not chunk:
                break
            held += chunk
        if held:
            os.write(fd, held)
        return len(held)
    finally:
        os.close(fd)


def take_one(fifo: Path) -> None:
    """A client that acquired a token and died holding it."""
    fd = os.open(fifo, os.O_RDWR | os.O_NONBLOCK)
    try:
        os.read(fd, 1)
    finally:
        os.close(fd)


def alive(pid_file: Path) -> bool:
    try:
        pid = int(pid_file.read_text().strip())
        os.kill(pid, 0)
        return True
    except (OSError, ValueError):
        return False


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        directory = Path(raw) / "js"
        fifo = directory / "fifo"
        pid_file = directory / "holder.pid"
        try:
            result = run(directory, "ensure", idle="1")
            case("ensure succeeds", result.returncode == 0,
                 f"exit {result.returncode}\n{result.stdout}\n{result.stderr}")
            case("ensure creates a fifo", fifo.exists() and stat.S_ISFIFO(fifo.stat().st_mode),
                 f"{fifo} is not a fifo")
            if not fifo.exists():
                return finish()
            case("a holder keeps the fifo open", alive(pid_file),
                 f"no live pid in {pid_file}")
            case("the pool starts with N tokens", tokens(fifo) == 3,
                 f"{tokens(fifo)} tokens, want 3")

            first_pid = pid_file.read_text().strip()
            result = run(directory, "ensure", idle="1")
            case("a second ensure keeps the same holder",
                 result.returncode == 0 and pid_file.read_text().strip() == first_pid,
                 f"exit {result.returncode}; holder pid changed")

            result = run(directory, "env", idle="1")
            case("env prints the export cargo reads",
                 result.returncode == 0
                 and f"MAKEFLAGS=--jobserver-auth=fifo:{fifo}" in result.stdout.replace('"', ""),
                 f"exit {result.returncode}\n{result.stdout}\n{result.stderr}")

            # ── a leaked token comes back, but only when nothing is running ─
            take_one(fifo)
            case("the fixture leaked one token", tokens(fifo) == 2, f"{tokens(fifo)} tokens")
            result = run(directory, "ensure", idle="0")
            case("no refill while a build is running -- a missing token is a live job",
                 result.returncode == 0 and tokens(fifo) == 2,
                 f"exit {result.returncode}; {tokens(fifo)} tokens, want 2 still")
            result = run(directory, "ensure", idle="1")
            case("idle ensure refills the pool to N",
                 result.returncode == 0 and tokens(fifo) == 3,
                 f"exit {result.returncode}; {tokens(fifo)} tokens, want 3\n{result.stderr}")

            # ── a dead holder is replaced, tokens and all ────────────────────
            os.kill(int(first_pid), 15)
            time.sleep(0.2)
            result = run(directory, "ensure", idle="1")
            case("a dead holder is replaced",
                 result.returncode == 0 and alive(pid_file)
                 and pid_file.read_text().strip() != first_pid and tokens(fifo) == 3,
                 f"exit {result.returncode}; alive={alive(pid_file)}; {tokens(fifo) if fifo.exists() else 'no'} tokens")

            result = run(directory, "stop")
            case("stop takes the holder and fifo down",
                 result.returncode == 0 and not alive(pid_file) and not fifo.exists(),
                 f"exit {result.returncode}; alive={alive(pid_file)}; fifo={fifo.exists()}")
        finally:
            run(directory, "stop")
    return finish()


def finish() -> int:
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("jobserver self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
