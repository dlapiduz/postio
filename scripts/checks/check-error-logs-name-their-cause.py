#!/usr/bin/env python3
"""An `error!` that carries a cause says the cause in its message text.

`tracing_journald` puts a field in a journal *field* — `%error` arrives as
`F_ERROR=...` — and `MESSAGE` is the event's message and nothing else. So
`journalctl` without `-o verbose` shows the sentence and drops the reason,
which is how a lost draft came to be reported as only "could not autosave
the draft". The cause was in the journal the whole time, and invisible at
the command anyone actually runs.

The fix is to say it twice: keep the field, and interpolate it into the
message as well, so the structured record and the readable one agree.

    tracing::error!(%error, "could not autosave the draft: {error}");

This does not leak anything the log did not already hold: the same string
is already going to journald as a field, and to stderr, where `fmt` renders
fields inline. Logs still carry ids, counts and outcomes only — an error's
own text is an outcome (CLAUDE.md, "Privacy is a feature").

Deliberately `error!` and not `warn!`. A warning is something the process
carried on through, and there are 87 of those against 26 of these; this is
about the record left behind when something actually failed. Widening it
later is mechanical, and this check is where that decision would be made.

    python3 scripts/checks/check-error-logs-name-their-cause.py
    python3 scripts/checks/check-error-logs-name-their-cause.py --root DIR

Exit status: 0 clean, 1 a problem was found and printed.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

CALL = re.compile(r"\btracing::error!\s*\(")
# `%error`, `?error`, `%refusal` — a *bare* field, where the field's name and
# the local it captures are the same thing.
#
# `name = %expression` is deliberately not matched, and the reason is a trap:
# a message's `{error}` is `format_args!`, so it captures the local `error`,
# not the field of that name. Where a site logs `error =
# %redact_addresses(&error.to_string())`, interpolating `{error}` would put
# the *unredacted* value in `MESSAGE` and undo the redaction on purpose-built
# code (postio-account/src/imap/pool.rs). Asking for that would be worse than
# asking for nothing, so this check does not ask.
# `\\[\\s\\S]` rather than `\\.`, so a literal broken across lines with a
# trailing backslash — which is how the longer messages here are written — is
# one literal and not an unterminated one.
STRING = re.compile(r'"(?:[^"\\]|\\[\s\S])*"')


def arguments_of(text: str, open_paren: int) -> tuple[str, int] | None:
    """The text between the macro's parentheses, and where it ends."""
    depth = 0
    in_string = False
    escaped = False
    for i in range(open_paren, len(text)):
        character = text[i]
        if in_string:
            if escaped:
                escaped = False
            elif character == "\\":
                escaped = True
            elif character == '"':
                in_string = False
            continue
        if character == '"':
            in_string = True
        elif character == "(":
            depth += 1
        elif character == ")":
            depth -= 1
            if depth == 0:
                return text[open_paren + 1 : i], i
    return None


def bare_cause_fields(arguments: str) -> set[str]:
    """The `%name` / `?name` fields, where the name and the value are one thing.

    `name = %expression` is excluded, and the reason is the trap in the module
    docstring: interpolating such a field's name reaches the local, not the
    expression's value.
    """
    names: set[str] = set()
    for match in re.finditer(r"[%?](\w+)", arguments):
        before = arguments[: match.start()].rstrip()
        if before.endswith("="):
            continue
        after = arguments[match.end() :].lstrip()
        # A bare field ends the argument; `%path::to(x)` does not.
        if after[:1] not in {",", ")", ""}:
            continue
        names.add(match.group(1))
    return names


def names_its_cause(arguments: str) -> bool:
    """Whether some field's name is interpolated into some string literal."""
    named = bare_cause_fields(arguments)
    if not named:
        # No cause field at all — nothing for this check to ask for.
        return True
    for literal in STRING.findall(arguments):
        for name in named:
            if "{" + name in literal:
                return True
    return False


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=str(Path(__file__).resolve().parent.parent.parent))
    root = Path(parser.parse_args().root)
    problems: list[str] = []

    for path in sorted((root / "crates").rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for match in CALL.finditer(text):
            found = arguments_of(text, match.end() - 1)
            if found is None:
                continue
            arguments, _ = found
            if names_its_cause(arguments):
                continue
            line = text.count("\n", 0, match.start()) + 1
            first = " ".join(arguments.split())
            problems.append(f"{path.relative_to(root)}:{line}: {first[:90]}")

    if problems:
        print(
            f"{len(problems)} error log(s) carry a cause in a field but not in the "
            "message, so `journalctl` shows the sentence without the reason:\n"
        )
        for problem in problems:
            print(f"  {problem}")
        print(
            '\nFix: interpolate the field into the message as well --\n'
            '  tracing::error!(%error, "could not autosave the draft: {error}");\n'
            "The field stays; the message gains the reason. See this check's docstring."
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
