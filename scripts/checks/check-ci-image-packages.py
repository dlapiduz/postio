#!/usr/bin/env python3
"""The CI image installs every system package a bare runner does.

Two places install the workspace's system packages: `.github/ci-image/
Dockerfile`, which the CI and nightly jobs run inside, and
`.github/actions/rust-workspace`, which installs them on a bare runner for
`release.yml`. A library added to only the second builds every pull request
green and then fails the release, forty minutes in; added to only the first,
the reverse. So every package the action names must be in the image.

The image may hold more: it also carries what a runner ships and a container
does not (git, curl, sudo and the like).

    python3 scripts/checks/check-ci-image-packages.py

Exit status: 0 the image covers the action's list, 1 otherwise (each missing
package named), 2 a list could not be found.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

DOCKERFILE = Path(".github/ci-image/Dockerfile")
ACTION = Path(".github/actions/rust-workspace/action.yml")


def action_packages(text: str) -> set[str] | None:
    match = re.search(r'packages="([^"]*)"', text)
    return set(match.group(1).split()) if match else None


def image_packages(text: str) -> set[str] | None:
    match = re.search(r"apt-get install[^\n]*\\\n((?:[ \t]+[^\n]*\\\n)+)", text)
    if not match:
        return None
    words = match.group(1).replace("\\", " ").split()
    return {w for w in words if not w.startswith("&&") and w != "rm" and not w.startswith("-")}


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    wanted = action_packages((root / ACTION).read_text(encoding="utf-8"))
    present = image_packages((root / DOCKERFILE).read_text(encoding="utf-8"))
    if wanted is None or present is None:
        print(
            f"ci-image-packages check could not run: no package list found in "
            f"{ACTION if wanted is None else DOCKERFILE}.",
            file=sys.stderr,
        )
        return 2
    missing = sorted(wanted - present)
    if missing:
        print(
            f"ci-image-packages check FAILED: {ACTION} installs packages "
            f"{DOCKERFILE} does not.\n",
            file=sys.stderr,
        )
        for package in missing:
            print(f"  {package}", file=sys.stderr)
        print(
            "\n  The CI and nightly jobs run in the image, so a package missing "
            "there is one\n"
            "  only the release finds out about. Add it to the Dockerfile's "
            "`apt-get install`;\n"
            "  a pull request that changes the Dockerfile builds its own image.",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
