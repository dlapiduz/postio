#!/usr/bin/env python3
"""Prove `check-lint-floor.py` fails when the floor is actually violated.

A guard that has never been seen to fail is a guard nobody should trust. The
boundary and tracking checks each have one of these for the same reason; this
is the lint floor's.

Each case copies the workspace manifest and the crate manifests into a
temporary tree, breaks exactly one thing, and asserts the check notices.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CHECK = ROOT / "scripts" / "checks" / "check-lint-floor.py"


def sandbox(tmp: Path) -> Path:
    """A minimal copy of the workspace: manifests only, which is all the
    check reads."""
    shutil.copy(ROOT / "Cargo.toml", tmp / "Cargo.toml")
    (tmp / "scripts" / "checks").mkdir(parents=True)
    shutil.copy(CHECK, tmp / "scripts" / "checks" / "check-lint-floor.py")
    for manifest in (ROOT / "crates").glob("*/Cargo.toml"):
        target = tmp / "crates" / manifest.parent.name
        target.mkdir(parents=True)
        shutil.copy(manifest, target / "Cargo.toml")
    return tmp


def run(tmp: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(tmp / "scripts" / "checks" / "check-lint-floor.py")],
        capture_output=True,
        text=True,
    )


def case(name: str, mutate) -> bool:
    with tempfile.TemporaryDirectory() as raw:
        tmp = sandbox(Path(raw))
        mutate(tmp)
        result = run(tmp)
        if result.returncode == 0:
            print(f"FAIL  {name}: check passed on a broken tree", file=sys.stderr)
            print(result.stdout, file=sys.stderr)
            return False
        print(f"ok    {name}  (exit {result.returncode})")
        return True


def drop_inheritance(tmp: Path) -> None:
    """A crate that forgets `[lints] workspace = true` -- the silent case."""
    p = tmp / "crates" / "postio-model" / "Cargo.toml"
    p.write_text(p.read_text().replace("[lints]\nworkspace = true\n", ""))


def weaken_the_floor(tmp: Path) -> None:
    """The workspace table downgraded from forbid to deny."""
    p = tmp / "Cargo.toml"
    p.write_text(p.read_text().replace('unsafe_code = "forbid"', 'unsafe_code = "deny"'))


def weaken_an_exception(tmp: Path) -> None:
    """An audited crate quietly allowing unsafe outright."""
    p = tmp / "crates" / "postio-gtk" / "Cargo.toml"
    p.write_text(p.read_text().replace('unsafe_code = "deny"', 'unsafe_code = "allow"'))


def exception_stops_declaring(tmp: Path) -> None:
    """An exception with no [lints.rust] table at all inherits nothing."""
    p = tmp / "crates" / "postio-app" / "Cargo.toml"
    text = p.read_text()
    head = text.split("[lints.rust]")[0]
    p.write_text(head)


def main() -> int:
    baseline = run(sandbox(Path(tempfile.mkdtemp())))
    if baseline.returncode != 0:
        print("FAIL  the unmodified tree does not pass:", file=sys.stderr)
        print(baseline.stdout + baseline.stderr, file=sys.stderr)
        return 1
    print("ok    baseline: the real tree passes")

    passed = all(
        [
            case("a crate drops [lints] workspace = true", drop_inheritance),
            case("the workspace floor is weakened to deny", weaken_the_floor),
            case("an audited crate weakens to allow", weaken_an_exception),
            case("an audited crate declares no lints", exception_stops_declaring),
        ]
    )
    print("\nall cases behaved" if passed else "\nsome cases did not fail", file=sys.stderr if not passed else sys.stdout)
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
