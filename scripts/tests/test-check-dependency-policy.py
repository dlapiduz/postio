#!/usr/bin/env python3
"""Prove `check-dependency-policy.py` fails when the policy is violated.

A guard that has never been seen to fail is a guard nobody should trust — and
this one guards against exactly that, so it had better not be an instance of
it. `deny.toml` sat unread for months while three crates drifted (#639).

Each case builds a tiny sandbox workspace, breaks one thing, and asserts the
check notices. The licence case reconstructs #639: a crate that hard-codes a
licence outside the allow-list instead of inheriting the workspace's.

cargo-deny is not in `mise.toml`, so the cases that need it skip when it is
absent rather than failing this test on a machine that made that choice.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CHECK = ROOT / "scripts" / "checks" / "check-dependency-policy.py"

DENY_TOML = """\
[licenses]
allow = ["MIT", "Apache-2.0"]

[bans]
multiple-versions = "allow"

[sources]
unknown-registry = "deny"
"""

WORKSPACE = """\
[workspace]
members = ["crates/leaf"]
resolver = "3"

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT"
publish = false
"""

INHERITING_CRATE = """\
[package]
name = "leaf"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true
"""

# #639's shape: hard-coded, outside the allow-list, and not inherited.
DRIFTED_CRATE = """\
[package]
name = "leaf"
version = "0.1.0"
edition = "2024"
license = "GPL-3.0-or-later"
"""


def sandbox(tmp: Path, crate_manifest: str) -> Path:
    """A minimal workspace with one crate, plus the check itself."""
    root = tmp / "tree"
    (root / "crates" / "leaf" / "src").mkdir(parents=True)
    (root / "scripts" / "checks").mkdir(parents=True)

    (root / "Cargo.toml").write_text(WORKSPACE)
    (root / "deny.toml").write_text(DENY_TOML)
    (root / "crates" / "leaf" / "Cargo.toml").write_text(crate_manifest)
    (root / "crates" / "leaf" / "src" / "lib.rs").write_text("")
    shutil.copy(CHECK, root / "scripts" / "checks" / CHECK.name)
    return root


def run(root: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(root / "scripts" / "checks" / CHECK.name)],
        capture_output=True,
        text=True,
        timeout=600,
    )


def have_cargo_deny() -> bool:
    return shutil.which("cargo-deny") is not None


def case_missing_deny_toml() -> bool:
    """No policy file at all is a failure, not a pass."""
    with tempfile.TemporaryDirectory() as tmp:
        root = sandbox(Path(tmp), INHERITING_CRATE)
        (root / "deny.toml").unlink()
        result = run(root)
        if result.returncode == 0:
            print("  FAIL: a missing deny.toml was accepted", file=sys.stderr)
            return False
        if "deny.toml is missing" not in result.stderr:
            print(f"  FAIL: unhelpful message: {result.stderr!r}", file=sys.stderr)
            return False
    return True


def case_licence_outside_the_allow_list() -> bool:
    """#639 itself: a crate hard-coding a licence the policy does not allow."""
    if not have_cargo_deny():
        print("  SKIP: cargo-deny is not installed")
        return True
    with tempfile.TemporaryDirectory() as tmp:
        root = sandbox(Path(tmp), DRIFTED_CRATE)
        result = run(root)
        if result.returncode == 0:
            print(
                "  FAIL: a GPL-3.0-or-later crate passed a policy allowing "
                "only MIT and Apache-2.0",
                file=sys.stderr,
            )
            return False
    return True


def case_clean_tree_passes() -> bool:
    """The inheriting crate is what every other Postio crate looks like."""
    if not have_cargo_deny():
        print("  SKIP: cargo-deny is not installed")
        return True
    with tempfile.TemporaryDirectory() as tmp:
        root = sandbox(Path(tmp), INHERITING_CRATE)
        result = run(root)
        if result.returncode != 0:
            print(
                f"  FAIL: a clean tree was rejected:\n{result.stderr}",
                file=sys.stderr,
            )
            return False
    return True


def main() -> int:
    cases = [
        ("a missing deny.toml fails", case_missing_deny_toml),
        ("a licence outside the allow-list fails", case_licence_outside_the_allow_list),
        ("an inheriting crate passes", case_clean_tree_passes),
    ]
    failed = 0
    for name, case in cases:
        print(f"case: {name}")
        if not case():
            failed += 1

    if failed:
        print(f"\n{failed} case(s) failed.", file=sys.stderr)
        return 1
    print("test-check-dependency-policy: all cases passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
