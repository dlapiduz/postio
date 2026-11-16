#!/usr/bin/env python3
"""Check that the refs a Flatpak manifest asks for exist on Flathub.

The manifests are built only by the release workflow, so a runtime bump used
to find out at release time that a branch was not there yet. This is the
cheap version of that failure, run on a pull request that touches `flatpak/`.

For each manifest it works out the ref of the runtime, the SDK and every SDK
extension, and asks Flathub whether each exists. Nothing is installed.

The branch an SDK extension must be on is the SDK's own base, not the
manifest's `runtime-version`: GNOME 50 is built on freedesktop 25.08, and
`org.freedesktop.Sdk.Extension.rust-stable//50` does not exist. A GNOME SDK
says so in its metadata (`[Extension org.freedesktop.Sdk.Extension]`,
`version = 25.08`). A freedesktop SDK's metadata names no version, because
its extensions use its own, so the manifest's `runtime-version` is the answer.

Two lookups, both read-only and both injectable (`--canned DIR`, used by the
self-test, answers them from files so nothing touches the network):

  existence  HTTP HEAD on https://dl.flathub.org/repo/refs/heads/runtime/
             <id>/<arch>/<branch>: 200 exists, 404 does not. No flatpak needed.
  metadata   `flatpak remote-info --show-metadata flathub <sdk>//<branch>`
             against a user remote. It reads the summary, installs nothing
             and needs only the `flatpak` package, not a runtime.

Usage: scripts/flatpak-refs-check.py [--arch x86_64] [--canned DIR] MANIFEST...
Exit status: 0 every ref exists, 1 one is missing or unreadable.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path

FLATHUB = "https://dl.flathub.org/repo/refs/heads/runtime"
EXTENSION_SECTION = "[Extension org.freedesktop.Sdk.Extension]"


class Lookup:
    """Where answers come from. The real one asks Flathub; the canned one reads files."""

    def __init__(self, arch: str, canned: Path | None) -> None:
        self.arch = arch
        self.canned = canned
        self._refs = (
            set((canned / "refs").read_text(encoding="utf-8").split()) if canned else None
        )

    def exists(self, ref_id: str, branch: str) -> bool:
        if self._refs is not None:
            return f"{ref_id}//{branch}" in self._refs
        url = f"{FLATHUB}/{ref_id}/{self.arch}/{branch}"
        try:
            with urllib.request.urlopen(urllib.request.Request(url, method="HEAD"), timeout=30):
                return True
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return False
            raise

    def metadata(self, sdk: str, branch: str) -> str:
        if self.canned is not None:
            return (self.canned / "metadata" / f"{sdk}@{branch}").read_text(encoding="utf-8")
        out = subprocess.run(
            ["flatpak", "--user", "remote-info", "--show-metadata", "flathub", f"{sdk}//{branch}"],
            capture_output=True, text=True, timeout=120, check=True,
        )
        return out.stdout


def extension_base(metadata: str) -> str | None:
    """The `version` of the SDK's `Extension org.freedesktop.Sdk.Extension` section."""
    in_section = False
    for line in metadata.splitlines():
        line = line.strip()
        if line.startswith("["):
            in_section = line == EXTENSION_SECTION
        elif in_section:
            m = re.match(r"version\s*=\s*(\S+)", line)
            if m:
                return m.group(1)
    return None


def check(manifest: Path, lookup: Lookup) -> list[str]:
    data = json.loads(manifest.read_text(encoding="utf-8"))
    try:
        runtime, sdk = data["runtime"], data["sdk"]
        version = str(data["runtime-version"])
    except KeyError as e:
        return [f"{manifest}: no {e.args[0]!r}"]

    problems: list[str] = []
    wanted = [(runtime, version), (sdk, version)]

    extensions = data.get("sdk-extensions", [])
    if extensions:
        base = version
        if sdk != "org.freedesktop.Sdk":
            try:
                base = extension_base(lookup.metadata(sdk, version)) or ""
            except (OSError, subprocess.SubprocessError) as e:
                return [f"{manifest}: cannot read {sdk}//{version}'s metadata: {e}"]
            if not base:
                return [f"{manifest}: {sdk}//{version} names no freedesktop base for its extensions"]
        wanted += [(ext, base) for ext in extensions]

    for ref_id, branch in wanted:
        found = lookup.exists(ref_id, branch)
        print(f"{'ok     ' if found else 'MISSING'} {ref_id}//{branch}  ({manifest.name})")
        if not found:
            problems.append(f"{manifest}: {ref_id}//{branch} is not on Flathub")
    return problems


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("manifests", nargs="+", type=Path)
    ap.add_argument("--arch", default="x86_64")
    ap.add_argument("--canned", type=Path, help="answer from files in DIR instead of Flathub")
    args = ap.parse_args()

    lookup = Lookup(args.arch, args.canned)
    problems = [p for m in args.manifests for p in check(m, lookup)]
    for p in problems:
        print(f"error: {p}", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
