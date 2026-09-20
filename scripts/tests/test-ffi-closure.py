#!/usr/bin/env python3
"""Prove `scripts/lib/ffi-closure.py` derives its answer instead of knowing it.

The helper decides whether a Rust change obliges the fourteen-minute macOS
job (#1449), and the whole argument for it is that the three crates outside
`postio-ffi`'s reach are *computed* rather than written down. A hand-listed
version would pass every test here on the day it was written and go on
skipping the macOS runner for a crate somebody later wired into the bindings.

So the cases below do not check for `postio-app`, `postio-gtk` and
`postio-bench`. They build workspaces whose graphs differ from this one and
check the answer moved with the graph -- which is the only way to tell a
derivation from a lookup table.

Usage: scripts/tests/test-ffi-closure.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / "scripts" / "lib" / "ffi-closure.py"
FAILURES: list[str] = []


def build(tmp: Path, graph: dict[str, list[str]]) -> Path:
    """A workspace of `name -> [workspace dependency names]`, manifests only."""
    shutil.copytree(HELPER.parent, tmp / "scripts" / "lib")
    for name, deps in graph.items():
        crate = tmp / "crates" / name
        crate.mkdir(parents=True)
        body = [f'[package]\nname = "{name}"\nversion = "0.1.0"\n', "[dependencies]"]
        # All three spellings the real manifests use, rotated, so a parser that
        # only understands one of them fails here rather than in CI.
        for index, dep in enumerate(deps):
            body.append(
                [f'{dep} = {{ path = "../{dep}" }}',
                 f"{dep}.workspace = true",
                 f'{dep} = {{ workspace = true, features = ["x"] }}'][index % 3]
            )
        (crate / "Cargo.toml").write_text("\n".join(body) + "\n")
    return tmp


def outside(tmp: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(tmp / "scripts" / "lib" / "ffi-closure.py")],
        capture_output=True, text=True,
    )


def case(name: str, graph: dict[str, list[str]], want: list[str]) -> None:
    with tempfile.TemporaryDirectory() as raw:
        tmp = build(Path(raw), graph)
        result = outside(tmp)
        got = result.stdout.split()
        ok = result.returncode == 0 and got == want
        print(f"{'ok   ' if ok else 'FAIL '} {name}")
        if not ok:
            FAILURES.append(f"{name}: want {want}, got {got} (exit {result.returncode})")


def failing(name: str, graph: dict[str, list[str]], mutate=None) -> None:
    """A tree the helper must refuse rather than answer for."""
    with tempfile.TemporaryDirectory() as raw:
        tmp = build(Path(raw), graph)
        if mutate:
            mutate(tmp)
        result = outside(tmp)
        ok = result.returncode != 0
        print(f"{'ok   ' if ok else 'FAIL '} {name}  (exit {result.returncode})")
        if not ok:
            FAILURES.append(f"{name}: answered {result.stdout!r} for a tree it cannot place")


def main() -> int:
    # The shape this exists for: a frontend the boundary crate cannot reach.
    case(
        "a crate nothing depends on is outside",
        {"postio-ffi": ["postio-core"], "postio-core": [], "postio-frontend": ["postio-core"]},
        ["postio-frontend"],
    )
    # The derivation, proven: the same three crates, one edge added, and the
    # answer changes. A hard-coded list cannot pass both this and the case
    # above.
    case(
        "an edge into it brings it inside",
        {"postio-ffi": ["postio-core", "postio-frontend"], "postio-core": [], "postio-frontend": []},
        [],
    )
    # Transitively, not just one hop.
    case(
        "reach is transitive",
        {"postio-ffi": ["postio-a"], "postio-a": ["postio-b"], "postio-b": ["postio-c"],
         "postio-c": [], "postio-far": []},
        ["postio-far"],
    )
    # A dev-dependency counts. It cannot change the generated bindings, but a
    # larger closure is the safe direction and the helper says so.
    case(
        "a dev-dependency still counts as reach",
        {"postio-ffi": ["postio-tool"], "postio-tool": []},
        [],
    )
    # A cycle must not hang or double-count.
    case(
        "a cycle terminates",
        {"postio-ffi": ["postio-a"], "postio-a": ["postio-b"], "postio-b": ["postio-a"],
         "postio-lonely": []},
        ["postio-lonely"],
    )
    # The directory is what a changed path carries, and it is only equal to the
    # package name by convention.
    def rename(tmp: Path) -> None:
        (tmp / "crates" / "postio-odd").rename(tmp / "crates" / "some-directory")
    with tempfile.TemporaryDirectory() as raw:
        tmp = build(Path(raw), {"postio-ffi": [], "postio-odd": []})
        rename(tmp)
        result = outside(tmp)
        ok = result.returncode == 0 and result.stdout.split() == ["some-directory"]
        print(f"{'ok   ' if ok else 'FAIL '} the directory is reported, not the package name")
        if not ok:
            FAILURES.append(f"directory case: got {result.stdout!r}")

    # Refusals. Each of these would otherwise produce an empty or short list,
    # which the caller reads as "everything obliges macOS" -- safe, but silent.
    # A non-zero exit is what makes it visible instead.
    failing("a workspace with no postio-ffi", {"postio-core": []})
    failing(
        "a manifest with no [package]",
        {"postio-ffi": []},
        lambda tmp: (tmp / "crates" / "postio-ffi" / "Cargo.toml").write_text("[dependencies]\n"),
    )
    failing(
        "no crates/ at all",
        {"postio-ffi": []},
        lambda tmp: shutil.rmtree(tmp / "crates"),
    )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("ffi-closure self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
