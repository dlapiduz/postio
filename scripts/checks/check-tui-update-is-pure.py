#!/usr/bin/env python3
"""The terminal's `App` never touches the filesystem: it asks with an `Effect`.

Tests drive `App::update` with no isolation, so a write in it lands in the
real config of whoever runs them. `run.rs` performs effects.

Exit status: 0 clean, 1 otherwise (each line printed).
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "crates/postio-tui/src"
FILES = [SRC / "app.rs", *sorted((SRC / "app").glob("*.rs")), *sorted((SRC / "view").glob("*.rs"))]
FORBIDDEN = re.compile(r"\bconfig_file::|\bstd::fs::|\bfs::(write|create|remove|rename|copy)\b|\bsave_search\(")


def main() -> int:
    bad = []
    for path in FILES:
        text = path.read_text(encoding="utf-8")
        live = text.split("#[cfg(test)]", 1)[0]
        for number, line in enumerate(live.splitlines(), 1):
            if line.lstrip().startswith("//"):
                continue
            if FORBIDDEN.search(line):
                bad.append(f"{path.relative_to(ROOT)}:{number}: {line.strip()}")
    for line in bad:
        print(line)
    if bad:
        print("The App writes nothing itself: return an Effect and perform it in run.rs.")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
