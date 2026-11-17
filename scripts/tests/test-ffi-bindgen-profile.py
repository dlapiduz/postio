#!/usr/bin/env python3
"""Self-test: `ffi-bindgen.sh --release` reads the release library.

`macos-build.sh --release` builds `postio-ffi` in release and then generates
the Swift from it. The generator used to build and read a *debug* library
whatever its caller had built, so every release of the Mac app compiled the
workspace a second time in debug, for twenty minutes, to read metadata the
release library already carried.

`cargo` is stubbed: it records what it was asked and leaves a library where
a build of that profile would, so this costs nothing to run.
`test-ffi-bindgen.py` runs the real generator.

Usage: scripts/tests/test-ffi-bindgen-profile.py [script]
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

SCRIPTS = Path(__file__).resolve().parent.parent
BINDGEN = Path(sys.argv[1]) if len(sys.argv) > 1 else SCRIPTS / "ffi-bindgen.sh"

FAILURES: list[str] = []

CARGO_STUB = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/cargo-args"
profile=debug
for arg in "$@"; do [ "$arg" = --release ] && profile=release; done
if [ "$1" = build ]; then
    mkdir -p "$CARGO_TARGET_DIR/$profile"
    touch "$CARGO_TARGET_DIR/$profile/libpostio_ffi.so"
fi
exit 0
"""


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def generate(*args: str) -> tuple[int, list[str], str]:
    with tempfile.TemporaryDirectory() as tmp:
        stub = Path(tmp)
        (stub / "bin").mkdir()
        cargo = stub / "bin" / "cargo"
        cargo.write_text(CARGO_STUB, encoding="utf-8")
        cargo.chmod(0o755)
        env = {
            **os.environ,
            "PATH": f"{stub / 'bin'}:{os.environ['PATH']}",
            "STUB_DIR": str(stub),
            "CARGO_TARGET_DIR": str(stub / "target"),
        }
        env.pop("POSTIO_FFI_FEATURES", None)
        done = patience.run(
            ["bash", str(BINDGEN), *args, str(stub / "out")],
            env=env, capture_output=True, text=True, timeout=60,
        )
        log = stub / "cargo-args"
        calls = log.read_text().splitlines() if log.exists() else []
        return done.returncode, calls, done.stderr


def main() -> int:
    status, calls, err = generate("--release")
    builds = [c for c in calls if c.startswith("build")]
    runs = [c for c in calls if c.startswith("run")]
    case("--release generates", status == 0, f"exit {status}: {err}")
    case("--release builds the library in release, and nothing in debug",
         bool(builds) and all("--release" in c.split() for c in builds),
         f"cargo was asked: {calls}")
    case("--release runs the generator in release, on the release library",
         bool(runs) and all("--release" in c.split() and "/release/libpostio_ffi" in c for c in runs),
         f"cargo was asked: {calls}")

    status, calls, err = generate()
    case("with no flag it is the debug library, as before",
         status == 0 and calls and not any("--release" in c.split() for c in calls)
         and any("/debug/libpostio_ffi" in c for c in calls),
         f"exit {status}, cargo was asked: {calls}, {err}")

    if FAILURES:
        print("\nFAILED:", *FAILURES, sep="\n  ")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
