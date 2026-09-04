#!/usr/bin/env python3
"""Self-test for scripts/restore-mtimes.py. See #1126.

The whole point of the script is a comparison cargo makes internally: is a
source file newer than the fingerprint recorded for the unit it belongs to.
Getting that comparison wrong in the "too old" direction is silent -- a real
change would be treated as if it never happened, and the only sign is a
build that is wrong and green. So this does not stop at checking the
script's own bookkeeping: the last case below builds a real crate with real
`cargo`, changes it, and asserts the second build actually recompiles.

Throwaway git repositories in a temp dir. The real repository is never
touched and nothing here reaches the network.

Usage: scripts/tests/test-restore-mtimes.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from datetime import datetime, timedelta, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "restore-mtimes.py"

GIT = ["git", "-c", "user.email=t@example.com", "-c", "user.name=Test"]

FAILURES: list[str] = []


def expect(case: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"  ok: {case}")
    else:
        FAILURES.append(f"{case}: {detail}")
        print(f"  FAILED: {case} — {detail}")


def write_and_commit(
    root: Path, relpath: str, content: str, message: str, *, date: str | None = None
) -> None:
    """`date`, when given, pins both author and committer date -- deterministic,
    and immune to two commits landing in the same wall-clock second, which
    git's default 1-second resolution makes a real risk for a fast test."""
    path = root / relpath
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")
    subprocess.run([*GIT, "add", relpath], cwd=root, check=True)
    env = None
    if date is not None:
        env = {**os.environ, "GIT_AUTHOR_DATE": date, "GIT_COMMITTER_DATE": date}
    subprocess.run([*GIT, "commit", "-qm", message], cwd=root, env=env, check=True)


def run_restore(root: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPT)],
        cwd=root,
        capture_output=True,
        text=True,
    )


