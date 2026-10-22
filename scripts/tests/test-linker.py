#!/usr/bin/env python3
"""Self-test for scripts/linker.sh, the `postio-linker` shim.

A test binary links the crate's whole dependency stack, and close to a third
of it is the symbol table: `ffi_suite` is 124 MB as linked and 85 MB
stripped. Nothing reads those symbols but a `RUST_BACKTRACE` dump, and a
failing test's own message carries its `file:line` as data. So the linker
strips test binaries, and only those:

  * a test binary is one cargo builds with `CARGO_TARGET_TMPDIR` set (an
    integration test or bench) or one that links libtest (a unit-test
    binary); an application or library is linked untouched;
  * on Linux the linker strips (`-Wl,--strip-all`); on macOS ld64 cannot
    drop global symbols, so `strip` runs on the output after a link that
    succeeded, which is what rustc's own `strip = "symbols"` does there;
  * `POSTIO_KEEP_SYMBOLS=1` keeps them, for a backtrace with names in it;
  * mold is still used when present, exactly as before.

The cases run on a fake `cc`, `strip`, `mold` and `uname`, so both platforms
are exercised wherever this runs.

Usage: scripts/tests/test-linker.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
LINKER = HERE / "linker.sh"

FAILURES: list[str] = []

LOG = """#!/usr/bin/env bash
printf '%s %s\\n' "$(basename "$0")" "$*" >> "$POSTIO_TEST_LOG"
"""


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def fakes(root: Path, system: str, *, mold: bool = False, cc_status: int = 0) -> Path:
    bin_dir = root / "bin"
    bin_dir.mkdir(parents=True)
    tools = {
        "cc": LOG + f"exit {cc_status}\n",
        "strip": LOG,
        "uname": f"#!/usr/bin/env bash\necho {system}\n",
    }
    if mold:
        tools["mold"] = LOG
    for name, text in tools.items():
        path = bin_dir / name
        path.write_text(text, encoding="utf-8")
        path.chmod(0o755)
    return bin_dir


def link(root: Path, system: str, args: list[str], *, env: dict[str, str] | None = None,
         mold: bool = False, cc_status: int = 0) -> tuple[int, list[str]]:
    bin_dir = fakes(root, system, mold=mold, cc_status=cc_status)
    log = root / "log"
    log.touch()
    environment = {
        "PATH": f"{bin_dir}:/usr/bin:/bin",
        "POSTIO_TEST_LOG": str(log),
        "HOME": str(root),
    }
    environment.update(env or {})
    result = subprocess.run(["bash", str(LINKER), *args], env=environment,
                            capture_output=True, text=True)
    return result.returncode, log.read_text(encoding="utf-8").splitlines()


def main() -> int:
    plain = ["main.o", "-o", "/t/deps/postio-1a2b"]
    unit = ["main.o", "/sysroot/lib/libtest-9f8e7d.rlib", "-o", "/t/deps/postio_core-3c4d"]
    tmpdir = {"CARGO_TARGET_TMPDIR": "/t/tmp"}

    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)

        status, calls = link(base / "l-int", "Linux", plain, env=tmpdir)
        case("Linux strips an integration test as it links",
             status == 0 and calls == ["cc " + " ".join(plain) + " -Wl,--strip-all"],
             f"exit {status}, calls {calls}")

        status, calls = link(base / "l-unit", "Linux", unit)
        case("Linux strips a unit-test binary, known by libtest",
             calls == ["cc " + " ".join(unit) + " -Wl,--strip-all"],
             f"calls {calls}")

        status, calls = link(base / "l-app", "Linux", plain)
        case("Linux links an application untouched",
             calls == ["cc " + " ".join(plain)],
             f"calls {calls}")

        status, calls = link(base / "l-mold", "Linux", plain, env=tmpdir, mold=True)
        case("Linux still links with mold when it is there",
             calls == ["cc " + " ".join(plain) + " -Wl,--strip-all -fuse-ld=mold"],
             f"calls {calls}")

        status, calls = link(base / "d-int", "Darwin", plain, env=tmpdir)
        case("macOS strips a test binary after the link",
             status == 0 and calls == ["cc " + " ".join(plain), "strip /t/deps/postio-1a2b"],
             f"exit {status}, calls {calls}")

        status, calls = link(base / "d-unit", "Darwin", unit)
        case("macOS strips a unit-test binary, known by libtest",
             calls == ["cc " + " ".join(unit), "strip /t/deps/postio_core-3c4d"],
             f"calls {calls}")

        status, calls = link(base / "d-app", "Darwin", plain)
        case("macOS links an application untouched",
             calls == ["cc " + " ".join(plain)],
             f"calls {calls}")

        status, calls = link(base / "d-fail", "Darwin", plain, env=tmpdir, cc_status=3)
        case("a failed link is reported as it failed, and nothing is stripped",
             status == 3 and calls == ["cc " + " ".join(plain)],
             f"exit {status}, calls {calls}")

        keep = {**tmpdir, "POSTIO_KEEP_SYMBOLS": "1"}
        status, calls = link(base / "l-keep", "Linux", plain, env=keep)
        case("POSTIO_KEEP_SYMBOLS=1 keeps them on Linux",
             calls == ["cc " + " ".join(plain)],
             f"calls {calls}")
        status, calls = link(base / "d-keep", "Darwin", plain, env=keep)
        case("POSTIO_KEEP_SYMBOLS=1 keeps them on macOS",
             calls == ["cc " + " ".join(plain)],
             f"calls {calls}")

    for failure in FAILURES:
        print(f"FAIL  {failure}", file=sys.stderr)
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("linker self-test passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
