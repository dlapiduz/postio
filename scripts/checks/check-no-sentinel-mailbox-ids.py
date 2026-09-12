#!/usr/bin/env python3
"""Refuse a negative `MailboxId`, which is an id that means "not an id".

The sidebar draws rows that are not folders: Flagged, Snoozed, and now the
Outbox. They are questions about messages filed elsewhere, so there is nothing
to `SELECT` and no row in `mailboxes` to point at.

For a long time `postio-gtk::feed` gave them ids anyway — `MailboxId::new(-1)`
and `MailboxId::new(-2)` — and relied on every reader remembering that a
negative id is not real. Two things went wrong with that, and both are the same
thing:

  * **A reader that forgets gets silence, not an error.** A sentinel travels
    everywhere a real id does. `MessageSet::InMailbox { mailbox: -1 }` matches
    no rows and reports success; `Command::Move` to -1 is a foreign key that
    does not resolve. The comment on `FLAGGED_ROW` listed the places it must
    never reach, which is a rule a compiler cannot check.

  * **A frontend that never knew has no rows at all.** The convention lived in
    one widget, so the macOS sidebar — which reads the same store through the
    same shared layer — simply never had Flagged or Snoozed.

Spec 003 replaced it: a view row is *unassigned*, which is already this
codebase's word for "not a row in the database", and `SidebarChoice` makes a
reader say which kind of thing it has rather than remember a number.

# The rule

No `MailboxId::new` with a negative literal, anywhere under `crates/`.

Prose is exempt: this file and the module docs that explain what was replaced
have to be able to name the thing they replaced.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATES = ROOT / "crates"

# `MailboxId::new(-1)`, and the same through a `use`d alias.
SENTINEL = re.compile(r"MailboxId::new\(\s*-\s*\d+")


def offenders() -> list[tuple[Path, int, str]]:
    found: list[tuple[Path, int, str]] = []
    for path in sorted(CRATES.rglob("*.rs")):
        for number, line in enumerate(path.read_text().splitlines(), start=1):
            stripped = line.lstrip()
            # Prose may name it; code may not.
            if stripped.startswith("//") or stripped.startswith("*"):
                continue
            if SENTINEL.search(line):
                found.append((path.relative_to(ROOT), number, stripped))
    return found


def main() -> int:
    found = offenders()
    if not found:
        print("no-sentinel-mailbox-ids check passed.")
        return 0

    print("no-sentinel-mailbox-ids check FAILED\n", file=sys.stderr)
    for path, number, line in found:
        print(f"  {path}:{number}: {line}", file=sys.stderr)
    print(
        f"\n{len(found)} occurrence(s).\n\n"
        "A negative id is a row that does not exist wearing the clothes of one\n"
        "that does. It reaches queries that then match nothing and report\n"
        "success, and it only works while every reader remembers the\n"
        "convention — which the macOS frontend never did, and so never drew\n"
        "the rows at all.\n\n"
        "A sidebar row that is a view has no id. Build it with\n"
        "`postio_ui::sidebar::view_rows`, which leaves the id unassigned, and\n"
        "tell the two apart with `postio_gtk::sidebar::SidebarChoice` rather\n"
        "than by the sign of a number.\n\n"
        "See specs/003-outbox-and-reserved-mailboxes/contracts/sidebar-rows.md.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
