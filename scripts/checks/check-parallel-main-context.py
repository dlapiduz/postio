#!/usr/bin/env python3
"""Refuse a parallel test target that acquires the default GLib main context.

GTK may be initialized from one thread per process (#41), and libtest runs a
target's tests on a thread pool. Two checks already guard that:
`check-no-gtk-init-in-unit-tests.py` and `check-one-gtk-test-per-binary.py`.

**Acquiring the default main context is process-global in the same way, and
nothing checked for it.** `MainContext::default().iteration()`,
`.block_on()`, `.acquire()` — the context can be held by one thread at a
time, and the loser gets

    default main context already acquired by another thread

which panics, and in a `harness = false` target aborts the binary.

That is not hypothetical. #841 consolidated 197 test binaries into a few
suites, and `logic_suite` was assembled on the rule "does the file call
`adw::init`" — the obvious question, and the wrong one. `list_model` and
`drag_out` initialize nothing and both acquire the context. It passed
locally and aborted on a four-core runner.

# The rule

A test target using libtest's default harness must not acquire the default
main context **if it has more than one test**.

The size condition is the whole subtlety. One test cannot race itself:
`drag_out` is a single-case parallel binary that calls `block_on`, and that
is correct — refusing it would push it into a sequential suite, which is
where it segfaulted. What is unsafe is a second test running beside it.

A `harness = false` target is exempt: it controls its own scheduling, which
is why `gtk_suite` and `app_suite` exist.

# Exit status

0 clean, 1 a parallel target may race for the context, 2 the check could not
run.
"""

from __future__ import annotations

import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ACQUIRES = re.compile(r"MainContext::default\s*\(\s*\)")
TEST_FN = re.compile(r"^\s*#\[(?:tokio::)?test\]", re.M)
# `harness = false` inside a `[[test]]` block, with the path it names.
HARNESS_FALSE = re.compile(
    r"\[\[test\]\](?P<body>(?:[^\[]|\[(?!\[))*)", re.S
)


class CheckError(Exception):
    """The check could not be run, as opposed to: the check failed."""


def tracked(pattern: str) -> list[Path]:
    try:
        listed = subprocess.run(
            ["git", "ls-files", pattern],
            capture_output=True,
            text=True,
            check=True,
        )
    except FileNotFoundError as error:
        raise CheckError("git is not on PATH") from error
    except subprocess.CalledProcessError as error:
        raise CheckError(f"git ls-files failed: {error.stderr.strip()}") from error
    return [Path(line) for line in listed.stdout.splitlines() if line]


def sequential_roots() -> set[Path]:
    """Directories and files belonging to a `harness = false` target."""
    roots: set[Path] = set()
    for manifest in tracked("crates/*/Cargo.toml"):
        try:
            text = manifest.read_text(encoding="utf-8")
        except OSError:
            continue
        for block in HARNESS_FALSE.finditer(text):
            body = block.group("body")
            if not re.search(r"^\s*harness\s*=\s*false", body, re.M):
                continue
            named = re.search(r"^\s*path\s*=\s*\"([^\"]+)\"", body, re.M)
            if not named:
                continue
            path = manifest.parent / named.group(1)
            # `tests/x/main.rs` covers the whole directory; `tests/x.rs` itself.
            roots.add(path.parent if path.name == "main.rs" else path)
    return roots


def target_of(path: Path, sequential: set[Path]) -> tuple[Path, bool]:
    """``(the target this file belongs to, is it sequential)``."""
    for root in sequential:
        if root == path or root in path.parents:
            return root, True
    # A suite directory is one target; a bare `tests/x.rs` is its own.
    if path.parent.name != "tests":
        return path.parent, False
    return path, False


def main() -> int:
    try:
        sequential = sequential_roots()
        # Deduplicated: git pathspec `*` matches `/` as well, unlike a shell
        # glob, so the two patterns return overlapping lists and a file would
        # otherwise be counted -- and reported -- twice.
        sources = sorted(
            set(tracked("crates/*/tests/*.rs")) | set(tracked("crates/*/tests/*/*.rs"))
        )
    except CheckError as error:
        print(f"cannot run the check: {error}", file=sys.stderr)
        return 2

    tests_in: dict[Path, int] = defaultdict(int)
    acquires_in: dict[Path, list[Path]] = defaultdict(list)
    parallel: set[Path] = set()

    for path in sources:
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        target, is_sequential = target_of(path, sequential)
        if is_sequential:
            continue
        parallel.add(target)
        tests_in[target] += len(TEST_FN.findall(text))
        if ACQUIRES.search(text):
            acquires_in[target].append(path)

    problems: list[str] = []
    for target in sorted(parallel):
        if tests_in[target] > 1:
            for path in acquires_in[target]:
                problems.append(f"{path}: acquires the default main context, "
                                f"beside {tests_in[target] - 1} other test(s) "
                                "in the same parallel target")

    if not problems:
        print(f"parallel-main-context check passed ({len(parallel)} targets).")
        return 0

    print("parallel-main-context check FAILED\n", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        f"\n{len(problems)} occurrence(s).\n\n"
        "The default GLib main context can be held by one thread at a time,\n"
        "and libtest runs a target's tests on a thread pool. The loser gets\n"
        "`default main context already acquired by another thread`, which\n"
        "panics -- and takes the whole binary down in a harness = false\n"
        "target.\n\n"
        "It passes locally and fails on a runner with more cores, which is\n"
        "how #841 shipped it: `logic_suite` was assembled on 'does it call\n"
        "adw::init', and these files initialize nothing.\n\n"
        "Move the file into a `harness = false` suite (gtk_suite, app_suite),\n"
        "which runs its cases sequentially -- or, if it genuinely needs its\n"
        "own process, give it a test file of its own with one test in it.\n"
        "One test cannot race itself, which is why `drag_out` is allowed to\n"
        "stand alone.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
