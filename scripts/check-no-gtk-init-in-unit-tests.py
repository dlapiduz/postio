#!/usr/bin/env python3
"""Refuse `adw::init()` from a unit test, which aborts the suite.

GTK may be initialized from exactly one thread in a process. `cargo test`
runs a crate's unit tests on a thread pool inside a single process, so two
unit tests that both initialize GTK are two threads racing for something that
tolerates one — and the loser aborts:

    thread 'toast::tests::only_an_undoable_completion_offers_the_button'
    panicked at gtk4/src/rt.rs: Attempted to initialize GTK from two
    different threads.

    Gdk-ERROR **: gdk_display_manager_get() was called before gtk_init()
    postio_gtk-... (signal: 6, SIGABRT)

Whether it aborts depends on which thread wins and whether a display exists,
which is why `crates/postio-gtk/src/toast.rs` survived every developer
machine and killed the workspace test job the first time a display-less
runner got far enough to run it. A whole crate's tests died on a signal, so
the 305 that would have passed were never reported at all.

Cargo gives every *integration* test its own process, so the process-wide
init is safe there. Anything in a crate with a lib target that needs a
display belongs in `tests/`, not in a `#[cfg(test)] mod tests`.

# The rule

No file under ``crates/*/src`` may initialize GTK inside a test region — a
`#[test]` function, or anything under `#[cfg(test)]`. Production code is
untouched: `postio-gtk/src/app.rs` and `postio-app/src/lib.rs` both call
`adw::init()` on the main thread, which is exactly right.

A crate with no lib target cannot move the test out — an integration test
under `tests/` has nothing to link against. `postio-app` is in that position
and keeps exactly one GTK-touching unit test. Such a file carries a
``POSTIO-GTK-INIT:`` line saying so, which is the only way past this check,
and it is per-file rather than per-crate so the second one has to be argued
for too.

# Exit status

0 clean, 1 a unit test initializes GTK, 2 the check could not run.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

# --- What counts as initializing GTK ---------------------------------------
#
# `init`, `init_check`, and libadwaita's `init` all reach `gtk_init`, and all
# three are process-wide. Spelled as a path so a local `fn init()` is not
# mistaken for one.

INIT = re.compile(r"\b(?:adw|libadwaita|gtk|gtk4)\s*::\s*(init\w*)\s*\(")

# `#[...]`, with `test` as a whole word inside it: `#[test]`, `#[cfg(test)]`,
# `#[cfg(all(test, unix))]`, `#[gtk::test]`. Attribute bodies are scanned
# after string literals have been blanked, so `#[cfg(feature = "testing")]`
# cannot match.
TEST_WORD = re.compile(r"\btest\b")

# The marker that says a human decided this one has to stay.
MARKER = "POSTIO-GTK-INIT:"


class CheckError(Exception):
    """The check could not be run, as opposed to: the check failed."""


def tracked_sources() -> list[Path]:
    """Every Rust file under a crate's ``src``, as git sees it.

    Asking git rather than walking the tree keeps the check off build output
    and off another session's scratch files, and means an untracked
    experiment cannot fail somebody else's run.
    """
    try:
        listed = subprocess.run(
            ["git", "ls-files", "crates/*/src/**/*.rs", "crates/*/src/*.rs"],
            capture_output=True,
            text=True,
            check=True,
        )
    except FileNotFoundError as error:
        raise CheckError("git is not on PATH") from error
    except subprocess.CalledProcessError as error:
        raise CheckError(f"git ls-files failed: {error.stderr.strip()}") from error

    return [Path(line) for line in listed.stdout.splitlines() if line]


def blank_noise(text: str) -> str:
    """`text` with comments and literals replaced by spaces, length preserved.

    Everything after this is brace counting and pattern matching, and both
    are wrong on raw Rust: `const CSS: &str = "window { }"` would close a
    module early, and a doc comment saying "does not call `adw::init()`"
    would be read as a call. Offsets and newlines are preserved so a match
    still maps back to the right line.
    """
    out = list(text)
    index = 0
    length = len(text)

    def blank(start: int, stop: int) -> None:
        for position in range(start, min(stop, length)):
            if out[position] != "\n":
                out[position] = " "

    while index < length:
        char = text[index]

        # Line comment.
        if text.startswith("//", index):
            stop = text.find("\n", index)
            stop = length if stop == -1 else stop
            blank(index, stop)
            index = stop
            continue

        # Block comment. Rust nests them, so count depth rather than
        # stopping at the first `*/`.
        if text.startswith("/*", index):
            depth = 1
            cursor = index + 2
            while cursor < length and depth:
                if text.startswith("/*", cursor):
                    depth += 1
                    cursor += 2
                elif text.startswith("*/", cursor):
                    depth -= 1
                    cursor += 2
                else:
                    cursor += 1
            blank(index, cursor)
            index = cursor
            continue

        # Raw string: `r"..."`, `r#"..."#`, and the `br` byte forms.
        raw = re.match(r'(?:b?r)(#*)"', text[index : index + 260])
        if raw and (index == 0 or not (text[index - 1].isalnum() or text[index - 1] == "_")):
            hashes = raw.group(1)
            closing = '"' + hashes
            stop = text.find(closing, index + raw.end())
            stop = length if stop == -1 else stop + len(closing)
            blank(index, stop)
            index = stop
            continue

        # Ordinary string, and the `b"..."` byte form.
        if char == '"':
            cursor = index + 1
            while cursor < length:
                if text[cursor] == "\\":
                    cursor += 2
                    continue
                if text[cursor] == '"':
                    cursor += 1
                    break
                cursor += 1
            blank(index, cursor)
            index = cursor
            continue

        # A quote is a char literal or a lifetime, and only one of them ends.
        # `'}'` must not close a module; `'a` must not swallow the rest of
        # the file looking for a partner it does not have.
        if char == "'":
            if text.startswith("'\\", index):
                stop = text.find("'", index + 2)
                stop = index + 1 if stop == -1 else stop + 1
            elif index + 2 < length and text[index + 2] == "'":
                stop = index + 3
            else:
                stop = index + 1  # a lifetime: blank just the quote
            blank(index, stop)
            index = stop
            continue

        index += 1

    return "".join(out)


def test_regions(clean: str) -> list[tuple[int, int]]:
    """Half-open ``(start, stop)`` spans of everything compiled only for tests.

    A test attribute applies to the item that follows it, so the span runs
    from the attribute to the end of that item — the matching `}` for a
    module or function, or the `;` for a `use`. Getting the end right is the
    whole job: a file whose test module is mistakenly unterminated would
    condemn every line of production code below it.
    """
    spans: list[tuple[int, int]] = []
    length = len(clean)
    index = 0

    while index < length:
        if not clean.startswith("#[", index):
            index += 1
            continue

        # Bracket-match the attribute, so `#[cfg(all(test, unix))]` is read
        # whole rather than ending at the first `]`.
        depth = 0
        cursor = index + 1
        while cursor < length:
            if clean[cursor] == "[":
                depth += 1
            elif clean[cursor] == "]":
                depth -= 1
                if depth == 0:
                    break
            cursor += 1
        if cursor >= length:
            break
        attribute_end = cursor + 1

        if not TEST_WORD.search(clean[index:attribute_end]):
            index = attribute_end
            continue

        # Walk to the item this attribute decorates, stepping over any
        # further attributes stacked between them.
        cursor = attribute_end
        while cursor < length:
            if clean[cursor] == "#" and clean.startswith("#[", cursor):
                depth = 0
                while cursor < length:
                    if clean[cursor] == "[":
                        depth += 1
                    elif clean[cursor] == "]":
                        depth -= 1
                        if depth == 0:
                            cursor += 1
                            break
                    cursor += 1
                continue
            if clean[cursor] in "{;":
                break
            cursor += 1

        if cursor >= length:
            spans.append((index, length))
            break

        if clean[cursor] == ";":
            spans.append((index, cursor + 1))
            index = cursor + 1
            continue

        depth = 0
        while cursor < length:
            if clean[cursor] == "{":
                depth += 1
            elif clean[cursor] == "}":
                depth -= 1
                if depth == 0:
                    cursor += 1
                    break
            cursor += 1

        spans.append((index, min(cursor, length)))
        index = min(cursor, length)

    return spans


def offences(path: Path) -> list[tuple[int, str]]:
    """Every GTK init inside a test region of `path`, as ``(line, call)``."""
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return []

    if MARKER in text:
        # The decision is recorded in the file. Whether it is a *good* reason
        # is a review question, not one a scanner can answer.
        return []

    if not INIT.search(text):
        return []  # the common case, and the cheap one

    clean = blank_noise(text)
    found: list[tuple[int, str]] = []
    for start, stop in test_regions(clean):
        for match in INIT.finditer(clean, start, stop):
            line = clean.count("\n", 0, match.start()) + 1
            call = f"{match.group(0).rstrip('(').strip()}()"
            found.append((line, call))
    return sorted(set(found))


def main() -> int:
    try:
        sources = tracked_sources()
    except CheckError as error:
        print(f"cannot run the check: {error}", file=sys.stderr)
        return 2

    problems: list[str] = []
    for path in sources:
        for line, call in offences(path):
            problems.append(f"{path}:{line}: `{call}` in a unit test")

    if not problems:
        print(f"no-gtk-init-in-unit-tests check passed ({len(sources)} files).")
        return 0

    print("no-gtk-init-in-unit-tests check FAILED\n", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        f"\n{len(problems)} occurrence(s).\n\n"
        "GTK may be initialized from one thread per process, and `cargo test`\n"
        "runs a crate's unit tests on a thread pool in one process. A second\n"
        "one aborts the whole binary with SIGABRT, taking every other test in\n"
        "the crate with it — and whether it aborts depends on which thread\n"
        "wins, so it passes locally and kills CI.\n\n"
        "Move the test to `crates/<crate>/tests/`, where cargo gives it a\n"
        "process of its own. See `crates/postio-gtk/tests/gtk_toast.rs`.\n\n"
        "If the crate has no lib target, an integration test has nothing to\n"
        "link against and the test has to stay. Say so in the file, on a\n"
        "line like:\n\n"
        f"    // {MARKER} `postio-app` is a binary crate, so this cannot\n"
        "    // move to `tests/`. It is the only GTK-touching test here.\n\n"
        'See CLAUDE.md, "Testing", and issue #41.',
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
