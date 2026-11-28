#!/usr/bin/env python3
"""Refuse a browser engine in the Linux app's dependency graph.

The reader is Blitz (ADR 0042) and the composer is a native text view
(ADR 0039), so no part of the app needs WebKit. Without an engine, "the
composer reaches no network and runs no script" is a fact of the graph
rather than something an egress test has to keep catching (spec 015 US5).

Reads the manifests, the lock file and the Flatpak manifests as text and
never runs cargo, so it costs nothing and needs no toolchain.

`ALLOWED` names what may still carry an engine, and why. It must shrink
to nothing: an entry is a debt with a named owner, not a home.

Usage: check-no-browser-engine.py [--root DIR]
Exit status: 0 clean, 1 an engine is named outside `ALLOWED`.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ENGINE = re.compile(
    r"(?<![\w-])(webkit6(?:-sys)?|webkit2gtk[\w.-]*|webkitgtk[\w.-]*|javascriptcore6(?:-sys)?)(?![\w-])",
    re.IGNORECASE,
)

# The GTK composer's WebView, until spec 015 step 4 deletes it.
ALLOWED: dict[str, set[str]] = {
    "crates/postio-widgets/Cargo.toml": {"webkit6"},
    "Cargo.lock": {"webkit6", "webkit6-sys", "javascriptcore6", "javascriptcore6-sys"},
}


def scanned(root: Path) -> list[Path]:
    files = [root / "Cargo.toml", root / "Cargo.lock"]
    files += sorted(root.glob("crates/*/Cargo.toml"))
    files += sorted(root.glob("flatpak/*.json"))
    return [path for path in files if path.is_file()]


def violations(root: Path) -> list[str]:
    found = []
    for path in scanned(root):
        relative = path.relative_to(root).as_posix()
        allowed = {name.lower() for name in ALLOWED.get(relative, set())}
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            for match in ENGINE.finditer(line):
                if match.group(1).lower() not in allowed:
                    found.append(f"{relative}:{number}: names `{match.group(1)}`")
    return found


def main(argv: list[str]) -> int:
    root = Path(__file__).resolve().parents[2]
    if len(argv) == 2 and argv[0] == "--root":
        root = Path(argv[1]).resolve()
    elif argv:
        print(__doc__, file=sys.stderr)
        return 2
    found = violations(root)
    if not found:
        return 0
    for line in found:
        print(line, file=sys.stderr)
    print(
        "\nA browser engine is back in the graph. The composer is a native text view (ADR 0039)\n"
        "and the reader is Blitz (ADR 0042): find what pulled it in "
        "(`cargo tree -i webkit6`) and remove it.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
