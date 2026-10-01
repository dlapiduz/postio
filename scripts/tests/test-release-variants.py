#!/usr/bin/env python3
"""Self-test for scripts/release-variants.py.

`.github/release-variants.json` is the list of packages a release ships, one
row each, and `release.yml` builds whatever it lists (#1714). So the file is
the release: a row that names a manifest which does not exist, or two rows
that would upload the same asset name, fails at the end of a forty-minute
run rather than here. This holds the three things the workflow asks of it
-- the matrix for one kind of package, the list of every asset a release
must carry, and the check that each row is buildable -- against fixture
trees whose problems are known.

Usage: scripts/tests/test-release-variants.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import json
import sys
import tempfile
from pathlib import Path

# The shared dial (#1249). `scripts/lib`, not beside this file, because CI
# runs every `scripts/tests/*.py` it finds as a self-test.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "release-variants.py"

FAILURES: list[str] = []

FLATPAK = {
    "id": "postio",
    "kind": "flatpak",
    "app_id": "dev.postio.Postio",
    "manifest": "flatpak/dev.postio.Postio.json",
    "asset": "postio-{version}-linux-x86_64.flatpak",
}
TUI_FLATPAK = {
    "id": "postio-tui",
    "kind": "flatpak",
    "app_id": "dev.postio.PostioTui",
    "manifest": "flatpak/dev.postio.PostioTui.json",
    "asset": "postio-tui-{version}-linux-x86_64.flatpak",
}
TARBALL = {
    "id": "postio-tui",
    "kind": "tarball",
    "package": "postio-tui",
    "bin": "postio-tui",
    "profile": "release-tui",
    "readme": "docs/terminal.md",
    "asset": "postio-tui-{version}-linux-x86_64.tar.zst",
}
MACOS = {
    "id": "postio",
    "kind": "macos-app",
    "app": "Postio.app",
    "asset": "postio-{version}-macos-arm64.zip",
}


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def world(base: Path, variants: list[dict]) -> Path:
    """A tree holding everything the four good rows point at."""
    root = base / "repo"
    (root / ".github").mkdir(parents=True)
    (root / ".github" / "release-variants.json").write_text(
        json.dumps({"variants": variants}, indent=2), encoding="utf-8"
    )
    (root / "flatpak").mkdir()
    for app_id in ("dev.postio.Postio", "dev.postio.PostioTui"):
        (root / "flatpak" / f"{app_id}.json").write_text(
            json.dumps({"app-id": app_id, "modules": []}), encoding="utf-8"
        )
    crate = root / "crates" / "postio-tui"
    crate.mkdir(parents=True)
    (crate / "Cargo.toml").write_text(
        '[package]\nname = "postio-tui"\n\n[[bin]]\nname = "postio-tui"\npath = "src/main.rs"\n',
        encoding="utf-8",
    )
    (root / "Cargo.toml").write_text(
        '[workspace]\nmembers = ["crates/*"]\n\n[profile.release-tui]\ninherits = "release"\n',
        encoding="utf-8",
    )
    (root / "docs").mkdir()
    (root / "docs" / "terminal.md").write_text("# Terminal\n", encoding="utf-8")
    (root / "macos").mkdir()
    (root / "macos" / "Package.swift").write_text("// swift-tools-version:5.9\n", encoding="utf-8")
    return root


def run(root: Path, *args: str):
    return patience.run(
        [sys.executable, str(SCRIPT), "--root", str(root), *args],
        capture_output=True,
        text=True,
        timeout=30,
    )


GOOD = [FLATPAK, TUI_FLATPAK, TARBALL, MACOS]


def main() -> int:
    if not SCRIPT.exists():
        print(f"missing {SCRIPT}", file=sys.stderr)
        return 1

    with tempfile.TemporaryDirectory() as directory:
        root = world(Path(directory), GOOD)

        r = run(root, "check")
        case("a buildable set of rows passes the check", r.returncode == 0, r.stdout + r.stderr)

        # The matrix is what `strategy.matrix` reads through fromJSON: the
        # rows of one kind, each with its asset name resolved.
        r = run(root, "matrix", "flatpak", "--version", "0.5.0")
        try:
            rows = json.loads(r.stdout)
        except json.JSONDecodeError:
            rows = None
        case(
            "the flatpak matrix is the two flatpak rows, as JSON",
            r.returncode == 0 and isinstance(rows, list) and [x["id"] for x in rows] == ["postio", "postio-tui"],
            f"exit {r.returncode}: {r.stdout!r} {r.stderr!r}",
        )
        case(
            "each matrix row carries its resolved asset name",
            bool(rows) and rows[0]["asset"] == "postio-0.5.0-linux-x86_64.flatpak",
            f"got {rows!r}",
        )
        case(
            "each matrix row carries its SBOM's name beside it",
            bool(rows) and rows[0]["sbom"] == "postio-0.5.0-linux-x86_64.flatpak.spdx.json",
            f"got {rows!r}",
        )
        r = run(root, "matrix", "macos-app", "--version", "0.5.0")
        case(
            "the macOS matrix is the one macOS row",
            r.returncode == 0 and [x["asset"] for x in json.loads(r.stdout or "[]")] == ["postio-0.5.0-macos-arm64.zip"],
            r.stdout + r.stderr,
        )
        # A kind with no rows is an empty list, which the workflow tests for
        # before it starts a matrix -- GitHub refuses an empty one.
        r = run(root, "matrix", "nothing-like-this", "--version", "0.5.0")
        case(
            "a kind with no rows is an empty JSON list, not an error",
            r.returncode == 0 and r.stdout.strip() == "[]",
            f"exit {r.returncode}: {r.stdout!r}",
        )

        # Every asset, and each one's SBOM: what `publish` refuses to
        # publish without.
        r = run(root, "assets", "--version", "0.5.0")
        assets = r.stdout.split()
        case(
            "assets lists every package and its SBOM",
            r.returncode == 0
            and "postio-0.5.0-linux-x86_64.flatpak" in assets
            and "postio-tui-0.5.0-linux-x86_64.tar.zst" in assets
            and "postio-0.5.0-macos-arm64.zip" in assets
            and "postio-0.5.0-macos-arm64.zip.spdx.json" in assets
            and len(assets) == 8,
            f"got {assets!r}",
        )

    # ── what the check refuses ────────────────────────────────────────
    refusals = [
        (
            "a flatpak whose manifest does not exist",
            [dict(FLATPAK, manifest="flatpak/missing.json")],
            "flatpak/missing.json",
        ),
        (
            "a flatpak whose app id disagrees with its manifest",
            [dict(FLATPAK, app_id="dev.postio.Other")],
            "dev.postio.Other",
        ),
        (
            "a tarball naming a binary its package does not have",
            [dict(TARBALL, bin="postio-nope")],
            "postio-nope",
        ),
        (
            "a tarball naming a profile the workspace does not define",
            [dict(TARBALL, profile="release-nope")],
            "release-nope",
        ),
        (
            "two rows that would upload the same asset",
            [FLATPAK, dict(TUI_FLATPAK, asset=FLATPAK["asset"])],
            FLATPAK["asset"],
        ),
        (
            "an asset name outside <id>-{version}-<os>-<arch>.<ext>",
            [dict(FLATPAK, asset="Postio-{version}.flatpak")],
            "Postio-{version}.flatpak",
        ),
        (
            "an asset name that does not start with its own id",
            [dict(TUI_FLATPAK, asset="postio-{version}-linux-x86_64.flatpak")],
            "postio-tui",
        ),
        (
            "a kind nothing builds",
            [dict(FLATPAK, kind="snap")],
            "snap",
        ),
    ]
    for label, variants, named in refusals:
        with tempfile.TemporaryDirectory() as directory:
            root = world(Path(directory), variants)
            r = run(root, "check")
            out = r.stdout + r.stderr
            case(f"refuses {label}", r.returncode != 0, f"passed:\n{out}")
            case(f"...and names {named!r}", named in out, out)

    for failure in FAILURES:
        print(f"FAIL  {failure}")
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.")
        return 1
    print("release-variants self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
