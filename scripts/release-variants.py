#!/usr/bin/env python3
"""What a release ships: `.github/release-variants.json`, read and checked.

A release builds one package per row of that file (#1714). There is one row
per *package*, not per app: the terminal app ships twice, as a Flatpak and as
a tarball. Adding a package of a kind the workflow already knows how to build
is one row. A new kind is a job in `release.yml` as well.

Each row has an `id` (the app's name in asset names), a `kind`, an `asset`
pattern, and whatever that kind needs:

    flatpak    app_id, manifest     the manifest's `app-id` must equal app_id
    tarball    package, bin,        a workspace package, one of its binaries,
               profile, readme      a cargo profile, the README shipped beside it
    macos-app  app                  the bundle `scripts/macos-bundle.sh` makes

Asset names follow `<id>-{version}-<os>-<arch>.<ext>`, so a release page lists
every package one way and a script can find one without knowing its history.
Two rows may not upload the same name: the second upload would silently
replace the first.

Usage:
    scripts/release-variants.py check
    scripts/release-variants.py matrix <kind> --version X.Y.Z   # JSON list, for fromJSON
    scripts/release-variants.py assets --version X.Y.Z          # one asset per line, SBOMs too
    (any of them with --root DIR, for a fixture tree)

Exit status: 0 fine, 1 the file has a problem (each is printed), 2 usage.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from pathlib import Path

VARIANTS = Path(".github/release-variants.json")

KINDS = {
    "flatpak": ("app_id", "manifest"),
    "tarball": ("package", "bin", "profile", "readme"),
    "macos-app": ("app",),
}
EXTENSIONS = {"flatpak": "flatpak", "tarball": "tar.zst", "macos-app": "zip"}
ASSET = re.compile(
    r"^(?P<id>[a-z0-9-]+)-\{version\}-(?P<os>linux|macos)-(?P<arch>x86_64|arm64|universal)"
    r"\.(?P<ext>flatpak|tar\.zst|zip)$"
)


def load(root: Path) -> list[dict]:
    data = json.loads((root / VARIANTS).read_text(encoding="utf-8"))
    return data["variants"]


def resolve(row: dict, version: str) -> dict:
    asset = row["asset"].replace("{version}", version)
    return dict(row, asset=asset, sbom=f"{asset}.spdx.json")


def package_dir(root: Path, name: str) -> tuple[Path, dict] | None:
    for manifest in sorted((root / "crates").glob("*/Cargo.toml")):
        data = tomllib.loads(manifest.read_text(encoding="utf-8"))
        if data.get("package", {}).get("name") == name:
            return manifest.parent, data
    return None


def binaries(directory: Path, manifest: dict) -> set[str]:
    names = {b["name"] for b in manifest.get("bin", []) if "name" in b}
    if (directory / "src" / "main.rs").exists():
        names.add(manifest["package"]["name"])
    names.update(p.stem for p in (directory / "src" / "bin").glob("*.rs"))
    return names


def problems_with(root: Path, rows: list[dict]) -> list[str]:
    problems: list[str] = []
    seen: dict[str, int] = {}
    workspace = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    profiles = set(workspace.get("profile", {})) | {"dev", "release", "test", "bench"}

    for index, row in enumerate(rows):
        where = f"row {index} ({row.get('id', '?')}, {row.get('kind', '?')})"
        kind = row.get("kind")
        if kind not in KINDS:
            problems.append(f"{where}: kind {kind!r} is not one release.yml builds ({', '.join(KINDS)})")
            continue
        missing = [key for key in ("id", "asset", *KINDS[kind]) if not row.get(key)]
        if missing:
            problems.append(f"{where}: missing {', '.join(missing)}")
            continue

        asset = row["asset"]
        shape = ASSET.match(asset)
        if not shape:
            problems.append(f"{where}: asset {asset!r} is not <id>-{{version}}-<os>-<arch>.<ext>")
        else:
            if shape["id"] != row["id"]:
                problems.append(f"{where}: asset {asset!r} does not start with its id {row['id']!r}")
            if shape["ext"] != EXTENSIONS[kind]:
                problems.append(f"{where}: a {kind} is a .{EXTENSIONS[kind]}, not {asset!r}")
        if asset in seen:
            problems.append(f"{where}: asset {asset!r} is also row {seen[asset]}'s -- one would replace the other")
        seen.setdefault(asset, index)

        if kind == "flatpak":
            manifest = root / row["manifest"]
            if not manifest.is_file():
                problems.append(f"{where}: manifest {row['manifest']} does not exist")
            else:
                app_id = json.loads(manifest.read_text(encoding="utf-8")).get("app-id")
                if app_id != row["app_id"]:
                    problems.append(
                        f"{where}: app_id {row['app_id']} but {row['manifest']} builds {app_id}"
                    )
        elif kind == "tarball":
            found = package_dir(root, row["package"])
            if found is None:
                problems.append(f"{where}: no workspace package named {row['package']}")
            elif row["bin"] not in binaries(*found):
                problems.append(f"{where}: {row['package']} has no binary {row['bin']}")
            if row["profile"] not in profiles:
                problems.append(f"{where}: the workspace defines no profile {row['profile']}")
            if not (root / row["readme"]).is_file():
                problems.append(f"{where}: readme {row['readme']} does not exist")
        elif kind == "macos-app":
            if not (root / "macos" / "Package.swift").is_file():
                problems.append(f"{where}: there is no macos/Package.swift to build {row['app']} from")
    return problems


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", default=str(Path(__file__).resolve().parent.parent))
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("check")
    matrix = sub.add_parser("matrix")
    matrix.add_argument("kind")
    matrix.add_argument("--version", required=True)
    assets = sub.add_parser("assets")
    assets.add_argument("--version", required=True)
    args = parser.parse_args(argv)

    root = Path(args.root)
    rows = load(root)

    if args.command == "check":
        problems = problems_with(root, rows)
        for problem in problems:
            print(f"FAIL: {VARIANTS}: {problem}")
        if problems:
            return 1
        print(f"release-variants check passed ({len(rows)} packages).")
        return 0
    if args.command == "matrix":
        print(json.dumps([resolve(r, args.version) for r in rows if r.get("kind") == args.kind]))
        return 0
    for row in rows:
        resolved = resolve(row, args.version)
        print(resolved["asset"])
        print(resolved["sbom"])
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
