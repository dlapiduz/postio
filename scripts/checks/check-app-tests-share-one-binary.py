#!/usr/bin/env python3
"""Refuse a new `crates/postio-app/tests/*.rs` that is not `e2e*`.

Every file directly under a crate's ``tests/`` is its own `[[test]]` target,
and every target in `postio-app` links the whole application — GTK, WebKit,
SQLite and all. Measured when this check was written, each of those binaries
was over 200 MB and `app_suite` took about eleven minutes to link.

That cost is not the reason this matters. `CLAUDE.md` prices the wiring tier
and then, correctly, tells sessions to iterate at the cheapest layer that can
fail — which routes everyone away from exactly the tests that catch this
project's characteristic bug: layers that each pass and are not joined up
(`postio-bl2`, eight instances, four of them shipped). The tests that can see
that bug are the expensive ones, so they are the ones nobody runs. Folding
them into one already-built binary is what makes the guidance to avoid them
stop being right.

`crates/postio-app/tests/app_suite/` is that binary: `harness = false`, one
`adw::init`, every case a plain `pub fn` run in sequence. #973 moved seven
files into it; this check is what stops the eighth appearing.

# The rule

A file directly under ``crates/postio-app/tests`` must be named `e2e*` or be
the `app_suite` directory. Nothing else.

`e2e*` is one documented exception, and it is not a style preference: the
headless runner's watchdog finds those binaries **by name**
(`scripts/headless-runner.sh`, #272) and runs them in isolation. A case that
genuinely needs that, or a private display (#45/#114), or a wall-clock budget
a shared process would disturb (#841), has a reason to stay out — and should
say so in its own doc comment rather than leaving the next person to work it
out, which is the gap that produced #973.

`ALLOWED_FILES` holds the rest, each with the reason it keeps a process --
and each says the same thing in its own doc comment, which is the half a
future reader actually reaches.

Otherwise: move it to ``crates/postio-app/tests/app_suite/<name>.rs``, turn
each `#[test] fn` into a `pub fn`, and add it to `main.rs`'s `mod` list and
`CASES` table.
"""

import sys
from pathlib import Path

TESTS = Path("crates/postio-app/tests")

# Named by the headless runner's watchdog, so it runs on its own (#272).
ALLOWED_PREFIX = "e2e"

# The files that keep a process of their own, and why. Each also says so in
# its own doc comment; this list is what makes the check enforce that the set
# does not grow quietly.
#
# Both of these write a user-overlay preset row into a temporary
# `XDG_CONFIG_HOME` and need discovery to read it. The table discovery reads is
# a `LazyLock` in `postio_account::discovery::builtin`, "computed once and
# shared for the life of the process" -- so the first case to resolve a preset
# fixes it for every case after, and the second silently gets the first's
# overlay. That is a fourth reason to stay out, beside the watchdog (#272), a
# private display (#45/#114) and a wall-clock budget (#841): process-global
# state computed once from the environment.
ALLOWED_FILES = {
    "backend_choice": "needs to be first to populate the preset LazyLock (#973)",
    "oauth_signin": "needs to be first to populate the preset LazyLock (#973)",
}


def main() -> int:
    if not TESTS.is_dir():
        print("app-tests-share-one-binary check skipped (no postio-app tests).")
        return 0

    stray = sorted(
        path
        for path in TESTS.glob("*.rs")
        if not path.stem.startswith(ALLOWED_PREFIX) and path.stem not in ALLOWED_FILES
    )

    if stray:
        print(
            "These files are each their own test target, so each links the "
            "whole\napplication. crates/postio-app/tests/app_suite/ exists so "
            "they do not have to:\n",
            file=sys.stderr,
        )
        for path in stray:
            print(f"  {path}", file=sys.stderr)
        print(
            "\nMove each to crates/postio-app/tests/app_suite/<name>.rs, make its\n"
            "`#[test] fn`s into `pub fn`s, and add them to that main.rs's `mod`\n"
            "list and `CASES` table. If one genuinely needs its own process — the\n"
            "watchdog finds `e2e*` by name (#272), a private display (#45/#114),\n"
            "or a wall-clock budget (#841) — say so in its doc comment and name it\n"
            f"`{ALLOWED_PREFIX}…`. See #973.",
            file=sys.stderr,
        )
        return 1

    modules = len(list((TESTS / "app_suite").glob("*.rs"))) if (TESTS / "app_suite").is_dir() else 0
    print(f"app-tests-share-one-binary check passed ({modules} suite modules).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
