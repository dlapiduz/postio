#!/usr/bin/env python3
"""Self-test for scripts/checks/check-one-gtk-test-per-binary.py.

The check exists because three integration test files each held two
display-needing `#[test]`s and had been running one of them, at random, since
they were written — reporting `ok` for both (#355). Once those moved into the
`gtk_suite` harness the tree became clean, and a guard that passes on a clean
tree passes whether it works or not.

So the failure modes are exercised here instead: throwaway git repositories in
a temp dir, one crate each, and a case for every way the rule can be met or
broken. Two in particular are worth their own case, because getting either
wrong makes the check useless rather than merely noisy:

  * a file with two tests where only ONE needs a display is legitimate and
    must pass — `gtk_shell.rs` is exactly that, one test building a window and
    one parsing the stylesheet as text;
  * an `adw::init()` named only in a comment or a string must not count, which
    is the whole reason the check borrows the sibling's Rust-blanking pass
    rather than grepping.

The real repository is never touched and nothing here reaches the network.

Usage: scripts/tests/test-check-one-gtk-test-per-binary.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-one-gtk-test-per-binary.py"
SIBLING = HERE / "checks" / "check-no-gtk-init-in-unit-tests.py"

FAILURES: list[str] = []

GUARD = (
    "    if adw::init().is_err() || gdk::Display::default().is_none() {\n"
    '        eprintln!("skipping: no display");\n'
    "        return;\n"
    "    }\n"
)


def build_repo(root: Path, test_source: str) -> None:
    """A git repository with one crate holding `test_source` under tests/."""
    tests = root / "crates" / "postio-thing" / "tests"
    tests.mkdir(parents=True, exist_ok=True)
    (tests / "gtk_thing.rs").write_text(test_source, encoding="utf-8")

    # The check loads its sibling by path, so both have to be reachable from
    # the copy under test. Ship them into the throwaway repo together.
    checks = root / "scripts" / "checks"
    checks.mkdir(parents=True, exist_ok=True)
    (checks / CHECK.name).write_text(CHECK.read_text(encoding="utf-8"), encoding="utf-8")
    (checks / SIBLING.name).write_text(SIBLING.read_text(encoding="utf-8"), encoding="utf-8")

    git = ["git", "-c", "user.email=t@example.com", "-c", "user.name=Test"]
    subprocess.run([*git, "init", "-q"], cwd=root, check=True)
    subprocess.run([*git, "add", "-A"], cwd=root, check=True)


def run_check(root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "scripts/checks/" + CHECK.name],
        cwd=root,
        capture_output=True,
        text=True,
        timeout=60,
    )


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def check_source(name: str, source: str, *, expect_fail: bool, detail: str) -> None:
    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw)
        build_repo(root, source)
        result = run_check(root)
        failed = result.returncode == 1
        case(
            name,
            failed == expect_fail,
            f"{detail} (exit {result.returncode}; {result.stdout.strip()}"
            f"{result.stderr.strip()})",
        )


def main() -> int:
    # ── the bug this check exists for ────────────────────────────────────
    check_source(
        "two display-needing tests in one file is refused",
        f"#[test]\nfn first() {{\n{GUARD}}}\n\n#[test]\nfn second() {{\n{GUARD}}}\n",
        expect_fail=True,
        detail="the check let through exactly the shape of #355",
    )

    # ── and the shapes that must keep working ────────────────────────────
    check_source(
        "one display-needing test is fine",
        f"#[test]\nfn only() {{\n{GUARD}}}\n",
        expect_fail=False,
        detail="a single GTK test is the normal, correct shape",
    )
    check_source(
        "two tests where only one needs a display is fine",
        f"#[test]\nfn draws() {{\n{GUARD}}}\n\n"
        '#[test]\nfn parses_css() {\n    assert!("a { }".contains(\'{\'));\n}\n',
        expect_fail=False,
        detail="gtk_shell.rs is legitimately this shape and must not be flagged",
    )
    check_source(
        "no tests at all is fine",
        "pub fn helper() {}\n",
        expect_fail=False,
        detail="a harness case file is a plain pub fn and has nothing to flag",
    )

    # ── the reason it blanks Rust rather than grepping ───────────────────
    check_source(
        "adw::init named in a comment does not count",
        f"#[test]\nfn draws() {{\n{GUARD}}}\n\n"
        "#[test]\nfn documented() {\n    // calls adw::init() one day\n}\n",
        expect_fail=False,
        detail="a comment was counted as a second GTK test",
    )
    check_source(
        "adw::init inside a string does not count",
        f"#[test]\nfn draws() {{\n{GUARD}}}\n\n"
        '#[test]\nfn mentions() {\n    let _ = "adw::init()";\n}\n',
        expect_fail=False,
        detail="a string literal was counted as a second GTK test",
    )

    # ── the custom harness is the sanctioned way to hold several ─────────
    check_source(
        "several harness cases in one file are fine",
        f"pub fn first() {{\n{GUARD}}}\n\npub fn second() {{\n{GUARD}}}\n",
        expect_fail=False,
        detail="gtk_suite cases are pub fn, run in sequence, and are the fix",
    )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("one-gtk-test-per-binary self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
