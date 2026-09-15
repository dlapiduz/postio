#!/usr/bin/env python3
"""Self-test for scripts/checks/check-version-agreement.py.

v0.3.0 was tagged on 2026-09-11. release.yml bumps the version *after* the
release gate passes, and the gate hung, so the bump never landed: `main`
kept saying 0.2.0, the AppStream changelog had no 0.3.0 entry, and nothing
noticed for three days -- nothing compared the three places a version is
written. The check does. This runs it against fixture trees so it is
proved to fail in each direction, not just to pass on today's tree.

Usage: scripts/tests/test-check-version-agreement.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

CHECK = Path(__file__).resolve().parent.parent / "checks" / "check-version-agreement.py"
METAINFO = "crates/postio-gtk/data/dev.postio.Postio.metainfo.xml"
FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    print(f"{'ok   ' if condition else 'FAIL '} {name}")
    if not condition:
        FAILURES.append(f"{name}: {detail}")


def tree(root: Path, *, workspace: str, pin: str, newest: str) -> None:
    (root / "Cargo.toml").write_text(
        f'[workspace]\nmembers = ["crates/*"]\n\n[workspace.package]\nversion = "{workspace}"\n'
        'rust-version = "1.98"\n',
        encoding="utf-8",
    )
    crate = root / "crates" / "postio-x"
    crate.mkdir(parents=True, exist_ok=True)
    crate.joinpath("Cargo.toml").write_text(
        '[package]\nname = "postio-x"\nversion.workspace = true\n\n[dependencies]\n'
        f'postio-model = {{ version = "{pin}", path = "../postio-model" }}\n'
        'some-unrelated-crate = { version = "0.2.0" }\n',
        encoding="utf-8",
    )
    metainfo = root / METAINFO
    metainfo.parent.mkdir(parents=True, exist_ok=True)
    metainfo.write_text(
        '<?xml version="1.0" encoding="UTF-8"?>\n<component type="desktop-application">\n'
        "  <releases>\n"
        f'    <release version="{newest}" date="2026-09-11" type="development">\n'
        "      <description><p>x</p></description>\n    </release>\n"
        '    <release version="0.1.0" date="2026-08-23">\n'
        "      <description><p>y</p></description>\n    </release>\n"
        "  </releases>\n</component>\n",
        encoding="utf-8",
    )


def run(root: Path) -> subprocess.CompletedProcess[str]:
    return patience.run(
        ["python3", str(CHECK), "--root", str(root)],
        capture_output=True, text=True, timeout=30,
    )


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        root = Path(raw)

        tree(root, workspace="0.3.0", pin="0.3.0", newest="0.3.0")
        r = run(root)
        case("agreeing versions pass", r.returncode == 0, r.stdout + r.stderr)

        tree(root, workspace="0.3.0", pin="0.2.0", newest="0.3.0")
        r = run(root)
        case("a lagging internal pin fails", r.returncode != 0, "passed with a pin at the old version")
        case("...and the pin's manifest is named", "crates/postio-x/Cargo.toml" in r.stdout + r.stderr, r.stdout + r.stderr)

        tree(root, workspace="0.3.0", pin="0.3.0", newest="0.2.0")
        r = run(root)
        case("a changelog whose newest entry lags fails", r.returncode != 0, "passed with a stale newest release entry")
        case("...and the metainfo is named", "metainfo.xml" in r.stdout + r.stderr, r.stdout + r.stderr)

        tree(root, workspace="0.2.0", pin="0.2.0", newest="0.3.0")
        r = run(root)
        case("a workspace behind its own changelog fails", r.returncode != 0, "passed with Cargo.toml behind the metainfo")

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("check-version-agreement self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
