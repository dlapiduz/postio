#!/usr/bin/env python3
"""Self-test for scripts/checks/check-workspace-hack.py.

A guard that has never been seen to fail is not a guard. This builds a
throwaway workspace -- two members asking one shared crate for different
features, and a hack `cargo hakari` generated for them -- and asserts that
the check passes while the hack is in step and fails, naming the fix, on
both ways it falls behind: a member's features change without the hack
being regenerated, and a new member arrives without its line.

The shared crate is a path dependency the workspace `exclude`s -- one inside
the workspace directory would become a member, which hakari never unifies --
so it counts as third-party and nothing is fetched. The real manifests are never
touched; the last case reads the real workspace.

Usage: scripts/tests/test-check-workspace-hack.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-workspace-hack.py"
REPO_ROOT = HERE.parent

sys.path.insert(0, str(HERE / "lib"))

import prereq  # noqa: E402  -- enabled by the sys.path line above

HACK = "fixture-hack"
FAILURES: list[str] = []


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def member(root: Path, name: str, features: str) -> None:
    write(root / "crates" / name / "src" / "lib.rs", "")
    write(
        root / "crates" / name / "Cargo.toml",
        f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2021"\n\n'
        f"[dependencies]\n"
        f'shared = {{ path = "../../shared", default-features = false, features = [{features}] }}\n',
    )


def hakari(root: Path, *args: str) -> None:
    subprocess.run(["cargo", "hakari", *args], cwd=root, check=True,
                   capture_output=True, text=True)


def fixture(root: Path) -> Path:
    """A workspace whose hack is in step: `a` asks for `one`, `b` for `two`."""
    write(root / "Cargo.toml",
          '[workspace]\nresolver = "2"\nmembers = ["crates/*"]\nexclude = ["shared"]\n')
    write(root / "shared" / "src" / "lib.rs", "")
    write(root / "shared" / "Cargo.toml",
          '[package]\nname = "shared"\nversion = "0.1.0"\nedition = "2021"\n\n'
          '[features]\none = []\ntwo = []\nthree = []\n')
    member(root, "a", '"one"')
    member(root, "b", '"two"')
    hakari(root, "init", f"crates/{HACK}", "--package-name", HACK, "--yes")
    hakari(root, "generate")
    hakari(root, "manage-deps", "--yes")
    return root


def run(root: Path) -> subprocess.CompletedProcess:
    return subprocess.run([sys.executable, str(CHECK), "--root", str(root)],
                          capture_output=True, text=True)


def expect(case: str, root: Path, status: int, *needles: str) -> None:
    print(f"case: {case}")
    proc = run(root)
    output = proc.stdout + proc.stderr
    ok = proc.returncode == status and all(n in output for n in needles)
    if ok:
        print(f"  ok   exit {status}" + "".join(f", names {n!r}" for n in needles))
        return
    print(f"  FAIL exit {proc.returncode} (expected {status}); wanted {needles!r}\n{output}")
    FAILURES.append(case)


def main() -> int:
    if not prereq.available("cargo-hakari", present=shutil.which("cargo-hakari") is not None):
        return 0
    with tempfile.TemporaryDirectory(prefix="postio-hack-") as tmp:
        tmp_path = Path(tmp)

        expect("a hack in step passes", fixture(tmp_path / "clean"), 0)

        # The trap: `b` now asks for `three`, which compiles and passes, and
        # the hack still says `one` and `two`.
        drifted = fixture(tmp_path / "drifted")
        member(drifted, "b", '"two", "three"')
        hakari(drifted, "manage-deps", "--yes")  # the line itself stays in step
        expect("a member's features change and the hack is not regenerated",
               drifted, 1, "contents are out of date", "cargo hakari generate")

        # A member added by hand, without the line naming the hack.
        newcomer = fixture(tmp_path / "newcomer")
        member(newcomer, "c", '"one"')
        expect("a new member arrives without its line", newcomer, 1,
               "missing or stale", "manage-deps")

    expect("the real workspace is in step", REPO_ROOT, 0)

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) misbehaved: {', '.join(FAILURES)}", file=sys.stderr)
        return 1
    print("\nall cases behaved")
    return 0


if __name__ == "__main__":
    sys.exit(main())
