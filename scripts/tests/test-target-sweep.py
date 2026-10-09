#!/usr/bin/env python3
"""Self-test for scripts/target-sweep.py.

A sweep that deletes something live costs a rebuild nobody connects to it;
one that deletes nothing is just slow to notice. This builds a throwaway
workspace whose one dependency is live in two variants at once -- a normal
dependency and a build-dependency asking for different features, which is
exactly what "newest per name" would get wrong -- then changes the normal
dependency's features so the old variant is superseded, and asserts:

  * the superseded variant goes, both live variants stay, and the sweep's
    own re-run finds every unit fresh;
  * `--dry-run` deletes nothing;
  * a measuring build that fails deletes nothing.

No network: the dependency is a path crate the workspace `exclude`s.

Usage: scripts/tests/test-target-sweep.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
SWEEP = HERE / "target-sweep.py"
FAILURES: list[str] = []
RLIB = re.compile(r"^libdep-([0-9a-f]{16})\.rlib$")


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def app_manifest(features: str) -> str:
    return (
        '[package]\nname = "app"\nversion = "0.1.0"\nedition = "2021"\n\n'
        f'[dependencies]\ndep = {{ path = "../../dep", features = [{features}] }}\n\n'
        '[build-dependencies]\ndep = { path = "../../dep", features = ["g"] }\n'
    )


def fixture(root: Path) -> Path:
    ws = root / "ws"
    write(ws / "Cargo.toml", '[workspace]\nresolver = "2"\nmembers = ["app"]\n')
    write(ws / "app" / "Cargo.toml", app_manifest('"f"'))
    write(ws / "app" / "build.rs", "fn main() { dep::hello(); }\n")
    # A binary and an example: cargo reports them by their uplifted,
    # unhashed copies, which is what a first draft of the sweep missed.
    write(ws / "app" / "src" / "main.rs", "fn main() { app::go() }\n")
    write(ws / "app" / "examples" / "ex.rs", "fn main() { app::go() }\n")
    write(ws / "app" / "src" / "lib.rs",
          "//! Fixture.\n\n/// Calls the dependency.\npub fn go() { dep::hello() }\n\n"
          "#[cfg(test)]\nmod tests {\n    #[test]\n    fn runs() { super::go() }\n}\n")
    write(root / "dep" / "Cargo.toml",
          '[package]\nname = "dep"\nversion = "0.1.0"\nedition = "2021"\n\n'
          "[features]\nf = []\ng = []\nh = []\n")
    write(root / "dep" / "src" / "lib.rs", "pub fn hello() {}\n")
    return ws


def cargo(ws: Path, *args: str) -> None:
    subprocess.run(["cargo", *args], cwd=ws, check=True, capture_output=True)


def rlib_hashes(ws: Path) -> set[str]:
    deps = ws / "target" / "debug" / "deps"
    return {m[1] for n in os.listdir(deps) if (m := RLIB.match(n))}


def sweep(ws: Path, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run([sys.executable, str(SWEEP), "--tree", str(ws), *args],
                          capture_output=True, text=True)


def expect(case: str, cond: bool, detail: str, proc: subprocess.CompletedProcess | None = None) -> None:
    if cond:
        print(f"  ok   {case}: {detail}")
        return
    print(f"  FAIL {case}: {detail}")
    if proc is not None:
        print(proc.stdout + proc.stderr)
    FAILURES.append(f"{case}: {detail}")


def main() -> int:
    os.environ.pop("RUSTUP_TOOLCHAIN", None)
    with tempfile.TemporaryDirectory(prefix="postio-sweep-") as tmp:
        tmp_path = Path(tmp)

        print("case: a superseded variant goes and both live ones stay")
        ws = fixture(tmp_path / "a")
        cargo(ws, "test", "--workspace", "--no-run")
        first = rlib_hashes(ws)
        write(ws / "app" / "Cargo.toml", app_manifest('"f", "h"'))
        cargo(ws, "test", "--workspace", "--no-run")
        both = rlib_hashes(ws)
        expect("setup", len(first) == 2 and len(both) == 3,
               f"two live variants, then a third ({len(first)} then {len(both)} rlibs)")
        proc = sweep(ws)
        left = rlib_hashes(ws)
        expect("sweep", proc.returncode == 0, "exits 0 and verifies every unit fresh", proc)
        expect("sweep", len(left) == 2, f"two dep rlibs left (found {len(left)})", proc)
        expect("sweep", len(left & first) == 1,
               "the build-dependency's variant, unchanged by the edit, survived", proc)
        expect("sweep", not (first - left - both) and len((first & both) - left) == 1,
               "the superseded normal-dependency variant is gone", proc)

        print("case: --dry-run deletes nothing")
        ws = fixture(tmp_path / "b")
        cargo(ws, "test", "--workspace", "--no-run")
        write(ws / "app" / "Cargo.toml", app_manifest('"f", "h"'))
        cargo(ws, "test", "--workspace", "--no-run")
        proc = sweep(ws, "--dry-run")
        expect("dry run", proc.returncode == 0 and len(rlib_hashes(ws)) == 3,
               "exits 0 and all three rlibs remain", proc)
        expect("dry run", "would free" in proc.stdout, "says what it would free", proc)

        print("case: a failing measuring build deletes nothing")
        write(ws / "app" / "src" / "lib.rs", "pub fn go( {\n")
        proc = sweep(ws)
        expect("broken build", proc.returncode == 2 and len(rlib_hashes(ws)) == 3,
               f"exits 2 and all three rlibs remain (exit {proc.returncode})", proc)

    if FAILURES:
        print(f"\n{len(FAILURES)} assertion(s) failed", file=sys.stderr)
        return 1
    print("\nall cases behaved")
    return 0


if __name__ == "__main__":
    sys.exit(main())
