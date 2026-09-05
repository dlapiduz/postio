#!/usr/bin/env python3
"""Refuse a tokio-dependent await on the GTK main context.

`0.1.0` panicked on the main thread the instant a login succeeded:

    thread 'main' panicked at postio-account/src/secret.rs:314:
    there is no reactor running, must be called from the context of a
    Tokio 1.x runtime

The line was an ordinary-looking ``secrets.store(&key, &password).await``
inside a ``glib::spawn_future_local`` block. It type-checks. Clippy is happy.
It panics only when that line is actually reached, which for onboarding means
only on a real first run against a real server -- so every automated signal
this project has was green while the application could not add an account.

`postio-app/src/feed.rs` already states the rule this violates:

    The frontend's futures are awaited by `glib::spawn_future_local` on the
    GTK main context. The store's are tokio futures [...] Neither loop can
    drive the other.

It was written down, and followed everywhere except the one path no test
could reach. `postio-66` asked for the guard that would have caught it by
inspection, having concluded that neither a stub secret store nor a
D-Bus-hosted Secret Service would: a fake built to need a reactor proves
nothing about the real one.

# The rule

Inside a ``spawn_future_local`` block, a ``.await`` may only be a channel
receive -- ``.recv()``, ``.next()`` -- because that is the crossing. Runtime
work is spawned onto the runtime and answered over a channel; it is never
awaited here.

Nested ``runtime.spawn(...)`` / ``tokio::spawn(...)`` blocks are *not*
checked. Those run on the runtime, which is the whole point, and their awaits
are unrestricted.

An await that is genuinely safe but is not a receive has to say so:

    // POSTIO-GLIB-SAFE: `MessageSource::fetch` returns a future the
    // implementation guarantees is pollable on the main context.
    match future.await {

The marker does not make anything true. What it does is force the decision to
be *made and written down* rather than arriving inside a diff about something
else -- which is the failure mode worth catching, because nobody awaits a
tokio future on the main context on purpose.

# Exit status

0 clean, 1 an unguarded await was found, 2 the check could not run.
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

# The call that opens a GTK-main-context future.
CROSSING = "spawn_future_local"

# Calls whose argument list runs on the tokio runtime instead. An await inside
# one of these is correct by construction, so their blocks are cut out before
# anything is checked.
RUNTIME_SPAWNS = (".spawn(", "tokio::spawn(", "spawn_blocking(")

# What a `.await` on the main context is allowed to be: taking the answer off
# a channel, or pulling the next item from a stream.
RECEIVES = re.compile(r"\.(recv|recv_async|next|recv_timeout)\s*\(\s*\)\s*$")

# The marker that says a human decided this await is pollable here.
MARKER = "POSTIO-GLIB-SAFE:"

# A line that is part of the comment block immediately above an await.
COMMENT = re.compile(r"^\s*(//.*)?$")


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


def blanked(text: str) -> str:
    """`text` with comments and string literals replaced by spaces.

    Length and line structure are preserved, so every offset into the result
    still points at the same place in the original -- which is what lets a
    match be reported with a real line number.
    """
    out = list(text)
    index = 0
    length = len(text)
    while index < length:
        pair = text[index : index + 2]
        if pair == "//":
            while index < length and text[index] != "\n":
                out[index] = " "
                index += 1
        elif pair == "/*":
            depth = 1
            out[index] = out[index + 1] = " "
            index += 2
            while index < length and depth:
                if text[index : index + 2] == "/*":
                    depth += 1
                    out[index] = out[index + 1] = " "
                    index += 2
                elif text[index : index + 2] == "*/":
                    depth -= 1
                    out[index] = out[index + 1] = " "
                    index += 2
                else:
                    if text[index] != "\n":
                        out[index] = " "
                    index += 1
        elif text[index] == '"':
            out[index] = " "
            index += 1
            while index < length and text[index] != '"':
                if text[index] == "\\":
                    out[index] = " "
                    index += 1
                    if index < length:
                        if text[index] != "\n":
                            out[index] = " "
                        index += 1
                    continue
                if text[index] != "\n":
                    out[index] = " "
                index += 1
            if index < length:
                out[index] = " "
                index += 1
        elif text[index] == "'":
            # A quote is a char literal or a lifetime, and only one of them
            # ends. `'"'` holds one double quote, so reading it as the start
            # of a string inverts the quote parity of everything after it in
            # the file: real code gets blanked as "string", string contents
            # get scanned as code, and a crossing past that point silently
            # stops being seen. A lifetime (`&'a str`, `'static`, a loop
            # label) must fall through with only its own quote blanked, or
            # the scan runs off looking for a partner quote that never comes.
            if text[index : index + 2] == "'\\":
                out[index] = " "
                index += 1
                out[index] = " "
                index += 1
                while index < length and text[index] != "'":
                    if text[index] != "\n":
                        out[index] = " "
                    index += 1
                if index < length:
                    out[index] = " "
                    index += 1
            elif index + 2 < length and text[index + 2] == "'":
                out[index] = out[index + 1] = out[index + 2] = " "
                index += 3
            else:
                out[index] = " "
                index += 1
        else:
            index += 1
    return "".join(out)


def balanced(text: str, start: int, opener: str, closer: str) -> int:
    """Index just past the `opener` at `start`'s matching `closer`."""
    depth = 0
    index = start
    while index < len(text):
        if text[index] == opener:
            depth += 1
        elif text[index] == closer:
            depth -= 1
            if depth == 0:
                return index + 1
        index += 1
    return len(text)


