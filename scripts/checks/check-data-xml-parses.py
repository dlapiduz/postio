#!/usr/bin/env python3
"""Every XML file Postio ships is well-formed.

`dev.postio.Postio.metainfo.xml` is the AppStream metadata GNOME Software
reads, and nothing in this repository ever parsed it. It was malformed for
three releases: a comment spelled a command out in full,

    drawn by `cargo run -p postio-app
    --example shot` over a seeded store

and XML forbids a double hyphen inside a comment, so the whole file was
unparseable. Nothing noticed, because the only thing that parses it is
`appstreamcli compose` inside the Flatpak build -- which runs *after* an
eleven-minute release build, on a tag, on the one workflow with no dry run.
v0.4.x reached it on the fifth attempt at cutting a release and failed with

    E: metainfo-parsing-error
    E: filters-but-no-output

A parse is not a validation: `appstreamcli validate` knows about required
tags, release ordering and OARS ratings, and is the better check when it is
installed. It usually is not, here or on a runner outside the Flatpak SDK,
and a check that silently does nothing is how this got here. So this asks
only the question every machine can answer -- does it parse -- and asks it
in `check.sh`, seconds after the file is edited rather than minutes into a
release.

    python3 scripts/checks/check-data-xml-parses.py            # the repository
    python3 scripts/checks/check-data-xml-parses.py --root DIR  # a fixture
Exit status: 0 clean, 1 a file this names does not parse.
"""

from __future__ import annotations

import argparse
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# Where shipped XML lives. Not a whole-tree walk: `target/` alone holds
# thousands of XML files belonging to dependencies, and none of them are ours
# to keep well-formed.
SHIPPED = ("crates/*/data/**/*.xml", "flatpak/**/*.xml")


def shipped_files(root: Path) -> list[Path]:
    found: list[Path] = []
    for pattern in SHIPPED:
        found.extend(sorted(root.glob(pattern)))
    return found


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args(argv)

    files = shipped_files(args.root)
    broken: list[tuple[Path, str]] = []
    for path in files:
        try:
            ET.parse(path)
        except ET.ParseError as error:
            broken.append((path, str(error)))

    if broken:
        print("XML that will not parse:", file=sys.stderr)
        for path, error in broken:
            print(f"  {path.relative_to(args.root)}: {error}", file=sys.stderr)
        print(
            "\nFix the file. A double hyphen inside an XML comment is the one "
            "that has bitten this repository -- rephrase the comment rather "
            "than escaping it.",
            file=sys.stderr,
        )
        return 1

    print(f"data-xml check passed ({len(files)} file(s) parse).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
