#!/usr/bin/env python3
"""A button's look is one of `widgets::button`'s kinds, never a raw class.

Postio had six looks for a button: libadwaita's `.suggested-action` pill in
the header, the composer, onboarding and search; a pale accent tint in the
action bars; a solid square in settings; `.postio-ghost`; three bordered
secondaries; and the parts panel's own. Each was a class somebody added by
hand, so the next surface picked whichever it had last seen.

`widgets::button::style(widget, Kind, Size)` is the one way now: Primary,
Secondary, Ghost or Destructive, Small or Regular. This refuses, anywhere in
`crates/postio-gtk/src` outside `widgets/`, adding the classes that used to
say it: `suggested-action`, `destructive-action`, `postio-ghost`, and the
retired `postio-settings-primary` and `postio-settings-small-button`.

Fix: `crate::widgets::button::style(&button, Kind::…, Size::…)`, or
`widgets::button::button(label, kind, size)` for a new one; an icon-only
button is `widgets::icon_button(icon, name)`.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCES = ROOT / "crates/postio-gtk/src"

RAW = re.compile(
    r'add_css_class\(\s*"(suggested-action|destructive-action|postio-ghost|'
    r'postio-settings-primary|postio-settings-small-button)"'
)


def main(argv: list[str]) -> int:
    sources = Path(argv[1]) if len(argv) == 2 else SOURCES
    problems = []
    for path in sorted(sources.rglob("*.rs")):
        rel = path.relative_to(sources).as_posix()
        if rel.startswith("widgets/"):
            continue
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if line.lstrip().startswith("//"):
                continue
            match = RAW.search(line)
            if match:
                problems.append(f"  {rel}:{number}: .{match.group(1)}")
    if not problems:
        return 0
    print("Buttons styled with a raw class rather than a widgets::button kind:")
    print("\n".join(problems))
    print()
    print("Fix: crate::widgets::button::style(&button, Kind::…, Size::…); an icon-only")
    print("button is widgets::icon_button(icon, name).")
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
