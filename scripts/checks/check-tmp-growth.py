#!/usr/bin/env python3
"""Notice `target/tmp` growing, since nothing else does (#605).

`.cargo/config.toml` points `TMPDIR` at `target/tmp` (relative to the
workspace) specifically so a test's temp directories land on disk rather
than the host's tmpfs (`scripts/rustc-wrapper.sh` does the matching thing
for sccache's own daemon). That fixed a memory problem, not a leak: dozens
of test files create a `tempfile::tempdir()` and immediately call
`.keep()` on it, or build a state directory under `std::env::temp_dir()`
by hand, and nothing ever removes either. One afternoon of
`cargo test -p postio-app` left 400 behind before this was noticed.

Fixing every call site is real work -- some `.keep()` calls are load-bearing
(the path outlives the test body, or a later assertion reads it back), so it
wants reading rather than a blanket search-and-replace. This is the fallback
named in #605 for while that work is unstarted or partial: a nudge, not a
gate. `target/tmp` growing is not a defect in the tree the way a missing
license or a floating toolchain is -- it is one worktree's own test runs
piling up -- so this never fails the build. It says so, loud enough to
notice, and names the one-line fix (`rm -rf target/tmp`; cargo recreates it
on demand).

# Exit status

Always 0. This is a diagnostic, not an invariant -- see above.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

# Past this, it is worth a person's attention: a handful of leaked temp dirs
# from one test run is normal wear, not a leak worth stopping to look at.
WARN_BYTES = 200 * 1024 * 1024  # 200 MiB
WARN_ENTRIES = 50


def repository_root() -> Path:
    """The checkout this script lives in."""
    try:
        top = subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True,
            text=True,
            check=True,
            cwd=Path(__file__).resolve().parent,
        ).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError) as error:
        print(f"cannot find the repository root: {error}", file=sys.stderr)
        raise SystemExit(2) from error
    return Path(top)


def human(size: int) -> str:
    """`size` in bytes, as the largest unit that keeps at least one digit."""
    value = float(size)
    for unit in ("B", "KiB", "MiB", "GiB"):
        if value < 1024 or unit == "GiB":
            return f"{value:.1f} {unit}"
        value /= 1024
    return f"{value:.1f} GiB"  # unreachable, satisfies type checkers


def measure(tmp: Path) -> tuple[int, int]:
    """Top-level entries under `tmp`, and their total size in bytes.

    Top-level count rather than every file: each leaked run is one
    directory (a blob store, a state dir), and counting every file inside
    each would report the same leak as a much bigger number for no reason.
    Size still walks everything, since that is the resource actually at
    stake.
    """
    entries = 0
    total = 0
    for child in tmp.iterdir():
        entries += 1
        if child.is_dir() and not child.is_symlink():
            for path in child.rglob("*"):
                if path.is_file() and not path.is_symlink():
                    total += path.stat().st_size
        elif child.is_file():
            total += child.stat().st_size
    return entries, total


def main() -> int:
    root = repository_root()
    tmp = root / "target" / "tmp"

    if not tmp.is_dir():
        print("temp-directory hygiene check passed (target/tmp does not exist yet).")
        return 0

    entries, total = measure(tmp)

    if entries > WARN_ENTRIES or total > WARN_BYTES:
        print(
            f"note: target/tmp holds {entries} entr{'y' if entries == 1 else 'ies'} "
            f"({human(total)}) that test runs left behind and nothing cleaned up "
            f"(#605). Not a failure -- safe to clear:\n"
            f"    rm -rf {tmp}\n"
            f"  cargo recreates it on demand.",
            file=sys.stderr,
        )
        return 0

    print(f"temp-directory hygiene check passed (target/tmp: {entries} entries, {human(total)}).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
