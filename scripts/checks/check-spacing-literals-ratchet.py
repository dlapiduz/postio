#!/usr/bin/env python3
"""Spacing in the GTK frontend comes from the token ramp, and the literals
that remain may only shrink.

Rust spacing had fourteen distinct values -- 4, 6, 8, 9, 10, 12, 14, 16, 18,
20, 22 ... -- beside a design system whose ramp is 3.4px steps, which no
`i32` could name. `widgets::space` is that ramp in whole pixels now
(`S1` 3 .. `S8` 27, generated from the same tokens as `--postio-space-N`),
and `widgets::SettingsGroup` lays out a settings column on it.

The rest of `crates/postio-gtk/src` still has literal margins and box
spacings, and converting every one at once would be a rewrite nobody could
review. So this is a ratchet: each file's count of spacing literals --
`set_margin_*(N)`, `set_spacing(N)`, `set_row_spacing(N)`,
`set_column_spacing(N)`, `gtk::Box::new(.., N)` with N not 0 -- may not grow
past the number recorded in `spacing-literals-baseline.txt`. `widgets/` is
exempt: it is where the ramp is applied.

Fix: use `crate::widgets::space::S*` (or `SettingsGroup`) instead of a new
literal. When a file's count drops, lower its line in the baseline so it
cannot grow back: `scripts/checks/check-spacing-literals-ratchet.py --write`.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCES = ROOT / "crates/postio-gtk/src"
BASELINE = Path(__file__).resolve().parent / "spacing-literals-baseline.txt"

LITERAL = re.compile(
    r"\.set_margin_(?:top|bottom|start|end)\(\s*[1-9]\d*\s*\)"
    r"|\.set_(?:row_|column_)?spacing\(\s*[1-9]\d*\s*\)"
    r"|Box::new\(\s*gtk::Orientation::\w+\s*,\s*[1-9]\d*\s*\)"
)


def counts(sources: Path) -> dict[str, int]:
    found: dict[str, int] = {}
    for path in sorted(sources.rglob("*.rs")):
        rel = path.relative_to(sources).as_posix()
        if rel.startswith("widgets/"):
            continue
        text = path.read_text(encoding="utf-8").split("#[cfg(test)]", 1)[0]
        lines = [line for line in text.splitlines() if not line.lstrip().startswith("//")]
        count = len(LITERAL.findall("\n".join(lines)))
        if count:
            found[rel] = count
    return found


def read_baseline(path: Path) -> dict[str, int]:
    baseline: dict[str, int] = {}
    if not path.exists():
        return baseline
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        rel, count = line.rsplit(" ", 1)
        baseline[rel] = int(count)
    return baseline


def write_baseline(path: Path, found: dict[str, int]) -> None:
    lines = [
        "# Spacing literals per file in crates/postio-gtk/src, outside widgets/.",
        "# A ratchet: check-spacing-literals-ratchet.py fails if a file's count grows.",
        "# Lower a line (or run the check with --write) when a file's count drops.",
    ]
    lines += [f"{rel} {count}" for rel, count in sorted(found.items())]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def main(argv: list[str]) -> int:
    args = [a for a in argv[1:] if a != "--write"]
    sources = Path(args[0]) if args else SOURCES
    baseline_path = Path(args[1]) if len(args) > 1 else BASELINE
    found = counts(sources)
    if "--write" in argv:
        write_baseline(baseline_path, found)
        print(f"wrote {baseline_path.name}: {sum(found.values())} literals in {len(found)} files")
        return 0

    baseline = read_baseline(baseline_path)
    grown = [
        f"  {rel}: {count} spacing literals, baseline {baseline.get(rel, 0)}"
        for rel, count in sorted(found.items())
        if count > baseline.get(rel, 0)
    ]
    if grown:
        print("New spacing literals in crates/postio-gtk/src (the ramp is widgets::space):")
        print("\n".join(grown))
        print()
        print("Fix: crate::widgets::space::S1..S8, or widgets::SettingsGroup for a")
        print("settings column, instead of a new literal margin or spacing.")
        return 1
    shrunk = [rel for rel, count in baseline.items() if found.get(rel, 0) < count]
    if shrunk:
        print(
            "spacing-literals ratchet: fewer literals than recorded in "
            + ", ".join(sorted(shrunk))
            + " -- lower the baseline (--write) so they cannot grow back."
        )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
