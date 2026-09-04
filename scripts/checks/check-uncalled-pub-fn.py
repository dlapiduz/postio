#!/usr/bin/env python3
"""Refuse a `pub fn` that nothing in the workspace calls.

Three times a mechanism has been written, tested, documented, and wired to
nothing — and its own unit tests passed the whole time, so the suite said
nothing at all:

  * ``MailBackend::list_mailboxes`` had no production caller for the life of
    the project. ``MockBackend::new()`` invented an INBOX, so no test ever had
    to say where folders come from.
  * ``index_body`` (#327) was written, tested, benched and uncalled, so
    ``search_documents.body`` was empty on **every message in every real
    store** and search matched metadata only.
  * ``collect_garbage``, ``purge_temporary`` and ``evict_to_fit`` (#416) were
    uncalled together, so deleting mail freed nothing, ever, and a
    ``UIDVALIDITY`` reset orphaned a whole mailbox permanently.

Three is not bad luck. It is what a workspace of deliberately independent
leaf crates produces: a crate that cannot see its callers cannot notice it
has none, and `cargo` will not warn, because `pub` is public API by
definition. `docs/engineering-notes.md` states the pattern directly — *a
`pub fn` in a leaf crate, fully tested, is not evidence that anything calls
it.*

# The rule

A `pub fn` defined in a non-frontend crate must be named at least once from
somewhere that is not a test. Anything else is listed in the baseline beside
this file, and the baseline may only shrink.

Matching is by name, not by type. That is deliberate and it is what makes the
check cheap enough to run on every commit: a method reached only through
``dyn MailBackend`` has no direct call site to resolve, but the call still
*writes* ``backend.list_mailboxes(...)``, so a name is exactly what a caller
leaves behind. The cost is that two different functions sharing a name cover
for each other. That trades a false negative for never having a false
positive from type inference, which is the right way round for a check whose
whole risk is crying wolf.

# What is not a call

**Comments are not calls.** All three failures above had thorough docs;
``collect_garbage`` was referenced from other doc comments as *the* mechanism
that prevents leaks. Matching prose would hide precisely what this exists to
find, so comments and string literals are blanked before anything is counted.

**Tests are not calls.** ``crates/*/tests``, ``benches`` and ``examples``, and
every ``#[cfg(test)]`` item, are removed from the caller side. Getting this
wrong is the whole failure: all three functions above had passing tests.

# What is not scanned

Definitions come from the crates the application drives, not the ones that
drive it:

  * **The frontends** (``postio-gtk``, ``postio-app``, ``postio-ffi``) are
    skipped. A widget's ``pub fn banner_visible`` exists so a test on a real
    display can read the widget back, and ``postio-ffi``'s surface is called
    from Swift, which this scan cannot see. Both are legitimately uncalled
    from Rust, in bulk, and neither is where any of the three failures lived.
  * **Test-support modules** shipped in ``src/`` — mocks, the corpus
    registry, seeds, the in-process IMAP server — exist to be called from
    tests and nowhere else.
  * ``#[doc(hidden)]`` items, which is how this codebase already marks "a
    test reaches this and nothing else should".
  * ``#[uniffi::export]`` blocks, for the same reason as ``postio-ffi``.
  * A ``pub fn`` gated on ``#[cfg(feature = "...")]`` where the feature name
    contains ``test`` -- ``test-support``, ``testing``, ``test-corpus`` and
    ``test-server`` all appear in this workspace, for a helper that ships in
    ``src/`` (so it is not a test-support module by path) but is still only
    ever meant to be reached from a test. #882 found this one: string
    literals are blanked before ``#[cfg(test)]`` is matched, so
    ``#[cfg(feature = "test-support")]`` never read as test scaffolding and
    the only mark that worked was ``#[doc(hidden)]``.

# Why a baseline rather than an allow-list

This check arrived long after the code it checks, and it finds around a
hundred functions on `main` that nothing calls. Most are ordinary public API
that happens to have no in-workspace caller yet; a few are real. Writing a
reason for each is a hundred guesses by whoever adds the check, which is how
an allow-list gets silenced wholesale the first time it is wrong.

So the existing set is recorded as **debt**, in ``uncalled-pub-fn-baseline.txt``,
and the check guards the derivative: a `pub fn` that becomes uncalled today
fails, which is the day the mistake is cheap. The baseline is also checked in
the other direction — an entry that has since gained a caller, or that no
longer exists, fails and must be deleted — so the list can only shrink and
cannot rot into a list of things that used to be true.

# Exit status

0 clean, 1 something is uncalled or the baseline is stale, 2 the check could
not run.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

# --- What is scanned for definitions ----------------------------------------

# Crates whose public surface is called from outside Rust, or from tests on a
# real display. Skipped wholesale rather than allow-listed one function at a
# time: the reason applies to the crate, not to any particular function in it.
FRONTENDS = frozenset({"postio-gtk", "postio-app", "postio-ffi"})

# Test-support shipped in `src/` because tests in several crates need it.
# Matched on the path so a new mock lands under the same reasoning.
TEST_SUPPORT = (
    "test_support",
    "test_corpus",
    "test_server",
    "/mock.rs",
    "/seed.rs",
)

# The crate that exists only to support tests.
TEST_SUPPORT_CRATES = frozenset({"postio-test-support"})

BASELINE = Path("scripts/checks/uncalled-pub-fn-baseline.txt")

# --- Rust, as much of it as counting names needs ----------------------------

DEFINITION = re.compile(
    r"\bpub(?:\s*\([^)]*\))?\s+"
    r'(?:(?:async|const|unsafe|extern\s+"[^"]*")\s+)*'
    r"fn\s+([A-Za-z_][A-Za-z0-9_]*)"
)
IDENTIFIER = re.compile(r"\b[A-Za-z_][A-Za-z0-9_]*\b")
CFG_TEST = re.compile(r"#\[cfg\((?:any\([^)]*\))?[^\]]*\btest\b[^\]]*\)\]")
# Same idea, a different way a `pub fn` says "tests only": a feature name
# containing "test" -- `test-support`, `testing`, `test-corpus`, `test-server`
# all appear in this workspace. Matched against `head` below, which is *not*
# comment/string-blanked -- see `definitions`' own note on why that is
# deliberate rather than a gap to close.
CFG_TEST_FEATURE = re.compile(r'#\[cfg\([^\]]*feature\s*=\s*"[^"]*test[^"]*"[^\]]*\)\]')
UNIFFI = re.compile(r"#\[uniffi::export[^\]]*\]")

# How far back to look for an attribute that exempts a definition. Long enough
# to clear the doc comment that sits between an attribute and its `pub fn` --
# but doc comments are blanked before this runs, so it is looking at
# whitespace and other attributes only.
ATTRIBUTE_REACH = 400


class CheckError(Exception):
    """The check could not be run, as opposed to: the check failed."""


def blank_comments_and_strings(text: str) -> str:
    """`text` with comments and string literals replaced by spaces.

    Offsets are preserved so a line number computed afterwards still points at
    the real line. Blanking rather than deleting is the whole trick: a
    ``/// See [`collect_garbage`]`` becomes whitespace, and the doc comments
    that made all three of these failures look wired up stop counting as
    callers.
    """
    out = list(text)
    index, length = 0, len(text)
    while index < length:
        char = text[index]
        if char == "/" and text[index + 1 : index + 2] == "/":
            end = text.find("\n", index)
            end = length if end < 0 else end
        elif char == "/" and text[index + 1 : index + 2] == "*":
            # Rust block comments nest, unlike C's.
            depth, end = 1, index + 2
            while end < length and depth:
                if text[end : end + 2] == "/*":
                    depth, end = depth + 1, end + 2
                elif text[end : end + 2] == "*/":
                    depth, end = depth - 1, end + 2
                else:
                    end += 1
        elif char == '"':
            end = index + 1
            while end < length:
                if text[end] == "\\":
                    end += 2
                    continue
                if text[end] == '"':
                    end += 1
                    break
                end += 1
        else:
            index += 1
            continue
        for position in range(index, end):
            out[position] = " "
        index = end
    return "".join(out)


def attribute_spans(text: str, pattern: re.Pattern[str]) -> list[tuple[int, int]]:
    """`(start, end)` of every ``#[attr] item { ... }`` matching `pattern`.

    Braces are counted rather than matched by regex, because the item a
    ``#[cfg(test)]`` guards is a whole `mod` with arbitrary nesting inside it.
    """
    spans = []
    for match in pattern.finditer(text):
        opening = text.find("{", match.end())
        if opening < 0:
            continue
        depth, end = 0, opening
        while end < len(text):
            if text[end] == "{":
                depth += 1
            elif text[end] == "}":
                depth -= 1
                if depth == 0:
                    end += 1
                    break
            end += 1
        spans.append((match.start(), end))
    return spans


def blank_spans(text: str, spans: list[tuple[int, int]]) -> str:
    """`text` with each span replaced by spaces, offsets preserved."""
    out = list(text)
    for start, end in spans:
        for position in range(start, end):
            out[position] = " "
    return "".join(out)


def tracked(*patterns: str) -> list[Path]:
    """Every tracked file matching `patterns`, as git sees it.

    Asking git rather than walking keeps the scan off build output and off
    another session's scratch files. Both a ``**`` and a flat pattern per
    shape, because ``git ls-files`` does not treat ``**`` the way a shell
    does and every real source file here is nested.
    """
    try:
        listed = subprocess.run(
            ["git", "ls-files", *patterns],
            capture_output=True,
            text=True,
            check=True,
        )
    except FileNotFoundError as error:
        raise CheckError("git is not on PATH") from error
    except subprocess.CalledProcessError as error:
        raise CheckError(f"git ls-files failed: {error.stderr.strip()}") from error
    return [Path(line) for line in listed.stdout.splitlines() if line]


def crate_of(path: Path) -> str:
    """The crate directory name a source path sits in."""
    parts = path.parts
    return parts[1] if len(parts) > 1 and parts[0] == "crates" else ""


def scanned_for_definitions(path: Path) -> bool:
    """Whether `path` is somewhere a `pub fn` owes a caller."""
    crate = crate_of(path)
    if crate in FRONTENDS or crate in TEST_SUPPORT_CRATES:
        return False
    posix = path.as_posix()
    return not any(marker in posix for marker in TEST_SUPPORT)


def definitions(paths: list[Path]) -> tuple[dict[str, list[str]], dict[Path, set[int]]]:
    """Every `pub fn` that owes a caller, and where each name is *defined*.

    The second return value is what keeps a definition from counting as its
    own caller: the offset of every defined name, per file, so the caller
    scan can step over exactly those and nothing else. Every definition site
    is recorded there — including the exempt ones — because a `pub fn` that
    is skipped still must not vouch for a same-named one that is not.
    """
    found: dict[str, list[str]] = {}
    sites: dict[Path, set[int]] = {}
    for path in paths:
        try:
            raw = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        clean = blank_comments_and_strings(raw)
        body = blank_spans(clean, attribute_spans(clean, CFG_TEST))
        offsets: set[int] = set()
        scanned = scanned_for_definitions(path)
        for match in DEFINITION.finditer(body):
            offsets.add(match.start(1))
            if not scanned:
                continue
            # Attributes sit above the `pub fn` with only whitespace and
            # other attributes between, the doc comment having been blanked.
            # Raw, not `clean`/`body`: the feature-name pattern needs to read
            # what a string literal actually says, which blanking erased
            # from those two on purpose (`blank_comments_and_strings`'s own
            # docs). The trade is the same one `#[doc(hidden)]` already
            # makes on this same line -- a doc comment that happens to quote
            # the attribute as prose reads the same as the real thing -- and
            # it is the right trade for the same reason: a false exemption
            # is a debt line worth reviewing, a false uncalled-fn report is
            # noise on every commit.
            head = raw[max(0, match.start()) - ATTRIBUTE_REACH : match.start()]
            if (
                "#[doc(hidden)]" in head
                or "#[uniffi" in head
                or CFG_TEST_FEATURE.search(head)
            ):
                continue
            line = body.count("\n", 0, match.start(1)) + 1
            found.setdefault(match.group(1), []).append(f"{path.as_posix()}:{line}")
        sites[path] = offsets
    return found, sites


def called(paths: list[Path], sites: dict[Path, set[int]]) -> set[str]:
    """Every identifier named by production code.

    Test files are skipped whole and ``#[cfg(test)]`` items are cut out of
    the rest, because a function called only by its own tests is exactly what
    this check is looking for.
    """
    names: set[str] = set()
    for path in paths:
        parts = path.parts
        if any(part in ("tests", "benches", "examples") for part in parts):
            continue
        try:
            raw = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        clean = blank_comments_and_strings(raw)
        body = blank_spans(clean, attribute_spans(clean, CFG_TEST))
        skip = sites.get(path, set())
        for match in IDENTIFIER.finditer(body):
            if match.start() in skip:
                continue
            names.add(match.group(0))
    return names


def read_baseline() -> set[str]:
    """The recorded debt: one `pub fn` name per line, `#` comments ignored."""
    if not BASELINE.exists():
        return set()
    try:
        text = BASELINE.read_text(encoding="utf-8")
    except OSError as error:
        raise CheckError(f"cannot read {BASELINE}: {error}") from error
    entries = set()
    for line in text.splitlines():
        entry = line.split("#", 1)[0].strip()
        if entry:
            entries.add(entry)
    return entries


def main() -> int:
    try:
        sources = tracked("crates/*/src/**/*.rs", "crates/*/src/*.rs")
        everything = tracked("crates/**/*.rs")
        baseline = read_baseline()
    except CheckError as error:
        print(f"cannot run the check: {error}", file=sys.stderr)
        return 2

    defined, sites = definitions(sources)
    live = called(everything, sites)

    uncalled = {name for name in defined if name not in live}
    fresh = sorted(uncalled - baseline)
    # An entry that has gained a caller, or whose function is gone. Either
    # way the line is no longer true, and a list of things that used to be
    # true is how an allow-list stops being read.
    stale = sorted(baseline - uncalled)

    if not fresh and not stale:
        print(
            f"uncalled-pub-fn check passed "
            f"({len(defined)} pub fn, {len(baseline)} known uncalled)."
        )
        return 0

    print("uncalled-pub-fn check FAILED\n", file=sys.stderr)

    if fresh:
        print("  nothing calls these:\n", file=sys.stderr)
        for name in fresh:
            for where in defined[name]:
                print(f"    {where}: {name}", file=sys.stderr)
        print(
            "\n  A `pub fn` with no caller outside its own tests is the shape\n"
            "  of #327 and #416: a mechanism written, tested, documented and\n"
            "  wired to nothing, with a green suite the whole time. Doc\n"
            "  comments do not count as callers -- `collect_garbage` was\n"
            "  named in three of them and called from none.\n\n"
            "  Wire it up, or delete it. If it is deliberately public API\n"
            "  with no in-workspace caller yet, add its name to\n"
            f"  {BASELINE} with a comment saying why.",
            file=sys.stderr,
        )

    if stale:
        if fresh:
            print(file=sys.stderr)
        print("  the baseline claims these are uncalled, and they are not:\n", file=sys.stderr)
        for name in stale:
            gone = "now called" if name in live else "no longer defined"
            print(f"    {name} ({gone})", file=sys.stderr)
        print(
            f"\n  Good news, and the fix is to delete those lines from\n"
            f"  {BASELINE}. The list is debt, not permission: it is allowed\n"
            "  to shrink and never to drift.",
            file=sys.stderr,
        )

    return 1


if __name__ == "__main__":
    sys.exit(main())