def committer_date(root: Path, relpath: str) -> str:
    """Ground truth, independent of the script under test."""
    result = subprocess.run(
        [*GIT, "log", "-1", "--format=%cI", "--", relpath],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout.strip()


def as_epoch(iso: str) -> float:
    return datetime.fromisoformat(iso).astimezone(timezone.utc).timestamp()


def test_each_file_gets_its_own_commits_date() -> None:
    """A file untouched by the newest commit must not inherit that commit's
    date -- that is exactly the "every source file looks equally new" bug
    this script exists to undo, reintroduced one file at a time instead of
    by a fresh checkout."""
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        subprocess.run([*GIT, "init", "-q"], cwd=root, check=True)

        write_and_commit(
            root, "old.txt", "first\n", "write old.txt", date="2026-01-01T00:00:00+00:00"
        )
        write_and_commit(
            root, "new.txt", "second\n", "write new.txt", date="2026-06-15T00:00:00+00:00"
        )

        # Give every file a fresh, identical "checkout" mtime -- the exact
        # state a runner hands the script, and the state that would make
        # this test pass by accident if the script did nothing at all.
        now = os.stat(root / "old.txt").st_mtime
        for name in ("old.txt", "new.txt"):
            os.utime(root / name, (now, now))

        expected_old = as_epoch(committer_date(root, "old.txt"))
        expected_new = as_epoch(committer_date(root, "new.txt"))

        result = run_restore(root)
        expect(
            "the script exits cleanly",
            result.returncode == 0,
            f"exit {result.returncode}\n{result.stdout}{result.stderr}",
        )

        got_old = os.stat(root / "old.txt").st_mtime
        got_new = os.stat(root / "new.txt").st_mtime

        expect(
            "a file from the older commit keeps that commit's own date",
            abs(got_old - expected_old) < 1,
            f"expected ~{expected_old}, got {got_old}",
        )
        expect(
            "a file from the newer commit gets its own, later date",
            abs(got_new - expected_new) < 1,
            f"expected ~{expected_new}, got {got_new}",
        )
        expect(
            "the two files end up with different mtimes",
            got_old != got_new,
            "both files landed on the same mtime -- the per-file history was lost",
        )


def test_an_uncommitted_file_is_left_alone() -> None:
    """Nothing to restore it to yet; touching it backwards would make a
    file that was never built at all look older than it is."""
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        subprocess.run([*GIT, "init", "-q"], cwd=root, check=True)
        write_and_commit(root, "committed.txt", "one\n", "write committed.txt")

        staged = root / "staged.txt"
        staged.write_text("two\n", encoding="utf-8")
        subprocess.run([*GIT, "add", "staged.txt"], cwd=root, check=True)
        before = os.stat(staged).st_mtime

        result = run_restore(root)
        expect(
            "the script still exits cleanly with an uncommitted file present",
            result.returncode == 0,
            f"exit {result.returncode}\n{result.stdout}{result.stderr}",
        )
        after = os.stat(staged).st_mtime
        expect(
            "a file with no commit history keeps its own mtime",
            after == before,
            f"expected {before}, got {after}",
        )


def test_a_changed_file_always_rebuilds() -> None:
    """The case the doc string calls out by name: a real cargo build,
    restored mtimes, a real source change, and a second build that must
    actually recompile. Anything short of building for real can only prove
    this script's own bookkeeping is right, not that cargo agrees with it."""
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        subprocess.run([*GIT, "init", "-q"], cwd=root, check=True)

        # No pinned dates for these two: real time is always safely *before*
        # the build that follows, which is the property this fixture needs
        # from them. `v2`, below, needs the opposite -- safely *after* it --
        # and that one is pinned relative to the actual build time rather
        # than to the clock, so the assertion cannot go flaky depending on
        # how fast this machine happens to build a two-line crate.
        write_and_commit(
            root,
            "Cargo.toml",
            '[package]\nname = "restore-mtime-fixture"\nversion = "0.1.0"\nedition = "2021"\n\n'
            '[[bin]]\nname = "fixture"\npath = "src/main.rs"\n',
            "add the crate",
        )
        write_and_commit(root, "src/main.rs", 'fn main() { println!("v1"); }\n', "v1")

        target = root / "target"
        env = {**os.environ, "CARGO_TARGET_DIR": str(target)}
        first = subprocess.run(
            ["cargo", "build", "--quiet"], cwd=root, env=env, capture_output=True, text=True
        )
        binary = target / "debug" / "fixture"
        expect(
            "the fixture crate builds the first time",
            first.returncode == 0 and binary.exists(),
            f"exit {first.returncode}\n{first.stdout}{first.stderr}",
        )
        built_once = os.stat(binary).st_mtime

        run_restore(root)
        rebuilt_unchanged = subprocess.run(
            ["cargo", "build", "--quiet"], cwd=root, env=env, capture_output=True, text=True
        )
        expect(
            "restoring mtimes on its own does not force a needless rebuild",
            rebuilt_unchanged.returncode == 0
            and os.stat(binary).st_mtime == built_once,
            "the binary's mtime moved even though nothing changed",
        )

        # This is the commit standing in for "a PR landed after the cache was
        # built" -- what matters is that it is later than the artifact's own
        # build time, the same relationship a real new commit always has to
        # a cache seeded by an earlier run. A fixed calendar date could
        # easily be *before* `built_once` (this machine's real clock), which
        # would make the assertion below fail for a reason that has nothing
        # to do with the script.
        v2_date = (
            datetime.fromtimestamp(built_once, tz=timezone.utc) + timedelta(seconds=5)
        ).isoformat()
        write_and_commit(
            root, "src/main.rs", 'fn main() { println!("v2"); }\n', "v2", date=v2_date
        )
        run_restore(root)
        rebuilt_changed = subprocess.run(
            ["cargo", "build", "--quiet"], cwd=root, env=env, capture_output=True, text=True
        )
        expect(
            "a source change after restoring mtimes triggers a real rebuild",
            rebuilt_changed.returncode == 0
            and os.stat(binary).st_mtime != built_once,
            "the binary was not rebuilt after src/main.rs changed -- this is the silent failure mode",
        )


def main() -> int:
    print("restore-mtimes self-test")
    test_each_file_gets_its_own_commits_date()
    test_an_uncommitted_file_is_left_alone()
    test_a_changed_file_always_rebuilds()

    print()
    if FAILURES:
        print(f"{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("all cases behaved.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
