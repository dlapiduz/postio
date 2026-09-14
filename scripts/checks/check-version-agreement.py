#!/usr/bin/env python3
"""The workspace version, every internal pin and the newest AppStream release
entry name the same version.

A version is written in three places by hand -- `[workspace.package]
version` in the root Cargo.toml, the `postio-* = { version = "...", path =
"../postio-..." }` pins some crates carry, and the newest `<release>` in
the metainfo GNOME Software reads -- and `scripts/release-bump.py` moves
all three together on a tag push. v0.3.0 was tagged on 2026-09-11 and the
release workflow failed after the tag existed, so that bump never landed:
`main` kept saying 0.2.0, the changelog had no 0.3.0 entry, and nothing
noticed for three days, because nothing compared the three. This does.

    python3 scripts/checks/check-version-agreement.py            # the repository
    python3 scripts/checks/check-version-agreement.py --root DIR  # a fixture (self-test)

Exit status: 0 they agree, 1 they do not (each disagreement is printed).
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

METAINFO = Path("crates/postio-gtk/data/dev.postio.Postio.metainfo.xml")
WORKSPACE_VERSION = re.compile(r'(?m)^version = "([0-9]+\.[0-9]+\.[0-9]+)"$')
INTERNAL_PIN = re.compile(r'version = "([0-9]+\.[0-9]+\.[0-9]+)", path = "\.\./postio-')
RELEASE = re.compile(r'<release version="([0-9]+\.[0-9]+\.[0-9]+)"')


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default=str(Path(__file__).resolve().parent.parent.parent))
    root = Path(parser.parse_args().root)
    problems: list[str] = []

    cargo_toml = (root / "Cargo.toml").read_text(encoding="utf-8")
    match = WORKSPACE_VERSION.search(cargo_toml)
    if match is None:
        print("FAIL: no `version = \"x.y.z\"` line in Cargo.toml")
        return 1
    version = match.group(1)

    for manifest in sorted((root / "crates").glob("*/Cargo.toml")):
        for pinned in INTERNAL_PIN.findall(manifest.read_text(encoding="utf-8")):
            if pinned != version:
                problems.append(
                    f"{manifest.relative_to(root)} pins a sibling at {pinned}; the workspace is "
                    f"{version} -- run scripts/release-bump.py, or fix the pin by hand"
                )

    metainfo = root / METAINFO
    releases = RELEASE.findall(metainfo.read_text(encoding="utf-8")) if metainfo.exists() else []
    if not releases:
        problems.append(f"{METAINFO} has no <release version=...> entry")
    elif releases[0] != version:
        problems.append(
            f"{METAINFO}'s newest release is {releases[0]}; the workspace is {version} -- "
            "the changelog entry and the version move together (scripts/release-bump.py)"
        )

    if problems:
        for problem in problems:
            print(f"FAIL: {problem}")
        return 1
    print(f"version-agreement check passed ({version} everywhere).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
