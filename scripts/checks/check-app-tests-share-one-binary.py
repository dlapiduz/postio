#!/usr/bin/env python3
"""Refuse a new `crates/postio-focus/tests/*.rs` that is not `e2e*`.

Every file directly under a crate's ``tests/`` is its own `[[test]]` target,
and every target in the desktop app's crate links the whole application —
GTK, the renderer, the store engine and all. Measured when this check was
written, each of those binaries was over 200 MB and the app's suite took
about eleven minutes to link.

That cost is not the reason this matters. `CLAUDE.md` prices the wiring tier
and then, correctly, tells sessions to iterate at the cheapest layer that can
fail — which routes everyone away from exactly the tests that catch this
project's characteristic bug: layers that each pass and are not joined up
(`postio-bl2`, eight instances, four of them shipped). The tests that can see
that bug are the expensive ones, so they are the ones nobody runs. Folding
them into one already-built binary is what makes the guidance to avoid them
stop being right.

`crates/postio-focus/tests/focus_suite/` is that binary: `harness = false`,
one `adw::init`, every case a plain `pub fn` run in sequence. #973 moved
seven files into the first such suite; this check is what stops a stray one
appearing.

# The rule

A file directly under ``crates/postio-focus/tests`` must be named `e2e*`,
be in ``ALLOWED_FILES``, or be the `focus_suite` directory. Nothing else.

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

Otherwise: move it to ``crates/postio-focus/tests/focus_suite/<name>.rs``,
turn each `#[test] fn` into a `pub fn`, and add it to `main.rs`'s `mod` list
and `CASES` table.
"""

import sys
from pathlib import Path

TESTS = Path("crates/postio-focus/tests")
SUITE = "focus_suite"

# Named by the headless runner's watchdog, so it runs on its own (#272).
ALLOWED_PREFIX = "e2e"

# The files that keep a process of their own, and why. Each also says so in
# its own doc comment; this list is what makes the check enforce that the set
# does not grow quietly.
ALLOWED_FILES = {
    "packaging": "reads the desktop entry, metainfo and manifests; draws nothing",
}


def main() -> int:
    if not TESTS.is_dir():
        print(f"app-tests-share-one-binary check could not run: {TESTS} is missing", file=sys.stderr)
        return 2

    stray = sorted(
        path
        for path in TESTS.glob("*.rs")
        if not path.stem.startswith(ALLOWED_PREFIX) and path.stem not in ALLOWED_FILES
    )

    if stray:
        print(
            "These files are each their own test target, so each links the "
            f"whole\napplication. {TESTS}/{SUITE}/ exists so they do not have to:\n",
            file=sys.stderr,
        )
        for path in stray:
            print(f"  {path}", file=sys.stderr)
        print(
            f"\nMove each to {TESTS}/{SUITE}/<name>.rs, make its\n"
            "`#[test] fn`s into `pub fn`s, and add them to that main.rs's `mod`\n"
            "list and `CASES` table. If one genuinely needs its own process — the\n"
            "watchdog finds `e2e*` by name (#272), a private display (#45/#114),\n"
            "or a wall-clock budget (#841) — say so in its doc comment and name it\n"
            f"`{ALLOWED_PREFIX}…`. See #973.",
            file=sys.stderr,
        )
        return 1

    modules = len(list((TESTS / SUITE).glob("*.rs"))) if (TESTS / SUITE).is_dir() else 0
    print(f"app-tests-share-one-binary check passed ({modules} suite modules).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
