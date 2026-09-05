#!/usr/bin/env python3
"""Self-test for #1101: the linker and C compiler cargo is told about are
bare program names, and `scripts/install-shims.sh` is what puts them on PATH.

`.cargo/config.toml` used to say `linker = "scripts/linker.sh"`, which cargo
resolves against the config's own directory -- so every worktree passed a
different `-C linker=/home/.../postio-worktrees/issue-N/scripts/linker.sh`.
sccache hashes that argument, and cargo folds it into its fingerprints, so a
registry crate compiled in one worktree was a miss in every other (2 hits,
178 misses, measured), and a copied `target/` rebuilt everything. `CC` had
the same shape for the C build scripts.

A bare name is the same string in every worktree. The price is that the
name has to resolve, which is this installer's job: it copies the two
scripts into `$CARGO_HOME/bin`, which rustup already puts on PATH.

Usage: scripts/tests/test-install-shims.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
ROOT = SCRIPTS.parent
INSTALL = SCRIPTS / "install-shims.sh"
SHIMS = {
    "postio-linker": SCRIPTS / "linker.sh",
    "postio-cc": SCRIPTS / "cc-wrapper.sh",
}

FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def install(cargo_home: Path) -> subprocess.CompletedProcess[str]:
    environment = dict(os.environ)
    environment["CARGO_HOME"] = str(cargo_home)
    return subprocess.run(
        ["bash", str(INSTALL)],
        env=environment,
        capture_output=True,
        text=True,
        timeout=60,
    )


def installer() -> None:
    with tempfile.TemporaryDirectory() as raw:
        cargo_home = Path(raw) / "cargo"
        result = install(cargo_home)
        case(
            "the installer succeeds into an empty CARGO_HOME",
            result.returncode == 0,
            f"exit {result.returncode}\n{result.stdout}\n{result.stderr}",
        )
        for name, source in SHIMS.items():
            shim = cargo_home / "bin" / name
            case(
                f"{name} is installed",
                shim.is_file(),
                f"{shim} is missing",
            )
            if not shim.is_file():
                continue
            case(
                f"{name} is executable",
                os.access(shim, os.X_OK),
                f"{shim} has no executable bit",
            )
            case(
                f"{name} is a byte-for-byte copy of {source.name}",
                shim.read_bytes() == source.read_bytes(),
                "the installed shim differs from its source in scripts/",
            )

        # A stale copy is replaced: the shim is the source of nothing, the
        # script in scripts/ is.
        stale = cargo_home / "bin" / "postio-linker"
        if not stale.parent.is_dir():
            return
        stale.write_text("#!/bin/sh\nexit 97\n", encoding="utf-8")
        result = install(cargo_home)
        case(
            "a stale shim is overwritten",
            result.returncode == 0
            and stale.read_bytes() == SHIMS["postio-linker"].read_bytes(),
            f"exit {result.returncode}; content {stale.read_text()[:40]!r}",
        )

        # Idempotent: nothing to do is still success, and it says so briefly
        # rather than reinstalling -- this runs before every gate chain.
        before = {name: (cargo_home / "bin" / name).stat().st_mtime_ns for name in SHIMS}
        result = install(cargo_home)
        after = {name: (cargo_home / "bin" / name).stat().st_mtime_ns for name in SHIMS}
        case(
            "an up-to-date shim is left alone",
            result.returncode == 0 and before == after,
            f"exit {result.returncode}; mtimes changed: {before != after}",
        )


def config() -> None:
    """The config names programs, not paths -- that is the whole fix."""
    text = (ROOT / ".cargo" / "config.toml").read_text(encoding="utf-8")
    linker = re.search(r'^linker\s*=\s*"([^"]*)"', text, re.MULTILINE)
    case(
        "config.toml names a linker",
        linker is not None,
        "no `linker = ...` line in .cargo/config.toml",
    )
    if linker:
        case(
            "the linker is a bare name, so it is the same string in every worktree",
            linker.group(1) == "postio-linker",
            f"linker = {linker.group(1)!r}; a path is resolved per worktree and "
            "defeats sccache and every copied target/",
        )
    cc = re.search(r'^CC\s*=\s*(.+)$', text, re.MULTILINE)
    case(
        "config.toml sets CC",
        cc is not None,
        "no `CC = ...` line under [env]",
    )
    if cc:
        case(
            "CC is a bare name too -- build scripts rerun whenever its value changes",
            cc.group(1).strip() == '"postio-cc"',
            f"CC = {cc.group(1).strip()}",
        )


def main() -> int:
    installer()
    config()
    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("install-shims self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