def crossings(text: str) -> list[tuple[int, int]]:
    """The `(start, end)` span of every `spawn_future_local` argument list."""
    spans = []
    for match in re.finditer(re.escape(CROSSING), text):
        opening = text.find("(", match.end())
        if opening == -1:
            continue
        spans.append((opening, balanced(text, opening, "(", ")")))
    return spans


def runtime_regions(text: str, start: int, end: int) -> list[tuple[int, int]]:
    """Spans within `start..end` that run on the runtime, not the main loop."""
    regions = []
    for spawn in RUNTIME_SPAWNS:
        index = text.find(spawn, start, end)
        while index != -1:
            opening = text.index("(", index)
            regions.append((index, balanced(text, opening, "(", ")")))
            index = text.find(spawn, opening, end)
    return regions


def covered(lines: list[str], marked: set[int], number: int) -> bool:
    """Whether the comment block above line `number` carries the marker.

    Walked rather than counted: the reason an await is safe here rarely fits
    on one line, and a fixed look-back silently stops covering a comment the
    moment somebody adds a sentence to it.
    """
    if number in marked:
        return True
    above = number - 1
    while above >= 1 and COMMENT.match(lines[above - 1]):
        if above in marked:
            return True
        above -= 1
    return False


def offences(path: Path) -> list[tuple[int, str]]:
    """Every unguarded await on the main context, as ``(line, source)``."""
    try:
        raw = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return []
    if CROSSING not in raw:
        return []

    text = blanked(raw)
    lines = raw.splitlines()
    marked = {
        number
        for number, line in enumerate(lines, start=1)
        if MARKER in line
    }

    found = []
    for start, end in crossings(text):
        cut = runtime_regions(text, start, end)
        for await_at in re.finditer(r"\.await\b", text[start:end]):
            offset = start + await_at.start()
            if any(low <= offset < high for low, high in cut):
                continue
            if RECEIVES.search(text[start:offset]):
                continue
            number = raw.count("\n", 0, offset) + 1
            if covered(lines, marked, number):
                continue
            found.append((number, lines[number - 1].strip()))
    return found


def main() -> int:
    try:
        sources = tracked_sources()
    except CheckError as error:
        print(f"cannot run the check: {error}", file=sys.stderr)
        return 2

    problems: list[str] = []
    for path in sources:
        for number, source in offences(path):
            problems.append(f"{path}:{number}: {source}")

    if not problems:
        print(f"runtime-crossing check passed ({len(sources)} files).")
        return 0

    print("runtime-crossing check FAILED\n", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        f"\n{len(problems)} await(s) on the GTK main context that are not a\n"
        "channel receive.\n\n"
        "`spawn_future_local` runs on the glib main loop, which has no tokio\n"
        "reactor. Awaiting a runtime-dependent future there panics with\n"
        "\"there is no reactor running\" the first time the line is reached --\n"
        "which shipped in 0.1.0 and made the app unable to add an account.\n\n"
        "Spawn the work on the runtime and take the answer over a channel:\n\n"
        "    let (sender, receiver) = async_channel::bounded(1);\n"
        "    runtime.spawn(async move { let _ = sender.send(work().await).await; });\n"
        "    glib::spawn_future_local(async move {\n"
        "        let answer = receiver.recv().await;\n"
        "    });\n\n"
        "If the future really is pollable on the main context, say why:\n\n"
        f"    // {MARKER} <why this one is safe here>\n",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
