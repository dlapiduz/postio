#!/usr/bin/env python3
"""Self-test for #571: the Swift bindings must be generatable, on any host.

`postio-ffi` is the boundary the macOS application talks to (ADR 0019). Its
Swift side is a build product, not a tracked file: the generator and the
`uniffi` runtime have to be the same version or the Swift compiles against
nothing, and building both from this workspace every time is the only way to
guarantee that. `scripts/ffi-bindgen.sh` is that build step.

The chain has four links, and a break in any of them is silent until somebody
opens Xcode:

  * the crate declares a `cdylib`, because library-mode generation reads its
    metadata rather than the source;
  * the in-workspace `uniffi-bindgen` binary builds and runs;
  * it emits Swift, a C header and a module map;
  * the Swift actually declares what Rust exported.

Written after the script rather than before it, so the assertions are pointed
at behaviour that can regress rather than at the script's existence: the
function's exact Swift signature, and the checksum guard that is the whole
reason the version-lock matters. Asserting "three files appeared" would pass
against a generator that emitted three empty files.

This runs on Linux and macOS alike. That is the point of it: the macOS seam is
proven on the cheap platform.

Usage: scripts/tests/test-ffi-bindgen.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
REPO_ROOT = SCRIPTS.parent
BINDGEN = SCRIPTS / "ffi-bindgen.sh"

FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def main() -> int:
    case("the generator script exists and is executable",
         BINDGEN.is_file() and os.access(BINDGEN, os.X_OK),
         f"{BINDGEN} is missing or not executable")
    if not BINDGEN.is_file():
        print("- cannot continue without the script", file=sys.stderr)
        return 1

    # A private out-dir under the repo, so nothing lands in a place a later
    # `git add -A` would pick up, and RUSTUP_TOOLCHAIN cleared for the same
    # reason `issue-land.sh` clears it: this workstation exports one that beats
    # rust-toolchain.toml.
    environment = dict(os.environ)
    environment.pop("RUSTUP_TOOLCHAIN", None)

    with tempfile.TemporaryDirectory(dir=REPO_ROOT) as directory:
        out = Path(directory) / "bindings"
        result = subprocess.run(
            ["bash", str(BINDGEN), str(out)],
            cwd=REPO_ROOT,
            env=environment,
            capture_output=True,
            text=True,
            timeout=900,
        )
        case("the generator runs to completion",
             result.returncode == 0,
             f"exit {result.returncode}\n--- stdout ---\n{result.stdout}"
             f"\n--- stderr ---\n{result.stderr}")
        if result.returncode != 0:
            print("- cannot check the output of a run that failed", file=sys.stderr)
            for failure in FAILURES:
                print(f"- {failure}", file=sys.stderr)
            return 1

        swift = out / "postio_ffi.swift"
        header = out / "postio_ffiFFI.h"
        modulemap = out / "postio_ffiFFI.modulemap"

        for path in (swift, header, modulemap):
            case(f"{path.name} was written",
                 path.is_file() and path.stat().st_size > 0,
                 f"{path} missing or empty")
        if not swift.is_file():
            for failure in FAILURES:
                print(f"- {failure}", file=sys.stderr)
            return 1

        text = swift.read_text(encoding="utf-8")

        # What Rust exported has to appear in Swift with the type it exported.
        # `String` rather than a buffer type is the marshalling working.
        case("the Swift declares the exported function",
             "public func probe() -> String" in text,
             "no `public func probe() -> String` in the generated Swift; the "
             "export did not cross, or its signature changed without this "
             "test being updated")

        # The reason the generator is built from this workspace rather than
        # installed: uniffi writes a per-function checksum into the Swift and
        # compares it against the library at startup. A generator of a
        # different version produces a Swift that fatalErrors on launch.
        case("the Swift carries the API checksum guard",
             "uniffi_postio_ffi_checksum_func_probe()" in text
             and "UniFFI API checksum mismatch" in text,
             "no checksum guard, so a version-skewed generator would produce "
             "Swift that fails at runtime instead of at build time")

        # The reason this boundary is UniFFI and not a hand-written C ABI:
        # the drain has to arrive in Swift as `async`, so the frontend can
        # write `while let e = await session.nextEvent()` on the main actor.
        # A synchronous signature here would mean a callback, a continuation
        # and manual cancellation on the far side -- the machinery that
        # produces "the UI froze". ADR 0019 Q3.
        case("the event drain crosses as Swift async",
             "func nextEvent() async" in text,
             "nextEvent() is not async in the generated Swift; the drain would "
             "have to be hand-wrapped on the Swift side")

        # The module map is what makes `import` work from Swift at all.
        case("the module map names a module Swift can import",
             "module postio_ffiFFI" in modulemap.read_text(encoding="utf-8"),
             "the module map does not declare postio_ffiFFI")

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("ffi-bindgen check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
