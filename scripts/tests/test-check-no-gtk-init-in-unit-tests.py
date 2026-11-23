#!/usr/bin/env python3
"""Self-test for scripts/checks/check-no-gtk-init-in-unit-tests.py.

The check exists because a toast's unit test in the GTK crate called
`adw::init()` and aborted CI with SIGABRT. Once that test moved to
`tests/`, the tree became clean — and a guard that passes on a clean tree
passes whether it works or not.

So the failure modes are exercised here instead: throwaway git repositories
in a temp dir, one crate each, and an assertion for every way the rule can be
met or broken. The brace scanner gets particular attention, because the
difference between "inside the test module" and "after it" is the whole rule.

The real repository is never touched and nothing here reaches the network.

Usage: scripts/tests/test-check-no-gtk-init-in-unit-tests.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-no-gtk-init-in-unit-tests.py"

FAILURES: list[str] = []


def build_repo(root: Path, source: str, *, track: bool = True) -> None:
    """A git repository with one crate whose `lib.rs` is `source`."""
    crate = root / "crates" / "postio-thing" / "src"
    crate.mkdir(parents=True, exist_ok=True)
    (crate / "lib.rs").write_text(source, encoding="utf-8")

    tests = root / "crates" / "postio-thing" / "tests"
    tests.mkdir(parents=True, exist_ok=True)
    (tests / "gtk_thing.rs").write_text(
        "#[test]\nfn shows() {\n    if adw::init().is_err() { return; }\n}\n",
        encoding="utf-8",
    )

    git = ["git", "-c", "user.email=t@example.com", "-c", "user.name=Test"]
    subprocess.run([*git, "init", "-q"], cwd=root, check=True)
    if track:
        subprocess.run([*git, "add", "."], cwd=root, check=True)
        subprocess.run([*git, "commit", "-qm", "fixture"], cwd=root, check=True)


def run_check(root: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(CHECK)],
        cwd=root,
        capture_output=True,
        text=True,
    )


def expect(case: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"  ok: {case}")
    else:
        FAILURES.append(f"{case}: {detail}")
        print(f"  FAILED: {case} — {detail}")


def case(
    name: str,
    source: str,
    *,
    should_fail: bool,
    expect_text: str = "",
    track: bool = True,
) -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        build_repo(root, source, track=track)
        result = run_check(root)

    if should_fail:
        expect(
            name,
            result.returncode == 1,
            f"expected exit 1, got {result.returncode}\n{result.stdout}{result.stderr}",
        )
        if expect_text:
            expect(
                f"{name} — says what and where",
                expect_text in result.stderr,
                f"{expect_text!r} not in:\n{result.stderr}",
            )
    else:
        expect(
            name,
            result.returncode == 0,
            f"expected exit 0, got {result.returncode}\n{result.stdout}{result.stderr}",
        )


def main() -> int:
    print("check-no-gtk-init-in-unit-tests self-test")

    case(
        "an ordinary crate passes",
        "pub fn build_a_row() {}\n",
        should_fail=False,
    )

    case(
        "production code may initialize GTK",
        "pub fn run() {\n    if adw::init().is_err() {\n        return;\n    }\n}\n",
        should_fail=False,
    )

    case(
        "a unit test that initializes GTK fails",
        "pub fn toast() {}\n"
        "\n"
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    #[test]\n"
        "    fn shows_a_toast() {\n"
        "        if adw::init().is_err() {\n"
        "            return;\n"
        "        }\n"
        "    }\n"
        "}\n",
        should_fail=True,
        expect_text="adw::init",
    )

    case(
        "it names the file and the line",
        "#[cfg(test)]\nmod tests {\n    fn go() {\n        adw::init().unwrap();\n    }\n}\n",
        should_fail=True,
        expect_text="src/lib.rs:4",
    )

    case(
        "a bare #[test] fn is a test region too",
        "#[test]\nfn direct() {\n    gtk::init().unwrap();\n}\n",
        should_fail=True,
        expect_text="gtk::init",
    )

    case(
        "code after the test module is not the test module",
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    #[test]\n"
        "    fn nested() {\n"
        "        if true {\n"
        "            let _ = 1;\n"
        "        }\n"
        "    }\n"
        "}\n"
        "\n"
        "pub fn run() {\n"
        "    adw::init().unwrap();\n"
        "}\n",
        should_fail=False,
    )

    case(
        "a brace in a string literal does not end the module early",
        "#[cfg(test)]\n"
        "mod tests {\n"
        '    const CSS: &str = "window { color: red; }";\n'
        "    #[test]\n"
        "    fn go() {\n"
        "        adw::init().unwrap();\n"
        "    }\n"
        "}\n",
        should_fail=True,
        expect_text="adw::init",
    )

    case(
        "a brace in a char literal does not end the module early",
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    const CLOSE: char = '}';\n"
        "    #[test]\n"
        "    fn go() {\n"
        "        adw::init().unwrap();\n"
        "    }\n"
        "}\n",
        should_fail=True,
        expect_text="adw::init",
    )

    case(
        "a comment naming the call is not a call",
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    // Deliberately does not call adw::init() -- see tests/.\n"
        '    /* nor does it call gtk::init() */\n'
        "    #[test]\n"
        "    fn pure() {\n"
        "        assert!(true);\n"
        "    }\n"
        "}\n",
        should_fail=False,
    )

    case(
        "a recorded exception passes",
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    // POSTIO-GTK-INIT: deliberately the only one in this crate.\n"
        "    #[test]\n"
        "    fn go() {\n"
        "        adw::init().unwrap();\n"
        "    }\n"
        "}\n",
        should_fail=False,
    )

    # The marker clears a file for one init: when it cleared the file outright, a second GTK test
    # could be added and panicked with "two different threads".
    case(
        "a marked file may keep one init, not two",
        "#[cfg(test)]\n"
        "mod tests {\n"
        "    // POSTIO-GTK-INIT: deliberately the only one in this crate.\n"
        "    #[test]\n"
        "    fn go() {\n"
        "        adw::init().unwrap();\n"
        "    }\n"
        "    #[test]\n"
        "    fn also_go() {\n"
        "        adw::init().unwrap();\n"
        "    }\n"
        "}\n",
        should_fail=True,
        expect_text="second",
    )

    case(
        "an untracked experiment is not the repository's problem",
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn go() { adw::init().unwrap(); }\n}\n",
        should_fail=False,
        track=False,
    )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) misbehaved:", file=sys.stderr)
        for failure in FAILURES:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print("\nall cases behaved.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
