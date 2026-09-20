#!/usr/bin/env python3
"""Print the workspace crates that `postio-ffi` cannot reach, one per line.

`scripts/ci-changes.sh` uses this to decide whether a diff obliges the macOS
runner. A macOS job is the only thing that compiles the Swift half and proves
the link, and the Swift compiles against bindings generated from `postio-ffi`
-- so a Rust change reaches it only if it can reach that crate.

Seventeen of the twenty crates are in `postio-ffi`'s closure. The three that
are not -- `postio-app`, `postio-gtk`, `postio-bench` -- are also the most
edited: 78 of the last 200 commits on `main` touched one of them, and every
one of those started a fourteen-minute macOS job that no binding change could
possibly have needed (#1449).

# Derived, never listed

The three names are *not* written down here, and must not be. A
hand-maintained list of "the crates Swift cannot see" fails in the direction
nobody notices: a crate added to `postio-ffi`'s graph keeps skipping the macOS
runner because somebody forgot a line, and the branch that breaks the seam is
green. The closure is computed from the manifests every run, so the answer
moves when the graph does.

# Manifests, not `cargo metadata`

`cargo metadata` would answer this too, and more robustly. It also needs a
toolchain, and the `changes` job in `ci.yml` deliberately has none -- it is
checkout, one `gh api` call and two scripts, and it finishes in six seconds
while every other job waits on it. `rust-toolchain.toml` would make the first
`cargo` invocation there install the pinned toolchain, which is minutes on the
job that gates the pipeline.

So this reads the manifests directly. It only ever needs dependency *names*,
which is the one thing every spelling agrees on:

    postio-model = { workspace = true }
    postio-index.workspace = true
    postio-search = { version = "0.2.0", path = "../postio-search" }

`tomllib` gives all three as the same key, so nothing here parses TOML by
hand. Dev- and build-dependencies count too: they cannot change the generated
bindings, but including them can only ever make the closure larger, and larger
is the safe direction.

Usage: scripts/lib/ffi-closure.py [--root <repo>]
Output: one crate directory name per line, sorted. Empty output is a valid
answer only if every crate is reachable; a *failure* exits non-zero, and the
caller must treat that as "everything obliges macOS" rather than as an empty
list.
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path

ROOT_CRATE = "postio-ffi"
DEP_SECTIONS = ("dependencies", "dev-dependencies", "build-dependencies")


def main() -> int:
    root = Path(__file__).resolve().parent.parent.parent
    if len(sys.argv) > 2 and sys.argv[1] == "--root":
        root = Path(sys.argv[2])

    crates_dir = root / "crates"
    if not crates_dir.is_dir():
        print(f"no crates/ under {root}", file=sys.stderr)
        return 1

    # name -> (directory, workspace dependency names)
    graph: dict[str, tuple[str, set[str]]] = {}
    for manifest in sorted(crates_dir.glob("*/Cargo.toml")):
        parsed = tomllib.loads(manifest.read_text())
        package = parsed.get("package", {}).get("name")
        if not package:
            # A manifest with no `[package]` is not a crate this can place, and
            # guessing from the directory name is exactly the kind of inference
            # that makes a stale answer look authoritative.
            print(f"{manifest} has no [package] name", file=sys.stderr)
            return 1
        deps: set[str] = set()
        for section in DEP_SECTIONS:
            deps |= {
                name
                for name in parsed.get(section, {})
                # Only workspace-local crates: an external crate cannot be a
                # path under `crates/`, so it can never be a changed file this
                # classifier has to place.
                if name.startswith("postio-")
            }
        graph[package] = (manifest.parent.name, deps)

    if ROOT_CRATE not in graph:
        print(f"{ROOT_CRATE} is not in the workspace", file=sys.stderr)
        return 1

    reachable = {ROOT_CRATE}
    frontier = [ROOT_CRATE]
    while frontier:
        for dependency in graph.get(frontier.pop(), ("", set()))[1]:
            if dependency not in reachable:
                reachable.add(dependency)
                frontier.append(dependency)

    # Directory names, because a changed path is `crates/<dir>/...` and the
    # two are only equal by convention.
    for name in sorted(graph):
        if name not in reachable:
            print(graph[name][0])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
