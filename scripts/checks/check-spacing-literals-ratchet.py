#!/usr/bin/env python3
"""Spacing in the GTK frontends comes from the token ramp, and the literals
that remain may only shrink.

Rust spacing had fourteen distinct values -- 4, 6, 8, 9, 10, 12, 14, 16, 18,
20, 22 ... -- beside a design system whose ramp is 3.4px steps, which no
`i32` could name. `widgets::space` is that ramp in whole pixels now
(`S1` 3 .. `S8` 27, generated from the same tokens as `--postio-space-N`),
and `widgets::SettingsGroup` lays out a settings column on it.

The rest of the desktop crates' `src/` -- the shared `postio-widgets` (ADR
0043) and `postio-gtk` -- still has literal margins
and box spacings, and converting every one at once would be a rewrite nobody
could review. So this is a ratchet: each file's count of spacing literals --
`set_margin_*(N)`, `set_spacing(N)`, `set_row_spacing(N)`,
`set_column_spacing(N)`, `gtk::Box::new(.., N)` with N not 0 -- may not grow
past the number recorded in `spacing-literals-baseline.txt`. A file is named
there by its path inside its crate's `src/`, so one that moves between the
desktop crates keeps its line. `widgets/` is exempt: it is where the ramp is
applied.

Fix: use `widgets::space::S*` (or `SettingsGroup`) instead of a new literal.
When a file's count drops, lower its line in the baseline so it cannot grow
back: `scripts/checks/check-spacing-literals-ratchet.py --write`.

Usage:
    python3 scripts/checks/check-spacing-literals-ratchet.py [--write]      # the repository
    python3 scripts/checks/check-spacing-literals-ratchet.py --root DIR     # a fixture repository
    python3 scripts/checks/check-spacing-literals-ratchet.py SRC_DIR BASELINE
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
# The crates whose `src/` lays out widgets: the desktop app, and the crate
# holding what it draws (ADR 0043; specs/007-postio-focus R1).
CRATES = ("postio-widgets", "postio-gtk")
BASELINE = Path("scripts/checks/spacing-literals-baseline.txt")

LITERAL = re.compile(
    r"\.set_margin_(?:top|bottom|start|end)\(\s*[1-9]\d*\s*\)"
    r"|\.set_(?:row_|column_)?spacing\(\s*[1-9]\d*\s*\)"
    r"|Box::new\(\s*gtk::Orientation::\w+\s*,\s*[1-9]\d*\s*\)"
)


def counts(sources: list[tuple[Path, str]]) -> tuple[dict[str, int], dict[str, list[str]]]:
    """Each file's count, by its path inside its crate's `src/`, and where it is."""
    found: dict[str, int] = {}
    where: dict[str, list[str]] = {}
    for base, shown in sources:
        for path in sorted(base.rglob("*.rs")):
            rel = path.relative_to(base).as_posix()
            if rel.startswith("widgets/"):
                continue
            text = path.read_text(encoding="utf-8").split("#[cfg(test)]", 1)[0]
            lines = [line for line in text.splitlines() if not line.lstrip().startswith("//")]
            count = len(LITERAL.findall("\n".join(lines)))
            if count:
                found[rel] = found.get(rel, 0) + count
                where.setdefault(rel, []).append(shown + rel)
    return found, where


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
        "# Spacing literals per file in the desktop crates' src/, outside widgets/,",
        "# each named by its path inside its crate's src/ (postio-widgets,",
        "# postio-gtk). A ratchet: check-spacing-literals-ratchet.py fails if a",
        "# file's count grows. Lower a line (or run the check with --write) when a",
        "# file's count drops.",
    ]
    lines += [f"{rel} {count}" for rel, count in sorted(found.items())]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def inputs(args: list[str]) -> tuple[list[tuple[Path, str]], Path] | None:
    """What to scan, with how its files are shown, and the baseline; None if a crate is missing."""
    if args and args[0] != "--root":
        return [(Path(args[0]), "")], Path(args[1]) if len(args) > 1 else ROOT / BASELINE
    root = Path(args[1]) if args else ROOT
    sources = [(root / "crates" / crate / "src", f"crates/{crate}/src/") for crate in CRATES]
    missing = [shown for path, shown in sources if not path.is_dir()]
    if missing:
        print(f"spacing-literals ratchet could not run: {', '.join(missing)} missing", file=sys.stderr)
        return None
    return sources, root / BASELINE


def main(argv: list[str]) -> int:
    given = inputs([a for a in argv[1:] if a != "--write"])
    if given is None:
        return 2
    sources, baseline_path = given
    found, where = counts(sources)
    if "--write" in argv:
        write_baseline(baseline_path, found)
        print(f"wrote {baseline_path.name}: {sum(found.values())} literals in {len(found)} files")
        return 0

    baseline = read_baseline(baseline_path)
    grown = [
        f"  {', '.join(where[rel])}: {count} spacing literals, baseline {baseline.get(rel, 0)}"
        for rel, count in sorted(found.items())
        if count > baseline.get(rel, 0)
    ]
    if grown:
        print("New spacing literals in the desktop crates (the ramp is widgets::space):")
        print("\n".join(grown))
        print()
        print("Fix: widgets::space::S1..S8, or widgets::SettingsGroup for a")
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
