#!/usr/bin/env python3
"""Delete what a worktree's `target/debug` holds that no build of it uses.

Cargo never collects anything. Every dependency bump, rebase, feature change
or flag change leaves the previous artifacts beside the new ones, and a seed
copies a sibling's whole history forward (#1717). Measured on a busy tree
(2026-10-09): 654 of its 667 crates had more than one copy, `html5ever` had
59, and 395 of its 509 test executables were superseded copies -- most of
41 GB.

"Newest per name" is the obvious sweep and the wrong one: several copies of
a crate can be live at once (a build-dependency and a dependency, `check`
and `test`, two feature sets). So this asks cargo instead. Each command
below runs with `--message-format=json`, and cargo reports every unit the
command uses -- the ones it did not need to rebuild too (`"fresh": true`)
-- with the hash that names its files. What no command reports is not used
by any of them, and goes.

The commands are what this tree is built with: the landing gate's
`check --all-targets` and clippy, and the test binaries `test-fast.sh`,
`test-sanity.sh` and nextest run. Run on a tree those have just built (after
a landing, say) the measurement costs seconds; on a stale tree it builds
first, which is the price of knowing.

After deleting, it runs the same commands again and fails unless every unit
is still fresh: a sweep that made the next build rebuild something has
deleted something live, and says so rather than leaving it to be noticed as
slowness.

A crate this host cannot build (`scripts/unbuildable-crates.sh`: the GTK
crates on a Mac) is left out of the measuring builds, so its artifacts go
too -- on this host nothing can use them.

Usage:
  scripts/target-sweep.py [--tree <worktree>] [--dry-run] [--no-verify]
Exit status: 0 swept (or nothing to sweep), 1 the verification rebuilt
something, 2 a measuring build failed and nothing was deleted.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

# What this tree is built with. `--workspace` everywhere: with the
# workspace-hack (ADR 0047) a `-p` build uses the same units, so these cover
# `test-fast.sh`'s per-crate builds as well.
COMMANDS = [
    ["check", "--workspace", "--all-targets"],
    ["clippy", "--workspace", "--all-targets"],
    ["test", "--workspace", "--no-run"],
]

# `<name>-<16 hex>` with an optional extension: how cargo names every unit's
# files, fingerprints, build-script dirs and incremental sessions.
HASHED = re.compile(r"^(?P<stem>.+?)-(?P<hash>[0-9a-f]{16})(?:[.].*)?$")
# Not `incremental/`: rustc names a session directory after the crate's
# stable id (13 base-36 characters), not the unit hash cargo reports, so
# nothing here can tell a live one from a dead one -- and deleting a live one
# costs a full recompile of that crate on the next edit, which the
# verification cannot see. Only workspace crates have sessions, their hashes
# survive source edits, and rustc prunes old sessions inside each directory.
SWEPT_DIRS = ("deps", "build", ".fingerprint", "examples")


class MeasureFailed(Exception):
    """A measuring build failed, so the live set is unknown."""


def measure(tree: Path, excludes: list[str]) -> tuple[set[str], int]:
    """Every unit hash the commands use, and how many units were not fresh."""
    live: set[str] = set()
    uplifted: list[Path] = []
    stale_units = 0
    for args in COMMANDS:
        proc = subprocess.run(
            ["cargo", *args, *excludes, "--message-format=json"],
            cwd=tree, capture_output=True, text=True, check=False,
        )
        if proc.returncode != 0:
            sys.stderr.write(proc.stderr[-4000:])
            raise MeasureFailed(f"`cargo {' '.join(args)}` failed")
        for line in proc.stdout.splitlines():
            if not line.startswith("{"):
                continue
            msg = json.loads(line)
            reason = msg.get("reason")
            paths: list[str] = []
            if reason == "compiler-artifact":
                paths = list(msg.get("filenames") or [])
                if msg.get("executable"):
                    paths.append(msg["executable"])
                if not msg.get("fresh", True):
                    stale_units += 1
            elif reason == "build-script-executed":
                paths = [msg.get("out_dir") or ""]
            for path in paths:
                hashes = [m["hash"] for part in Path(path).parts if (m := HASHED.match(part))]
                live.update(hashes)
                if not hashes:
                    uplifted.append(Path(path))
    live |= uplifted_hashes(tree / "target" / "debug", uplifted)
    return live, stale_units


def uplifted_hashes(debug: Path, uplifted: list[Path]) -> set[str]:
    """The hashes of the files cargo reported under an unhashed name.

    A binary or an example is reported as its uplifted copy --
    `target/debug/postio`, `target/debug/examples/shot` -- which cargo hard
    links to the hashed original in `deps/` or `examples/`. Match by inode.
    Where cargo had to copy instead, keep every hashed file of that name:
    keeping too much costs disk, deleting a live one costs a rebuild.
    """
    wanted = {}
    for path in uplifted:
        try:
            st = path.stat()
        except OSError:
            continue
        wanted[(st.st_dev, st.st_ino)] = path.name
    hashes: set[str] = set()
    found: set[str] = set()
    by_name: dict[str, set[str]] = {}
    for name in ("deps", "examples"):
        root = debug / name
        if not root.is_dir():
            continue
        for entry in root.iterdir():
            m = HASHED.match(entry.name)
            if not m or not entry.is_file():
                continue
            by_name.setdefault(m["stem"], set()).add(m["hash"])
            st = entry.stat()
            if (st.st_dev, st.st_ino) in wanted:
                hashes.add(m["hash"])
                found.add(wanted[(st.st_dev, st.st_ino)])
    for name in set(wanted.values()) - found:
        hashes |= by_name.get(name.replace("-", "_"), set()) | by_name.get(name, set())
    return hashes


def size(path: Path) -> int:
    if path.is_symlink() or path.is_file():
        return path.lstat().st_size
    return sum(p.lstat().st_size for p in path.rglob("*") if p.is_file() and not p.is_symlink())


def sweep(debug: Path, live: set[str], dry_run: bool) -> tuple[int, int]:
    """Delete every hashed entry whose hash no command reported."""
    count = freed = 0
    for name in SWEPT_DIRS:
        root = debug / name
        if not root.is_dir():
            continue
        for entry in root.iterdir():
            m = HASHED.match(entry.name)
            if not m or m["hash"] in live:
                continue
            freed += size(entry)
            count += 1
            if dry_run:
                continue
            if entry.is_dir() and not entry.is_symlink():
                shutil.rmtree(entry)
            else:
                entry.unlink()
    return count, freed


def unbuildable(tree: Path) -> set[str]:
    """What the tree's own `unbuildable-crates.sh` says this host cannot build.

    Measuring with one of them in would fail the build and delete nothing;
    measuring without them means their artifacts are unused here, which on
    this host they are.
    """
    script = tree / "scripts" / "unbuildable-crates.sh"
    if not script.is_file():
        return set()
    out = subprocess.run([str(script)], cwd=tree, capture_output=True, text=True, check=False)
    return {line.strip() for line in out.stdout.splitlines() if line.strip()}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--tree", type=Path, default=Path.cwd())
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--no-verify", action="store_true")
    parser.add_argument(
        "--exclude", action="append", default=[], metavar="CRATE",
        help="leave a crate out of the measuring builds (one this host cannot build); "
             "its artifacts are kept only if another command reports them",
    )
    opts = parser.parse_args(argv)
    tree = opts.tree.resolve()
    debug = tree / "target" / "debug"
    if not debug.is_dir():
        print(f"target-sweep: {debug} does not exist; nothing to sweep.")
        return 0
    os.environ.pop("RUSTUP_TOOLCHAIN", None)  # CLAUDE.md: it overrides the pin
    crates = set(opts.exclude) | unbuildable(tree)
    if crates - set(opts.exclude):
        print(f"target-sweep: this host cannot build {', '.join(sorted(crates - set(opts.exclude)))}; "
              "measuring without them, so their artifacts go too.")
    excludes = [arg for crate in sorted(crates) for arg in ("--exclude", crate)]

    try:
        live, _ = measure(tree, excludes)
    except MeasureFailed as failure:
        print(f"target-sweep: {failure}; nothing was deleted.")
        return 2
    count, freed = sweep(debug, live, opts.dry_run)
    verb = "would free" if opts.dry_run else "freed"
    print(f"target-sweep: {len(live)} live units; {count} unused entries, {verb} {freed / (1 << 30):.1f} GB.")
    if opts.dry_run or opts.no_verify:
        return 0

    try:
        _, rebuilt = measure(tree, excludes)
    except MeasureFailed as failure:
        print(f"target-sweep: FAILED -- the verifying build failed after the sweep: {failure}.")
        return 1
    if rebuilt:
        print(f"target-sweep: FAILED -- {rebuilt} unit(s) rebuilt after the sweep, so it deleted "
              "something live. Report it with this tree's commands.")
        return 1
    print("target-sweep: verified -- every unit still fresh.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
