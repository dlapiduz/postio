#!/usr/bin/env python3
"""Self-test for scripts/checks/check-uncalled-pub-fn.py.

The check has to tell four things apart that all look alike to a grep: a
call, a doc comment naming the function, a test calling it, and the
definition itself. Get the last three wrong in either direction and the
check either cries wolf on every crate or reproduces #327 exactly — a
mechanism written, tested, documented and wired to nothing, with a green
suite the whole time.

So: throwaway git repositories in a temp dir, one crate each, and an
assertion for every way the rule can be met or broken. The real repository is
never touched and nothing here reaches the network.

Usage: scripts/tests/test-check-uncalled-pub-fn.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
CHECK = HERE / "checks" / "check-uncalled-pub-fn.py"

FAILURES: list[str] = []


def build_repo(
    root: Path,
    *,
    lib: str,
    caller: str = "",
    test: str = "",
    baseline: str = "",
    crate: str = "postio-thing",
    mock: str = "",
) -> None:
    """A git repository with one crate, and optionally a caller and a test.

    `caller` lands in a second crate's `src/lib.rs` — production code, the
    only thing that counts. `test` lands in the first crate's `tests/`, which
    must not count.

    Callers here are deliberately *not* `pub`, so the fixture's own scaffolding
    does not become the thing the check reports. That is the check working:
    a `pub fn run` calling the function under test would itself be a `pub fn`
    nothing calls.
    """
    source = root / "crates" / crate / "src"
    source.mkdir(parents=True)
    (source / "lib.rs").write_text(lib, encoding="utf-8")

    if mock:
        (source / "mock.rs").write_text(mock, encoding="utf-8")

    if caller:
        other = root / "crates" / "postio-caller" / "src"
        other.mkdir(parents=True)
        (other / "lib.rs").write_text(caller, encoding="utf-8")

    if test:
        suite = root / "crates" / crate / "tests"
        suite.mkdir(parents=True)
        (suite / "suite.rs").write_text(test, encoding="utf-8")

    if baseline:
        checks = root / "scripts" / "checks"
        checks.mkdir(parents=True)
        (checks / "uncalled-pub-fn-baseline.txt").write_text(baseline, encoding="utf-8")

    git = ["git", "-c", "user.email=t@example.com", "-c", "user.name=Test"]
    subprocess.run([*git, "init", "-q"], cwd=root, check=True)
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


def case(name: str, *, should_fail: bool, expect_text: str = "", **repo: str) -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        build_repo(root, **repo)
        result = run_check(root)

    want = 1 if should_fail else 0
    expect(
        name,
        result.returncode == want,
        f"expected exit {want}, got {result.returncode}\n{result.stdout}{result.stderr}",
    )
    if expect_text:
        output = result.stdout + result.stderr
        expect(
            f"{name} — says what and where",
            expect_text in output,
            f"{expect_text!r} not in:\n{output}",
        )


def main() -> int:
    print("check-uncalled-pub-fn:")

    # ── the bug this exists for ──────────────────────────────────────────
    #
    # #327 exactly: written, documented, tested, called by nothing.
    case(
        "a pub fn with only a test caller is flagged",
        lib="pub fn index_body(id: u32) -> u32 { id }\n",
        test="#[test]\nfn it_works() { postio_thing::index_body(1); }\n",
        should_fail=True,
        expect_text="index_body",
    )

    # ── the three things that must not count as calls ────────────────────
    case(
        "a doc comment naming it is not a caller",
        lib="pub fn collect_garbage() {}\n",
        caller="/// Leaks are prevented by [`collect_garbage`], the mechanism.\n"
        "/// See collect_garbage() for the ordering.\n"
        "fn unrelated() {}\n",
        should_fail=True,
        expect_text="collect_garbage",
    )
    case(
        "a line comment naming it is not a caller",
        lib="pub fn evict_to_fit() {}\n",
        caller="fn unrelated() {\n    // evict_to_fit() would go here\n}\n",
        should_fail=True,
        expect_text="evict_to_fit",
    )
    case(
        "a string naming it is not a caller",
        lib="pub fn purge_temporary() {}\n",
        caller='fn unrelated() -> &\'static str { "purge_temporary" }\n',
        should_fail=True,
        expect_text="purge_temporary",
    )
    case(
        "a #[cfg(test)] module in the same file is not a caller",
        lib="pub fn sweep() {}\n\n"
        "#[cfg(test)]\nmod tests {\n"
        "    use super::*;\n"
        "    #[test]\n    fn t() { sweep(); }\n}\n",
        should_fail=True,
        expect_text="sweep",
    )

    # ── a char literal is not a string, and reading it as one loses the
    #    rest of the file ────────────────────────────────────────────────
    #
    # `'"'` holds one double quote. Scanned as the start of a string literal
    # it inverts the quote parity of everything after it, so real code is
    # blanked as "string" and string contents are read as code. The caller
    # below then disappears and a wired-up function is reported as dead --
    # silently, in whichever file happened to contain the char literal.
    case(
        "a char literal holding a quote does not swallow the caller after it",
        lib="pub fn index_body() {}\n",
        caller="fn escape(c: char) -> &'static str {\n"
        "    match c {\n"
        "        '\"' => \"&quot;\",\n"
        "        _ => \"\",\n"
        "    }\n"
        "}\n"
        "fn run() { postio_thing::index_body(); }\n",
        should_fail=False,
    )
    case(
        "a lifetime is still not a char literal",
        lib="pub fn index_body() {}\n",
        caller="fn borrow<'a>(text: &'a str) -> &'a str { text }\n"
        "fn run() { postio_thing::index_body(); }\n",
        should_fail=False,
    )

    # ── and the thing that does ──────────────────────────────────────────
    case(
        "one production caller is enough",
        lib="pub fn index_body() {}\n",
        caller="fn run() { postio_thing::index_body(); }\n",
        should_fail=False,
    )
    case(
        "a method reached through a trait object counts, because the call names it",
        lib="pub trait Backend { fn list_mailboxes(&self); }\n"
        "pub struct Imap;\n"
        "impl Imap { pub fn list_mailboxes(&self) {} }\n",
        caller="fn run(backend: &dyn postio_thing::Backend) { backend.list_mailboxes(); }\n",
        should_fail=False,
    )
    case(
        "a definition does not count as its own caller",
        lib="pub fn alone() {}\n",
        should_fail=True,
        expect_text="alone",
    )

    # ── what is deliberately not scanned ─────────────────────────────────
    case(
        "a frontend crate's test accessors are not scanned",
        lib="pub fn banner_visible() -> bool { true }\n",
        crate="postio-gtk",
        should_fail=False,
    )
    case(
        "a #[doc(hidden)] item is not scanned",
        lib="#[doc(hidden)]\npub fn test_open_menu() {}\n",
        should_fail=False,
    )
    case(
        "a test-support module is not scanned",
        lib="pub mod mock;\n",
        mock="pub fn fail_all() {}\npub fn change_uid_validity() {}\n",
        should_fail=False,
    )
    case(
        "the same functions outside a test-support module are scanned",
        lib="pub fn fail_all() {}\npub fn change_uid_validity() {}\n",
        should_fail=True,
        expect_text="change_uid_validity",
    )
    case(
        "a #[cfg(test)] definition is not scanned",
        lib="#[cfg(test)]\nmod helpers {\n    pub fn only_for_tests() {}\n}\n",
        should_fail=False,
    )

    # ── #882: a feature-gated test helper is exempt without #[doc(hidden)] ─
    case(
        "a #[cfg(feature = \"test-support\")] item with only a test caller is exempt",
        lib='#[cfg(feature = "test-support")]\npub fn stopping_after() {}\n',
        test="#[test]\nfn it_works() { postio_thing::stopping_after(); }\n",
        should_fail=False,
    )
    case(
        "the same function without the feature gate is still scanned",
        lib="pub fn stopping_after() {}\n",
        test="#[test]\nfn it_works() { postio_thing::stopping_after(); }\n",
        should_fail=True,
        expect_text="stopping_after",
    )
    case(
        "any feature name containing test is exempt, not only test-support",
        lib='#[cfg(feature = "testing")]\npub fn only_in_debug_builds() {}\n',
        test="#[test]\nfn it_works() { postio_thing::only_in_debug_builds(); }\n",
        should_fail=False,
    )
    case(
        "a feature gate that does not name test is not exempted by it",
        lib='#[cfg(feature = "gtk")]\npub fn banner_widget() {}\n',
        test="#[test]\nfn it_works() { postio_thing::banner_widget(); }\n",
        should_fail=True,
        expect_text="banner_widget",
    )
    case(
        # The real shape #882 was found in: the exempting attribute sits
        # right beside a doc comment that quotes it as prose, on the
        # function it actually belongs to. Blanking strings is what stops a
        # comment from creating a *caller* (the case above); this is the
        # companion property that a real attribute is still read correctly
        # with prose right next to it.
        "a doc comment quoting the attribute does not stop the real one working",
        lib="/// Behind the `test-support` feature. `#[cfg(feature = \"test-support\")]`\n"
        "/// is what marks it as test scaffolding.\n"
        '#[cfg(feature = "test-support")]\n'
        "pub fn stopping_after() {}\n",
        test="#[test]\nfn it_works() { postio_thing::stopping_after(); }\n",
        should_fail=False,
    )

    # ── the baseline, in both directions ─────────────────────────────────
    case(
        "a baselined name is accepted",
        lib="pub fn known_debt() {}\n",
        baseline="# debt\nknown_debt  # crates/postio-thing/src/lib.rs:1\n",
        should_fail=False,
    )
    case(
        "a baselined name that has gained a caller fails, so the list shrinks",
        lib="pub fn was_debt() {}\n",
        caller="fn run() { postio_thing::was_debt(); }\n",
        baseline="was_debt\n",
        should_fail=True,
        expect_text="now called",
    )
    case(
        "a baselined name that no longer exists fails",
        lib="pub fn something_else() {}\n",
        caller="fn run() { postio_thing::something_else(); }\n",
        baseline="deleted_long_ago\n",
        should_fail=True,
        expect_text="no longer defined",
    )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("\nall cases behaved.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
