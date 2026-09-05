#!/usr/bin/env python3
"""Self-test for the logging `scripts/rustc-wrapper.sh` gives the daemon (#1184).

`SCCACHE_ERROR_LOG` was added so that when the compile cache goes wrong there
is evidence. It went wrong twice, and both times the log had nothing in it
from the server.

It is not the wrapper failing to pass the variable -- the running daemon's
`/proc/<pid>/environ` has it. It is that `SCCACHE_ERROR_LOG` only says *where*
log records go. **`SCCACHE_LOG` is what decides there are any**, and without
it sccache's server writes nothing at all. Measured on this box with an
isolated daemon on its own port and cache directory:

    SCCACHE_ERROR_LOG only          the file is created, 0 bytes
    SCCACHE_ERROR_LOG + SCCACHE_LOG=info    348 bytes of server lifecycle

The 45 bytes the live log did contain came from a *client* failing to start a
second server ("Address in use"), which is written regardless of level -- so
the instrument looked like it was working, and that is why two occurrences
went by without anyone noticing it was not.

`info` and not `debug`: measured at four lines per server start and **nothing
per compile** -- six compiles produced the same 348 bytes as one -- so it costs
nothing and records the thing a wedge investigation actually wants, which is
when the daemon started and how it was configured.

`sccache` is stubbed on PATH and reports the environment it was handed, so
this runs anywhere, fast, and never starts or stops the real shared daemon.

Usage: scripts/tests/test-rustc-wrapper-logging.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
WRAPPER = HERE / "rustc-wrapper.sh"

REPORT_ENVIRONMENT = """#!/usr/bin/env bash
printf 'SCCACHE_LOG=%s\\n' "${SCCACHE_LOG-<unset>}"
printf 'SCCACHE_ERROR_LOG=%s\\n' "${SCCACHE_ERROR_LOG-<unset>}"
"""

FAILURES: list[str] = []


def run(stub_dir: Path, overrides: dict[str, str]) -> dict[str, str]:
    """Run the wrapper and report the sccache-facing environment it built."""
    environment = dict(os.environ)
    # The stub directory first and the real toolchain dropped, so a real
    # sccache on this machine cannot make a case pass that would fail without
    # one. /usr/bin and /bin stay because the wrapper is a bash script.
    environment["PATH"] = f"{stub_dir}:/usr/bin:/bin"
    for name in ("SCCACHE_LOG", "SCCACHE_ERROR_LOG", "OUT_DIR"):
        environment.pop(name, None)
    environment.update(overrides)
    result = subprocess.run(
        ["bash", str(WRAPPER), "rustc", "--crate-name", "probe", "src/lib.rs"],
        env=environment,
        capture_output=True,
        text=True,
        timeout=30,
    )
    reported = {}
    for line in result.stdout.splitlines():
        name, _, value = line.partition("=")
        reported[name] = value
    return reported


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        stub_dir = Path(raw) / "stubs"
        stub_dir.mkdir()
        sccache = stub_dir / "sccache"
        sccache.write_text(REPORT_ENVIRONMENT)
        sccache.chmod(0o755)

        plain = run(stub_dir, {})
        case(
            "the daemon is given a log level, not just a log path",
            plain.get("SCCACHE_LOG", "<unset>") != "<unset>",
            "SCCACHE_ERROR_LOG names a file and SCCACHE_LOG decides whether "
            "anything is written to it. Without one the server logs nothing "
            "and the evidence #1184 asked for does not exist; got "
            f"{plain.get('SCCACHE_LOG')!r}",
        )
        case(
            "and it is still told where to put it",
            plain.get("SCCACHE_ERROR_LOG", "<unset>") != "<unset>",
            f"got {plain.get('SCCACHE_ERROR_LOG')!r}",
        )

        chosen = run(stub_dir, {"SCCACHE_LOG": "debug"})
        case(
            "a level chosen in the environment still wins",
            chosen.get("SCCACHE_LOG") == "debug",
            "every other setting in this wrapper defers to an explicit one, "
            f"and troubleshooting means raising this; got {chosen.get('SCCACHE_LOG')!r}",
        )

    for failure in FAILURES:
        print(f"FAIL  {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed", file=sys.stderr)
        return 1
    print("\nrustc-wrapper logging: all cases behaved")
    return 0


if __name__ == "__main__":
    sys.exit(main())
