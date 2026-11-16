#!/usr/bin/env python3
"""Self-test for scripts/flatpak-refs-check.py.

The script reads a Flatpak manifest's runtime, SDK and SDK extensions, works
out the branch each must be on, and asks Flathub whether each ref exists.
Here Flathub is a directory of canned answers (`--canned DIR`), so nothing
touches the network: `DIR/refs` lists the refs that exist, one
`<id>//<branch>` per line, and `DIR/metadata/<sdk>@<branch>` holds the
`flatpak remote-info --show-metadata` text of an SDK.

Usage: scripts/tests/test-flatpak-refs-check.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402

CHECK = Path(__file__).resolve().parent.parent / "flatpak-refs-check.py"
FAILURES: list[str] = []

GNOME_META = "[Runtime]\nname = org.gnome.Sdk\n\n[Extension org.freedesktop.Sdk.Extension]\ndirectory = lib/sdk\nversion = 25.08\n\n[Extension org.freedesktop.Platform.GL]\nversion = 1.4\n"
FDO_META = "[Runtime]\nname = org.freedesktop.Sdk\n\n[Extension org.freedesktop.Sdk.Extension]\ndirectory = lib/sdk\n"
RUST = "org.freedesktop.Sdk.Extension.rust-stable"


def case(name: str, condition: bool, detail: str) -> None:
    print(f"{'ok   ' if condition else 'FAIL '} {name}")
    if not condition:
        FAILURES.append(f"{name}: {detail}")


def manifest(root: Path, name: str, **fields: object) -> Path:
    path = root / name
    path.write_text(json.dumps(fields), encoding="utf-8")
    return path


def run(root: Path, *manifests: Path) -> subprocess.CompletedProcess[str]:
    return patience.run(
        ["python3", str(CHECK), "--canned", str(root / "canned"), *map(str, manifests)],
        capture_output=True, text=True, timeout=30,
    )


def canned(root: Path, refs: list[str]) -> None:
    d = root / "canned"
    (d / "metadata").mkdir(parents=True, exist_ok=True)
    (d / "refs").write_text("\n".join(refs) + "\n", encoding="utf-8")
    (d / "metadata" / "org.gnome.Sdk@51").write_text(GNOME_META, encoding="utf-8")
    (d / "metadata" / "org.freedesktop.Sdk@26.08").write_text(FDO_META, encoding="utf-8")


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw)
        gnome = manifest(root, "gnome.json", runtime="org.gnome.Platform", sdk="org.gnome.Sdk",
                         **{"runtime-version": "51", "sdk-extensions": [RUST]})
        fdo = manifest(root, "fdo.json", runtime="org.freedesktop.Platform", sdk="org.freedesktop.Sdk",
                       **{"runtime-version": "26.08", "sdk-extensions": [RUST]})
        all_refs = [
            "org.gnome.Platform//51", "org.gnome.Sdk//51", f"{RUST}//25.08",
            "org.freedesktop.Platform//26.08", "org.freedesktop.Sdk//26.08", f"{RUST}//26.08",
        ]

        canned(root, all_refs)
        r = run(root, gnome, fdo)
        case("every ref present passes", r.returncode == 0, r.stdout + r.stderr)

        # GNOME's extension is on freedesktop's branch, not GNOME's own: the
        # check must look for rust-stable//25.08, not //51.
        canned(root, [x for x in all_refs if x != f"{RUST}//25.08"])
        r = run(root, gnome)
        case("a GNOME manifest needs its extension on the SDK's freedesktop base",
             r.returncode != 0 and f"{RUST}//25.08" in r.stdout + r.stderr, r.stdout + r.stderr)

        canned(root, [x for x in all_refs if x != f"{RUST}//26.08"])
        r = run(root, fdo)
        case("a freedesktop manifest needs its extension on its own branch",
             r.returncode != 0 and f"{RUST}//26.08" in r.stdout + r.stderr, r.stdout + r.stderr)

        canned(root, [x for x in all_refs if x != "org.gnome.Platform//51"])
        r = run(root, gnome)
        case("a missing runtime is named", r.returncode != 0 and "org.gnome.Platform//51" in r.stdout + r.stderr,
             r.stdout + r.stderr)

        canned(root, [x for x in all_refs if x != "org.freedesktop.Sdk//26.08"])
        r = run(root, fdo)
        case("a missing SDK is named", r.returncode != 0 and "org.freedesktop.Sdk//26.08" in r.stdout + r.stderr,
             r.stdout + r.stderr)

        # An SDK whose metadata cannot be read must fail, not pass unchecked.
        canned(root, all_refs)
        (root / "canned" / "metadata" / "org.gnome.Sdk@51").unlink()
        r = run(root, gnome)
        case("unreadable SDK metadata fails", r.returncode != 0, r.stdout + r.stderr)

        broken = manifest(root, "broken.json", runtime="org.gnome.Platform")
        canned(root, all_refs)
        r = run(root, broken)
        case("a manifest without a runtime-version fails", r.returncode != 0, r.stdout + r.stderr)

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
