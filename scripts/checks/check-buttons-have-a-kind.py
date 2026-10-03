#!/usr/bin/env python3
"""A button's look is one of `widgets::button`'s kinds, never a raw class.

Postio had six looks for a button: libadwaita's `.suggested-action` pill in
the header, the composer, onboarding and search; a pale accent tint in the
action bars; a solid square in settings; `.postio-ghost`; three bordered
secondaries; and the parts panel's own. Each was a class somebody added by
hand, so the next surface picked whichever it had last seen.

`widgets::button::style(widget, Kind, Size)` is the one way now: Primary,
Secondary, Ghost or Destructive, Small or Regular. This refuses, anywhere in
the `src/` of a desktop crate -- the shared `postio-widgets` (ADR 0043) and
`postio-focus` -- outside its `widgets/`, adding the classes
that used to say it: `suggested-action`, `destructive-action`,
`postio-ghost`, and the retired `postio-settings-primary` and
`postio-settings-small-button`.

Fix: `widgets::button::style(&button, Kind::…, Size::…)`, or
`widgets::button::button(label, kind, size)` for a new one; an icon-only
button is `widgets::icon_button(icon, name)`.

Usage:
    python3 scripts/checks/check-buttons-have-a-kind.py             # the repository
    python3 scripts/checks/check-buttons-have-a-kind.py --root DIR  # a fixture repository
    python3 scripts/checks/check-buttons-have-a-kind.py SRC_DIR     # one crate's src/
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# The crates whose `src/` draws buttons: the desktop app, and the crate
# holding what it draws (ADR 0043; specs/007-postio-focus R1).
CRATES = ("postio-widgets", "postio-focus")

RAW = re.compile(
    r'add_css_class\(\s*"(suggested-action|destructive-action|postio-ghost|'
    r'postio-settings-primary|postio-settings-small-button)"'
)


def sources_of(argv: list[str]) -> list[tuple[Path, str]] | None:
    """Each `src/` to scan, with how its files are shown; None if one is missing."""
    args = argv[1:]
    if not args or args[0] == "--root":
        root = Path(args[1]) if args else ROOT
        sources = [(root / "crates" / crate / "src", f"crates/{crate}/src/") for crate in CRATES]
        missing = [shown for path, shown in sources if not path.is_dir()]
        if missing:
            print(f"button-kind check could not run: {', '.join(missing)} missing", file=sys.stderr)
            return None
        return sources
    return [(Path(args[0]), "")]


def main(argv: list[str]) -> int:
    sources = sources_of(argv)
    if sources is None:
        return 2
    problems = []
    for base, shown in sources:
        for path in sorted(base.rglob("*.rs")):
            rel = path.relative_to(base).as_posix()
            if rel.startswith("widgets/"):
                continue
            for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
                if line.lstrip().startswith("//"):
                    continue
                match = RAW.search(line)
                if match:
                    problems.append(f"  {shown}{rel}:{number}: .{match.group(1)}")
    if not problems:
        return 0
    print("Buttons styled with a raw class rather than a widgets::button kind:")
    print("\n".join(problems))
    print()
    print("Fix: widgets::button::style(&button, Kind::…, Size::…); an icon-only")
    print("button is widgets::icon_button(icon, name).")
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
