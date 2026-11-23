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
import re
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

SCRIPTS = Path(__file__).resolve().parent.parent
REPO_ROOT = SCRIPTS.parent
BINDGEN = SCRIPTS / "ffi-bindgen.sh"
SANDBOXES = REPO_ROOT / "target" / "tmp"
SANDBOXES.mkdir(parents=True, exist_ok=True)

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

    environment = dict(os.environ)
    environment.pop("RUSTUP_TOOLCHAIN", None)

    with tempfile.TemporaryDirectory(dir=SANDBOXES) as directory:
        out = Path(directory) / "bindings"
        result = patience.run(
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

        case("the Swift declares the exported function",
             "public func probe() -> String" in text,
             "no `public func probe() -> String` in the generated Swift; the "
             "export did not cross, or its signature changed without this "
             "test being updated")

        # The generator is built from this workspace because uniffi writes a per-function checksum
        # into the Swift and compares it at startup; another version fatalErrors on launch.
        case("the Swift carries the API checksum guard",
             "uniffi_postio_ffi_checksum_func_probe()" in text
             and "UniFFI API checksum mismatch" in text,
             "no checksum guard, so a version-skewed generator would produce "
             "Swift that fails at runtime instead of at build time")

        case("the event drain crosses as Swift async",
             "func nextEvent() async" in text,
             "nextEvent() is not async in the generated Swift; the drain would "
             "have to be hand-wrapped on the Swift side")

        # Visibility, asserted on the token not the line: uniffi 0.32 puts `public` on its own line
        # above the declaration.
        for error in ("SessionError", "SettingsError"):
            declaration = re.search(
                r"\bpublic\s+enum\s+" + error + r"\b", text
            )
            case(f"{error} is public in the generated Swift",
                 declaration is not None,
                 f"{error} is not declared public, so Swift outside this "
                 f"module cannot name it in a `catch`")

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
