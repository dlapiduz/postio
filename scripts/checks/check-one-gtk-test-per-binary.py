#!/usr/bin/env python3
"""Refuse a second display-needing `#[test]` in one integration test file.

`check-no-gtk-init-in-unit-tests.py` keeps GTK out of `src` unit tests, and
its rationale ends "cargo gives every *integration* test its own process, so
the process-wide init is safe there." That is half true, and the missing half
is this check: cargo gives every integration test *file* its own process, but
libtest still runs the `#[test]`s **inside** that file on a thread pool. Two
of them initializing GTK is the same two-threads race, just one level down.

It does not fail honestly. `crates/postio-gtk/tests/gtk_composer_autosave.rs`
held two of them and reported `ok` for both on every run since it was
written, because the loser takes the `adw::init().is_err()` branch every
GTK test has:

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` ...)");
        return;
    }

which is there for a headless CI box and cannot tell that case apart from
"another thread in this process got GTK first". So the test returns before
asserting anything and libtest calls that a pass. Worse, *which* of the two
evaporates depends on thread scheduling: three consecutive runs of that file
took 1.88s, 1.89s and 0.42s, the fast one being the run where the debounce
test — the one with actual timing to prove — was the one that vanished. Pin
libtest to one thread and it stops being silent and starts aborting:
"Attempted to initialize GTK from two different threads." #355.

# The rule

A file under ``crates/*/tests`` may contain **at most one** `#[test]` whose
body initializes GTK. Tests that need no display are unaffected, which is
why `gtk_shell.rs` legitimately keeps two: one builds a window, the other
parses the stylesheet as text.

The fix for a violation is not to delete a test. It is to move the cases into
the custom harness at ``crates/postio-gtk/tests/gtk_suite/``, which exists for
exactly this (#329): `harness = false`, one `adw::init`, every case a plain
`pub fn` run in sequence on the main thread. A case there is a `pub fn`, not a
`#[test]`, so this check sees nothing to complain about and both cases
actually run.

# Exit status

0 clean, 1 a file holds two display-needing tests, 2 the check could not run.
"""

from __future__ import annotations

import importlib.util
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent

# The sibling owns the delicate part -- blanking comments and literals before
# anything counts a brace, and finding where a `#[test]` item ends. Importing
# it keeps one implementation of "how to read Rust safely enough for a
# check", rather than a second copy that drifts. Its filename is not an
# identifier, so it is loaded by path.
_SIBLING = HERE / "check-no-gtk-init-in-unit-tests.py"


def _load_sibling():
    spec = importlib.util.spec_from_file_location("_gtk_init_check", _SIBLING)
    if spec is None or spec.loader is None:
        raise CheckError(f"cannot load {_SIBLING}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class CheckError(Exception):
    """The check could not be run, as opposed to: the check failed."""


FN_NAME = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")


def tracked_tests() -> list[Path]:
    """Every Rust file under a crate's ``tests``, as git sees it."""
    try:
        listed = subprocess.run(
            ["git", "ls-files", "crates/*/tests/*.rs", "crates/*/tests/**/*.rs"],
            capture_output=True,
            text=True,
            check=True,
        )
    except FileNotFoundError as error:
        raise CheckError("git is not on PATH") from error
    except subprocess.CalledProcessError as error:
        raise CheckError(f"git ls-files failed: {error.stderr.strip()}") from error
    return [Path(line) for line in listed.stdout.splitlines() if line]


def display_tests(source: str, sibling) -> list[str]:
    """Names of the `#[test]` functions in `source` that initialize GTK."""
    clean = sibling.blank_noise(source)
    found: list[str] = []
    for start, stop in sibling.test_regions(clean):
        region = clean[start:stop]
        if not sibling.INIT.search(region):
            continue
        # `test_regions` also spans `#[cfg(test)] mod` items; an integration
        # test has none, but name the function when there is one so the
        # report says which tests collided.
        name = FN_NAME.search(region)
        found.append(name.group(1) if name else "<unnamed>")
    return found


def main() -> int:
    try:
        sibling = _load_sibling()
        files = tracked_tests()
    except CheckError as error:
        print(f"one-gtk-test-per-binary check could not run: {error}", file=sys.stderr)
        return 2

    violations: list[tuple[Path, list[str]]] = []
    for path in files:
        try:
            source = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        names = display_tests(source, sibling)
        if len(names) > 1:
            violations.append((path, names))

    if violations:
        print(
            "These integration test files hold more than one `#[test]` that "
            "initializes GTK.\nlibtest runs them on a thread pool, GTK "
            "tolerates one thread, and the loser\nsilently returns through "
            "its `no display` guard and is reported as passing:\n",
            file=sys.stderr,
        )
        for path, names in violations:
            print(f"  {path}", file=sys.stderr)
            for name in names:
                print(f"      {name}", file=sys.stderr)
        print(
            "\nMove the cases into crates/postio-gtk/tests/gtk_suite/ — a custom\n"
            "harness that runs each as a plain `pub fn`, in sequence, on the one\n"
            "thread GTK allows. See that directory's main.rs, and #355.",
            file=sys.stderr,
        )
        return 1

    print(f"one-gtk-test-per-binary check passed ({len(files)} files).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
