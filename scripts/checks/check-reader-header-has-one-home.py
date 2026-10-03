#!/usr/bin/env python3
"""The reading pane's header rules have exactly one definition.

`postio_ui::reader::header` owns the part of the header that has no toolkit in
it: how a sender reads, how a recipient list joins, what a missing subject
says, and how an opened message is dated. #1259 is why -- macOS had no header
at all, and a mail client that does not say who a message is from is one you
cannot safely act on, because every phishing judgement starts with the sender.

The rule this enforces is #1259's last acceptance line: *anything toolkit-free
is shared, not duplicated*. It exists because the fix landed in two halves --
`postio-ui` gained the module on `feature/macos`, and the GTK app kept its own
private copies of all six names, so the same six rules existed twice (#1285).

Two frontends drawing different senders for the same message has already
happened once here (#1150). A boundary that carries the answer cannot be read
two ways, and the header is the surface where a disagreement is most visible to
a user.

The rule holds in every desktop crate: the shared
`postio-widgets` that holds the reader both desktop apps draw (ADR 0043), and
`postio-gtk`.

Fix: delete the private copy from the desktop crate and call
`postio_ui::reader::header` instead. If a rule genuinely needs to differ per
toolkit, it is layout rather than content -- and layout is what
`MessageHeader` deliberately does not carry, since it holds rendered strings
rather than widgets.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OWNER = "crates/postio-ui/src/reader/header.rs"
# The crates whose `src/` draws a header: the desktop app, and the crate
# holding the reader it draws (ADR 0043; specs/007-postio-focus R1).
CRATES = ("postio-widgets", "postio-gtk")

# The toolkit-free *rules* #1285 names. Each is matched as a definition, not a
# use, so calling through to postio-ui is exactly what this check wants to see.
#
# `MessageHeader` is deliberately absent. Both crates define one and that is
# correct: `postio_ui`'s carries rendered strings, `postio_gtk`'s is the widget
# that draws them. A name in common is not a rule in common, and a check that
# confused the two would push a frontend into renaming its widget to satisfy
# a rule about content.
SHARED = {
    "address_line": r"\bfn\s+address_line\b",
    "address_list": r"\bfn\s+address_list\b",
    "subject_text": r"\bfn\s+subject_text\b",
    "absolute_date": r"\bfn\s+absolute_date\b",
    "NO_SUBJECT": r"\bconst\s+NO_SUBJECT\b",
}


def main(argv: list[str]) -> int:
    # `--root DIR` is for the self-test: a fixture repository.
    root = Path(argv[2]) if len(argv) == 3 and argv[1] == "--root" else ROOT
    owner = root / OWNER
    if not owner.exists():
        print(f"{OWNER} is missing: the shared header rules have no home.")
        print("Fix: land postio_ui::reader::header (#1285).")
        return 1

    owner_text = owner.read_text(encoding="utf-8")
    missing = [name for name, pattern in SHARED.items() if not re.search(pattern, owner_text)]
    if missing:
        print(f"{OWNER} does not define: {', '.join(sorted(missing))}")
        print("Fix: the shared module must own every rule this check forbids duplicating.")
        return 1

    searched = [root / "crates" / crate / "src" for crate in CRATES]
    absent = [str(path.relative_to(root)) for path in searched if not path.is_dir()]
    if absent:
        print(f"reader-header check could not run: {', '.join(absent)} missing", file=sys.stderr)
        return 2

    found: list[str] = []
    for base in searched:
        for path in sorted(base.rglob("*.rs")):
            text = path.read_text(encoding="utf-8")
            for name, pattern in SHARED.items():
                if re.search(pattern, text):
                    found.append(f"  {path.relative_to(root).as_posix()}: defines {name}")

    if found:
        print("The reading pane's header rules are defined twice (#1285, #1259):")
        print("\n".join(found))
        print()
        print("Fix: delete the private copy and call postio_ui::reader::header.")
        print("Anything toolkit-free is shared, not duplicated -- two frontends")
        print("drawing different senders for the same message has happened (#1150).")
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
