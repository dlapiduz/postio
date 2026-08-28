#!/usr/bin/env python3
"""Self-test for scripts/checks/check-no-silent-tracking.py.

A guard that has never been seen to fail is not a guard — and this one guards
something that currently does not exist, so on the real tree it passes whether
it works or not. That is precisely the shape of check that rots silently.

So: throwaway git repositories in a temp dir, each with one crate in it, and
an assertion for every way the rule can be met or broken. The real repository
is never touched and nothing here reaches the network.

Usage: scripts/tests/test-check-no-silent-tracking.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-no-silent-tracking.py"

FAILURES: list[str] = []


def build_repo(root: Path, source: str, *, track: bool = True) -> None:
    """A git repository with one crate whose `lib.rs` is `source`."""
    crate = root / "crates" / "postio-thing" / "src"
    crate.mkdir(parents=True, exist_ok=True)
    (crate / "lib.rs").write_text(source, encoding="utf-8")

    git = ["git", "-c", "user.email=t@example.com", "-c", "user.name=Test"]
    subprocess.run([*git, "init", "-q"], cwd=root, check=True)
    if track:
        subprocess.run([*git, "add", "."], cwd=root, check=True)
        subprocess.run([*git, "commit", "-qm", "fixture"], cwd=root, check=True)


def build_swift_repo(root: Path, source: str, *, at: str) -> None:
    """A git repository with one Swift file at `macos/Sources/<at>`.

    Nested by default in the cases below, because `git ls-files` treats `**`
    differently from a shell and a pattern that only matched a top-level file
    would leave every real source file — all of which are nested — unscanned.
    """
    swift = root / "macos" / "Sources" / at
    swift.parent.mkdir(parents=True, exist_ok=True)
    swift.write_text(source, encoding="utf-8")
    # A crate too, so the repository looks like the real one and the Rust
    # scan is exercised alongside rather than replaced.
    crate = root / "crates" / "postio-thing" / "src"
    crate.mkdir(parents=True, exist_ok=True)
    (crate / "lib.rs").write_text("pub fn ordinary() {}\n", encoding="utf-8")

    git = ["git", "-c", "user.email=t@example.com", "-c", "user.name=Test"]
    subprocess.run([*git, "init", "-q"], cwd=root, check=True)
    subprocess.run([*git, "add", "."], cwd=root, check=True)
    subprocess.run([*git, "commit", "-qm", "fixture"], cwd=root, check=True)


def swift_case(name: str, source: str, *, should_fail: bool, at: str, expect_text: str = "") -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        build_swift_repo(root, source, at=at)
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


def case(name: str, source: str, *, should_fail: bool, expect_text: str = "", track: bool = True) -> None:
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
    print("check-no-silent-tracking self-test")

    case(
        "an ordinary crate passes",
        "pub fn read_a_message() {}\n",
        should_fail=False,
    )

    case(
        "a read receipt header fails",
        'const HEADER: &str = "Disposition-Notification-To";\n',
        should_fail=True,
        expect_text="read receipt (MDN)",
    )

    case(
        "the legacy spelling fails too",
        'const HEADER: &str = "Return-Receipt-To";\n',
        should_fail=True,
        expect_text="Return-Receipt-To",
    )

    case(
        "One-Click unsubscribe fails",
        'fn post() { let h = "List-Unsubscribe-Post"; }\n',
        should_fail=True,
        expect_text="One-Click",
    )

    case(
        "launching the system browser fails without recorded consent",
        'Command::new("xdg-open").arg(url).spawn()\n',
        should_fail=True,
        expect_text="OAuth consent",
    )

    case(
        "a browser launch with recorded consent passes",
        "// POSTIO-CONSENT: only from the account's own \"Sign in\" action,\n"
        "// one attempt per click, never on render.\n"
        'Command::new("xdg-open").arg(url).spawn()\n',
        should_fail=False,
    )

    case(
        "the GTK browser launcher fails without recorded consent",
        "let launcher = gtk::UriLauncher::new(&uri);\n",
        should_fail=True,
        expect_text="UriLauncher",
    )

    case(
        "the GTK file launcher fails without recorded consent",
        "let launcher = gtk::FileLauncher::new(Some(&file));\n",
        should_fail=True,
        expect_text="FileLauncher",
    )

    case(
        "a GTK launcher with recorded consent passes",
        "// POSTIO-CONSENT: launched only from a deliberate click on a link\n"
        "// in the message being read, never on render.\n"
        "gtk::UriLauncher::new(&uri).launch(parent, cancellable, done);\n",
        should_fail=False,
    )

    # Header names are case-insensitive and so is a patch author.
    case(
        "a lower-case spelling does not slip past",
        'let header = "disposition-notification-to";\n',
        should_fail=True,
        expect_text="read receipt (MDN)",
    )

    case(
        "it names the file and the line",
        '// a comment\nconst H: &str = "Disposition-Notification-To";\n',
        should_fail=True,
        expect_text="src/lib.rs:2",
    )

    # The whole point: the mechanism is allowed once somebody has written down
    # how the user asks for it.
    case(
        "a recorded consent path passes",
        "// POSTIO-CONSENT: only from the reader's `Send receipt` button,\n"
        "// per message, never from a setting and never on render.\n"
        'const HEADER: &str = "Disposition-Notification-To";\n',
        should_fail=False,
    )

    # ---------------------------------------------------------------- Swift --
    #
    # The macOS frontend is Swift, and a guard that only reads Rust would leave
    # half the application outside it -- the half whose whole privacy claim is
    # that its web view has no network. ADR 0019 Q6, #625.

    swift_case(
        "an ordinary Swift file passes",
        "import SwiftUI\n\nstruct Shell: View { var body: some View { Text(\"hi\") } }\n",
        should_fail=False,
        at="PostioUI/Shell.swift",
    )

    swift_case(
        "a URLSession in the Swift half is refused",
        "import Foundation\n\nfunc fetch() { _ = URLSession.shared }\n",
        should_fail=True,
        at="PostioUI/Reader/Fetcher.swift",
        expect_text="Fetcher.swift",
    )

    swift_case(
        "...and is allowed once somebody has argued for it",
        "import Foundation\n\n"
        "// POSTIO-CONSENT: only from the account's own \"Sign in\" action, and\n"
        "// only to the loopback redirect the OAuth flow is listening on.\n"
        "func fetch() { _ = URLSession.shared }\n",
        should_fail=False,
        at="PostioUI/Reader/Fetcher.swift",
    )

    swift_case(
        "opening the system browser from Swift is refused",
        "import AppKit\n\nfunc open(_ url: URL) { NSWorkspace.shared.open(url) }\n",
        should_fail=True,
        at="PostioUI/Reader/Links.swift",
        expect_text="Links.swift",
    )

    swift_case(
        "a nested Swift file is scanned, not just a top-level one",
        "import Foundation\n\nfunc fetch() { _ = URLSession.shared }\n",
        should_fail=True,
        at="PostioUI/Reader/Inline/Deep.swift",
        expect_text="Deep.swift",
    )

    # `git ls-files` is the input, so the check is about the repository rather
    # than about whatever happens to be in somebody's working tree.
    case(
        "an untracked experiment is not the repository's problem",
        'const HEADER: &str = "Disposition-Notification-To";\n',
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
