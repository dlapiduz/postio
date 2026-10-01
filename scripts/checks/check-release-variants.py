#!/usr/bin/env python3
"""Every package `.github/release-variants.json` lists can be built (#1714).

The release workflow builds whatever that file lists, so a row naming a
manifest that was renamed, a binary that moved, or an asset name another row
already uses fails forty minutes into a release. This finds it on the commit
that broke it. The rules are `scripts/release-variants.py check`'s, and its
docstring says what a row holds.

    python3 scripts/checks/check-release-variants.py

Exit status: 0 every row is buildable, 1 otherwise (each problem printed).
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "release-variants.py"

if __name__ == "__main__":
    raise SystemExit(subprocess.run([sys.executable, str(SCRIPT), "check", *sys.argv[1:]]).returncode)
