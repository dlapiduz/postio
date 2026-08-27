#!/usr/bin/env python3
"""Refuse `UidValidity` in engine code above the backend seam (#543).

ADR 0018 Q2: the `MailBackend` trait addresses messages by the opaque
`RemoteId`, and the IMAP adapter keeps the UIDVALIDITY-generation dance —
resync-on-renumber included — entirely behind that seam. The engine tracks
an opaque `Generation` it may only compare for equality; "this mailbox
needs a full resync" is a seam *answer*, never a comparison the engine
performs on wire counters.

The failure mode this guards is quiet: a sync- or app-layer change that
reaches for `UidValidity` compiles fine, works against IMAP, and silently
re-couples the engine to one protocol's invalidation model — the exact
coupling #542's JMAP and Gmail backends need gone. By the time a second
backend exists, unpicking it means migrating live stores again.

# The rule

Production sources (``src/``) of the crates above the seam —
``postio-sync``, ``postio-runtime``, ``postio-session``, ``postio-core``,
``postio-app``, ``postio-gtk`` — may not mention ``UidValidity``. The type
stays available in ``postio-model`` for the wire-pair columns
(`ServerIdentifiers`) and for the adapter, and tests anywhere may name it
to *configure* mocks and fixtures, which is describing a server rather
than being coupled to one.

# Exit status

0 clean, 1 a crate above the seam names UidValidity, 2 the check could
not run.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ABOVE_THE_SEAM = [
    "postio-sync",
    "postio-runtime",
    "postio-session",
    "postio-core",
    "postio-app",
    "postio-gtk",
]

WORD = re.compile(r"\bUidValidity\b")


def main() -> int:
    root = Path(__file__).resolve().parent.parent.parent
    crates = root / "crates"
    if not crates.is_dir():
        print(f"error: {crates} is not a directory", file=sys.stderr)
        return 2

    offences: list[str] = []
    for crate in ABOVE_THE_SEAM:
        src = crates / crate / "src"
        if not src.is_dir():
            continue
        for path in sorted(src.rglob("*.rs")):
            for number, line in enumerate(
                path.read_text(encoding="utf-8").splitlines(), start=1
            ):
                if WORD.search(line):
                    offences.append(f"{path.relative_to(root)}:{number}: {line.strip()}")

    if offences:
        print("UidValidity above the backend seam (#543, ADR 0018 Q2):")
        for offence in offences:
            print(f"  {offence}")
        print(
            "\nfix: address messages by RemoteId, track the mailbox's opaque\n"
            "Generation, and let the adapter answer 'needs a resync' -- the\n"
            "generation dance lives behind the MailBackend seam."
        )
        return 1

    print(f"uidvalidity-behind-the-seam check passed ({len(ABOVE_THE_SEAM)} crates).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
