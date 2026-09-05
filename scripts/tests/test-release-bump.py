#!/usr/bin/env python3
"""Self-test for scripts/release-bump.py.

#886: cutting v0.2.0 by hand meant bumping the workspace version, six
internal path-dependency pins that name it explicitly, and the AppStream
changelog, one at a time, hoping none was missed. This is the mechanical
half of automating that -- the part with no judgment calls, which is
exactly the part that should never depend on a person remembering all of
it correctly under time pressure.

Throwaway trees in a temp dir, real relative paths (`Cargo.toml`,
`crates/*/Cargo.toml`, the metainfo file) with synthetic content, so the
real script runs against them unmodified. The real repository is never
touched and nothing here reaches the network.

Usage: scripts/tests/test-release-bump.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "release-bump.py"

METAINFO_PATH = "crates/postio-gtk/data/dev.postio.Postio.metainfo.xml"

CARGO_TOML = """\
[workspace]
resolver = "3"
members = ["crates/postio-model", "crates/postio-storage", "crates/postio-gtk"]

[workspace.package]
version = "0.2.0"
edition = "2024"
rust-version = "1.98"
"""

METAINFO = """\
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>dev.postio.Postio</id>
  <releases>
    <release version="0.2.0" date="2026-09-02">
      <description>
        <p>Second release.</p>
      </description>
    </release>
    <release version="0.1.0" date="2026-08-23">
      <description>
        <p>Initial development release.</p>
      </description>
    </release>
  </releases>
</component>
"""

FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def world(base: Path) -> Path:
    """A tree with the real relative paths the script looks for."""
    root = base / "repo"
    (root / "crates" / "postio-model").mkdir(parents=True)
    (root / "crates" / "postio-storage").mkdir(parents=True)
    (root / "crates" / "postio-gtk" / "data").mkdir(parents=True)

    (root / "Cargo.toml").write_text(CARGO_TOML, encoding="utf-8")
    (root / METAINFO_PATH).write_text(METAINFO, encoding="utf-8")

    # A pin that must be bumped: names the current workspace version and a
    # sibling path.
    (root / "crates" / "postio-storage" / "Cargo.toml").write_text(
        '[package]\nname = "postio-storage"\nversion.workspace = true\n\n'
        "[dependencies]\n"
        'postio-model = { version = "0.2.0", path = "../postio-model" }\n',
        encoding="utf-8",
    )
    # A pin that must NOT be bumped: it is not a sibling path dependency, so
    # matching it would be the same bug as touching an unrelated third-party
    # crate that happens to share the old version number.
    (root / "crates" / "postio-gtk" / "Cargo.toml").write_text(
        '[package]\nname = "postio-gtk"\nversion.workspace = true\n\n'
        "[dependencies]\n"
        'some-unrelated-crate = { version = "0.2.0" }\n',
        encoding="utf-8",
    )
    return root


def run(root: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        cwd=root,
        capture_output=True,
        text=True,
        timeout=30,
    )


def main() -> int:
    # ── the happy path ────────────────────────────────────────────────
    with tempfile.TemporaryDirectory() as directory:
        root = world(Path(directory))
        notes = root / "notes.txt"
        notes.write_text("- did a thing\n- did another thing\n", encoding="utf-8")

        result = run(root, "0.3.0", "--notes-file", str(notes), "--date", "2026-09-10")
        out = result.stdout + result.stderr

        case(
            "exits 0 on a clean bump",
            result.returncode == 0,
            f"exit {result.returncode}; output:\n{out}",
        )

        cargo_toml = (root / "Cargo.toml").read_text(encoding="utf-8")
        case(
            "the workspace version is bumped",
            'version = "0.3.0"' in cargo_toml,
            f"Cargo.toml did not gain the new version:\n{cargo_toml}",
        )
        case(
            "rust-version is untouched -- a different key, not a second version",
            'rust-version = "1.98"' in cargo_toml,
            f"rust-version changed unexpectedly:\n{cargo_toml}",
        )
        case(
            "the old workspace version line is gone",
            'version = "0.2.0"' not in cargo_toml,
            f"the old version line is still there:\n{cargo_toml}",
        )

        storage_toml = (
            root / "crates" / "postio-storage" / "Cargo.toml"
        ).read_text(encoding="utf-8")
        case(
            "an internal sibling pin is bumped",
            'postio-model = { version = "0.3.0", path = "../postio-model" }'
            in storage_toml,
            f"the sibling pin was not bumped:\n{storage_toml}",
        )

        gtk_toml = (root / "crates" / "postio-gtk" / "Cargo.toml").read_text(
            encoding="utf-8"
        )
        case(
            "an unrelated dependency sharing the old version number is untouched",
            'some-unrelated-crate = { version = "0.2.0" }' in gtk_toml,
            f"a non-sibling dependency was touched:\n{gtk_toml}",
        )

        metainfo = (root / METAINFO_PATH).read_text(encoding="utf-8")
        case(
            "a new release entry is added with the given version and date",
            '<release version="0.3.0" date="2026-09-10">' in metainfo,
            f"no new release entry found:\n{metainfo}",
        )
        case(
            "the notes reach the changelog entry",
            "did a thing" in metainfo and "did another thing" in metainfo,
            f"notes text is missing from the changelog:\n{metainfo}",
        )
        case(
            "the new entry comes before the previous newest one -- newest first",
            metainfo.index('version="0.3.0"') < metainfo.index('version="0.2.0"'),
            f"release entries are out of order:\n{metainfo}",
        )
        case(
            "older entries are preserved untouched",
            '<release version="0.1.0" date="2026-08-23">' in metainfo
            and "Initial development release." in metainfo,
            f"an older release entry was lost or altered:\n{metainfo}",
        )

    # ── refusals ──────────────────────────────────────────────────────
    with tempfile.TemporaryDirectory() as directory:
        root = world(Path(directory))
        notes = root / "notes.txt"
        notes.write_text("- x\n", encoding="utf-8")

        result = run(root, "0.2.0", "--notes-file", str(notes))
        case(
            "bumping to the version already in place is refused",
            result.returncode != 0,
            f"expected a non-zero exit, got {result.returncode}",
        )

        for bad in ("abc", "1.2", "v0.3.0", ""):
            result = run(root, bad, "--notes-file", str(notes))
            case(
                f"a non-semver version {bad!r} is refused",
                result.returncode != 0,
                f"expected a non-zero exit for {bad!r}, got {result.returncode}",
            )

        result = run(root, "0.3.0", "--notes-file", str(root / "missing.txt"))
        case(
            "a missing notes file is refused",
            result.returncode != 0,
            f"expected a non-zero exit, got {result.returncode}",
        )

        # And nothing was left half-changed by the refusals above.
        cargo_toml = (root / "Cargo.toml").read_text(encoding="utf-8")
        case(
            "a refused run leaves the workspace version untouched",
            'version = "0.2.0"' in cargo_toml,
            f"Cargo.toml changed despite every attempt being refused:\n{cargo_toml}",
        )

    for failure in FAILURES:
        print(f"FAIL  {failure}")
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed.")
        return 1
    print("release-bump self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
