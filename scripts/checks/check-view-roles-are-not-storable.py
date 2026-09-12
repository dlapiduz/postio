#!/usr/bin/env python3
"""Keep the `mailboxes.role` CHECK and `MailboxRole::kind()` saying the same thing.

A mailbox role is either a **folder** on the server or a **view** over messages
filed elsewhere. `Flagged`, `Snoozed` and `Outbox` are views: they have a name,
a position and a count, and nothing else — no path, no UIDVALIDITY, no sync
state, and nothing a message could be moved into.

Two things enforce that a view is never stored, and they are written in
different languages:

  * `MailboxRole::kind()` in `crates/postio-model/src/mailbox.rs`, which
    `MailboxRepository::create`/`update` consult before writing; and
  * the `CHECK (role IN (...))` on `mailboxes.role` in
    `crates/postio-storage/src/migrations/0001_initial_schema.sql`.

Nothing makes them agree. They already disagreed once: `MailboxRole::Snoozed`
has existed in the enum and in `from_name` since the sidebar gained the row,
and could never be stored — not by decision, but because whoever wrote the
`CHECK` listed the roles that existed that day and nobody revisited it. That
worked, silently, for the wrong reason. The next role added to one and not the
other would break just as silently in the other direction: a view that stores
fine, and a sidebar row that a `Command::Move` can target.

# The rule

The set of roles spelled in the `CHECK` must be exactly the set
`MailboxRole::kind()` answers `Folder` for.

Both sides are read out of the source rather than restated here, so this file
has no third copy of the list to drift from the other two.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MODEL = ROOT / "crates/postio-model/src/mailbox.rs"
SCHEMA = ROOT / "crates/postio-storage/src/migrations/0001_initial_schema.sql"


def folder_roles(source: str) -> set[str]:
    """The roles `kind()` answers `Folder` for, in their stored spelling.

    Read as two passes over `mailbox.rs`: the `Folder` arm of `kind()` gives
    the variant names, and `as_str` maps each to the string the schema uses.
    """
    kind = re.search(
        r"pub fn kind\(self\) -> RoleKind \{(.*?)\n    \}", source, re.S
    )
    if not kind:
        raise SystemExit(
            "check-view-roles-are-not-storable: no `kind()` in mailbox.rs.\n"
            "If it was renamed, update this check — do not delete it."
        )
    # Comments inside the arms mention other items by path (`Self::from_special_use`
    # explains why Flagged is a folder), and a naive `Self::(\w+)` sweep would
    # read those as variants. Strip comment lines first.
    body = "\n".join(
        line for line in kind.group(1).splitlines() if not line.lstrip().startswith("//")
    )
    folder_arm = re.search(r"(.*?)=>\s*RoleKind::Folder", body, re.S)
    if not folder_arm:
        raise SystemExit(
            "check-view-roles-are-not-storable: `kind()` has no `RoleKind::Folder` arm."
        )
    variants = set(re.findall(r"Self::(\w+)", folder_arm.group(1)))

    spellings = dict(re.findall(r'Self::(\w+) => "(\w+)"', source))
    missing = variants - spellings.keys()
    if missing:
        raise SystemExit(
            "check-view-roles-are-not-storable: no `as_str` spelling for "
            f"{', '.join(sorted(missing))}."
        )
    return {spellings[variant] for variant in variants}


def checked_roles(schema: str) -> set[str]:
    """The role spellings the `mailboxes.role` CHECK permits."""
    match = re.search(
        r"\brole\s+TEXT\s+NOT NULL\s+DEFAULT\s+'regular'\s*"
        r"CHECK\s*\(\s*role IN \(([^)]*)\)",
        schema,
    )
    if not match:
        raise SystemExit(
            "check-view-roles-are-not-storable: no CHECK on `mailboxes.role` in "
            "0001_initial_schema.sql.\n"
            "If the column moved to a later migration, point this check at it —\n"
            "do not delete it."
        )
    return set(re.findall(r"'(\w+)'", match.group(1)))


def main() -> int:
    folders = folder_roles(MODEL.read_text())
    checked = checked_roles(SCHEMA.read_text())

    storable_views = checked - folders
    unstorable_folders = folders - checked
    if not storable_views and not unstorable_folders:
        print(f"view-roles-are-not-storable check passed ({len(folders)} folder role(s)).")
        return 0

    print("view-roles-are-not-storable check FAILED\n", file=sys.stderr)
    for role in sorted(storable_views):
        print(
            f"  `{role}` is in the schema's CHECK but is not a RoleKind::Folder.\n"
            f"    A view would store as a real mailbox, and the sidebar row it\n"
            f"    draws could be the target of a move.",
            file=sys.stderr,
        )
    for role in sorted(unstorable_folders):
        print(
            f"  `{role}` is a RoleKind::Folder but is not in the schema's CHECK.\n"
            f"    Discovery will fail to store a folder the server really has.",
            file=sys.stderr,
        )
    print(
        "\nThe two lists must match exactly:\n"
        f"    {MODEL.relative_to(ROOT)}  — the `RoleKind::Folder` arm of `kind()`\n"
        f"    {SCHEMA.relative_to(ROOT)}  — CHECK (role IN (...))\n\n"
        "Adding a role means deciding whether it names a folder or is a view,\n"
        "and saying so in both places. See\n"
        "specs/003-outbox-and-reserved-mailboxes/contracts/mailbox-role.md.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
