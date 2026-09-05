#!/usr/bin/env python3
"""Bump the workspace version everywhere it is written by hand, for a
tag-triggered release. See #886.

Cutting v0.2.0 meant bumping `[workspace.package] version` in the root
`Cargo.toml`, six internal path-dependency pins that name it explicitly
(`postio-model = { version = "0.1.0", path = "../postio-model" }` and
siblings -- `cargo check` fails otherwise, since nothing else in the
workspace resolves them), and adding a changelog entry to the AppStream
metainfo GNOME Software reads, all by hand. This is that, mechanised: the
part with no judgment calls, which is exactly the part that should never
depend on a person remembering all of it correctly under time pressure.

Deliberately does not touch `Cargo.lock` or run `cargo` at all -- a
release workflow runs `cargo check --workspace --all-targets` right after
this anyway (to verify the bump compiles), and that regenerates the lock
file as a side effect. Keeping this script to text edits only is what
lets it run in well under a second with no toolchain, in a test.

Usage:
    scripts/release-bump.py <version> --notes-file <path> [--date YYYY-MM-DD]

Run from the repository root (the paths below are relative to it, the same
way scripts/issue-land.sh and its siblings assume). Exit status: 0 on a
clean bump, 1 if the version is not new, not shaped like semver, or the
notes file is missing -- refused before anything is written, never
half-applied.
"""

from __future__ import annotations

import argparse
import datetime
import glob
import re
import sys
from pathlib import Path

CARGO_TOML = Path("Cargo.toml")
METAINFO = Path("crates/postio-gtk/data/dev.postio.Postio.metainfo.xml")
INTERNAL_PIN = re.compile(r'version = "([0-9]+\.[0-9]+\.[0-9]+)", path = "\.\./postio-')
SEMVER = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+$")


def current_version(cargo_toml: str) -> str | None:
    """The one `version = "..."` line, not `rust-version` or anything else."""
    match = re.search(r'(?m)^version = "([0-9.]+)"$', cargo_toml)
    return match.group(1) if match else None


def bump_workspace_version(cargo_toml: str, old: str, new: str) -> str:
    pattern = re.compile(r'(?m)^version = "' + re.escape(old) + r'"$')
    return pattern.sub(f'version = "{new}"', cargo_toml, count=1)


def bump_internal_pins(root: Path, old: str, new: str) -> None:
    """Every sibling path dependency that pins the old version explicitly.

    Scoped to `path = "../postio-` so an unrelated third-party crate that
    happens to share the old version number is never touched -- that scope
    is the whole reason this is a regex over the pin's own text and not a
    blanket find-and-replace of the version string.
    """
    for manifest_path in sorted(glob.glob(str(root / "crates" / "*" / "Cargo.toml"))):
        manifest = Path(manifest_path)
        text = manifest.read_text(encoding="utf-8")
        replaced = INTERNAL_PIN.sub(
            lambda m: f'version = "{new}", path = "../postio-'
            if m.group(1) == old
            else m.group(0),
            text,
        )
        if replaced != text:
            manifest.write_text(replaced, encoding="utf-8")


def insert_changelog_entry(
    metainfo: str, *, version: str, date: str, notes: str
) -> str:
    """Newest release first, matching every existing entry in the file."""
    indent = "    "
    entry = (
        f'{indent}<release version="{version}" date="{date}">\n'
        f"{indent}  <description>\n"
        f"{indent}    <p>\n{notes.rstrip()}\n{indent}    </p>\n"
        f"{indent}  </description>\n"
        f"{indent}</release>\n"
    )
    marker = "<releases>\n"
    idx = metainfo.index(marker)
    if idx == -1:
        raise ValueError("no <releases> element found")
    insert_at = idx + len(marker)
    return metainfo[:insert_at] + entry + metainfo[insert_at:]


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", help="new version, e.g. 0.3.0")
    parser.add_argument("--notes-file", required=True, help="changelog text to insert")
    parser.add_argument(
        "--date",
        default=datetime.date.today().isoformat(),
        help="release date, YYYY-MM-DD (defaults to today)",
    )
    args = parser.parse_args(argv)

    if not SEMVER.match(args.version):
        print(f"not a semver version: {args.version!r}", file=sys.stderr)
        return 1

    notes_path = Path(args.notes_file)
    if not notes_path.is_file():
        print(f"notes file not found: {notes_path}", file=sys.stderr)
        return 1

    root = Path.cwd()
    cargo_toml_path = root / CARGO_TOML
    cargo_toml = cargo_toml_path.read_text(encoding="utf-8")
    old_version = current_version(cargo_toml)
    if old_version is None:
        print(f"could not find a workspace version in {cargo_toml_path}", file=sys.stderr)
        return 1
    if old_version == args.version:
        print(f"already at {args.version}", file=sys.stderr)
        return 1

    metainfo_path = root / METAINFO
    metainfo = metainfo_path.read_text(encoding="utf-8")
    notes = notes_path.read_text(encoding="utf-8")

    cargo_toml_path.write_text(
        bump_workspace_version(cargo_toml, old_version, args.version), encoding="utf-8"
    )
    bump_internal_pins(root, old_version, args.version)
    metainfo_path.write_text(
        insert_changelog_entry(
            metainfo, version=args.version, date=args.date, notes=notes
        ),
        encoding="utf-8",
    )

    print(f"bumped {old_version} -> {args.version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
