#!/usr/bin/env python3
"""Self-test for scripts/install-local.sh's icon cache refresh.

#427: after `scripts/install-local.sh`, the app grid kept showing the
default icon instead of Postio's. `gtk-update-icon-cache -qt` passes
`--ignore-theme-index`, not `--force` -- so a cache already sitting in
`~/.local/share/icons/hicolor` (from an earlier run, or another app
installed into the same user theme) is left exactly as it was, serving up
whatever it indexed before Postio's icon ever existed there. `--force`
makes the refresh unconditional instead of a no-op on an already-present
cache.

`cargo` and `gtk-update-icon-cache` are stubbed on PATH: the former drops a
placeholder file where the real build would have (this only exercises the
desktop-entry/icon install, not a real Postio binary), and the latter
records every call it is given so this can assert `--force` was actually
passed -- no compiler, no display, no network.

Usage: scripts/tests/test-install-local-icon.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "install-local.sh"

FAILURES: list[str] = []

CARGO_STUB = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/cargo-calls"
if [ "$1" = "build" ]; then
    mkdir -p "$CARGO_TARGET_DIR/release"
    printf '#!/bin/sh\\n' > "$CARGO_TARGET_DIR/release/postio"
    chmod +x "$CARGO_TARGET_DIR/release/postio"
fi
exit 0
"""

ICON_CACHE_STUB = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/icon-cache-calls"
exit 0
"""

NOOP_STUB = """#!/usr/bin/env bash
exit 0
"""


def run(*args: str) -> tuple[subprocess.CompletedProcess, Path]:
    tmp = tempfile.TemporaryDirectory()
    stub_dir = Path(tmp.name) / "stub"
    bin_dir = stub_dir / "bin"
    bin_dir.mkdir(parents=True)
    (bin_dir / "cargo").write_text(CARGO_STUB, encoding="utf-8")
    (bin_dir / "cargo").chmod(0o755)
    (bin_dir / "gtk-update-icon-cache").write_text(ICON_CACHE_STUB, encoding="utf-8")
    (bin_dir / "gtk-update-icon-cache").chmod(0o755)
    (bin_dir / "update-desktop-database").write_text(NOOP_STUB, encoding="utf-8")
    (bin_dir / "update-desktop-database").chmod(0o755)
    (stub_dir / "cargo-calls").write_text("", encoding="utf-8")
    (stub_dir / "icon-cache-calls").write_text("", encoding="utf-8")

    data_home = Path(tmp.name) / "data-home"
    prefix = Path(tmp.name) / "prefix"
    target = Path(tmp.name) / "target"
    data_home.mkdir()
    prefix.mkdir()
    target.mkdir()

    env = dict(os.environ)
    env["PATH"] = f"{bin_dir}:{env['PATH']}"
    env["STUB_DIR"] = str(stub_dir)
    env["XDG_DATA_HOME"] = str(data_home)
    env["PREFIX"] = str(prefix)
    env["CARGO_TARGET_DIR"] = str(target)

    proc = subprocess.run(
        ["bash", str(SCRIPT), *args],
        capture_output=True,
        text=True,
        env=env,
        stdin=subprocess.DEVNULL,
        timeout=60,
    )
    proc._tmp = tmp  # keep the tempdir alive until the caller is done with it
    proc._data_home = data_home
    proc._icon_cache_calls = stub_dir / "icon-cache-calls"
    return proc, stub_dir / "icon-cache-calls"


def calls(calls_path: Path) -> list[str]:
    if not calls_path.exists():
        return []
    return [line for line in calls_path.read_text(encoding="utf-8").splitlines() if line]


def case_install_forces_the_icon_cache_to_rebuild() -> None:
    label = "install passes --force, not just --ignore-theme-index"
    proc, calls_path = run()
    if proc.returncode != 0:
        FAILURES.append(f"{label}: expected exit 0, got {proc.returncode}\n{proc.stderr}")
        return
    logged = calls(calls_path)
    if len(logged) != 1:
        FAILURES.append(f"{label}: expected exactly one gtk-update-icon-cache call, got {logged}")
        return
    args = logged[0].split()
    if not any(
        arg == "--force" or (arg.startswith("-") and not arg.startswith("--") and "f" in arg[1:])
        for arg in args
    ):
        FAILURES.append(
            f"{label}: no --force/-f in the call, so an existing cache is left stale: {logged[0]!r}"
        )


def case_installed_files_land_where_the_desktop_entry_names_them() -> None:
    label = "the installed icon files match the desktop entry's Icon="
    proc, _ = run()
    if proc.returncode != 0:
        FAILURES.append(f"{label}: expected exit 0, got {proc.returncode}\n{proc.stderr}")
        return
    data_home = proc._data_home
    desktop = data_home / "applications" / "dev.postio.Postio.desktop"
    png = data_home / "icons" / "hicolor" / "128x128" / "apps" / "dev.postio.Postio.png"
    svg = data_home / "icons" / "hicolor" / "scalable" / "apps" / "dev.postio.Postio.svg"
    if not desktop.exists():
        FAILURES.append(f"{label}: no desktop entry installed at {desktop}")
        return
    icon_name = None
    for line in desktop.read_text(encoding="utf-8").splitlines():
        if line.startswith("Icon="):
            icon_name = line.removeprefix("Icon=")
            break
    if icon_name != "dev.postio.Postio":
        FAILURES.append(f"{label}: Icon= was {icon_name!r}, not the installed basename")
    if not png.exists():
        FAILURES.append(f"{label}: {icon_name}.png missing at the path the theme expects: {png}")
    if not svg.exists():
        FAILURES.append(f"{label}: {icon_name}.svg missing at the path the theme expects: {svg}")


def case_uninstall_also_forces_a_cache_rebuild() -> None:
    label = "--uninstall forces a cache rebuild too, so a removed icon does not linger"
    run()  # install first, so there is something to uninstall
    proc, calls_path = run("--uninstall")
    if proc.returncode != 0:
        FAILURES.append(f"{label}: expected exit 0, got {proc.returncode}\n{proc.stderr}")
        return
    logged = calls(calls_path)
    if len(logged) != 1:
        FAILURES.append(f"{label}: expected exactly one gtk-update-icon-cache call, got {logged}")
        return
    args = logged[0].split()
    if not any(
        arg == "--force" or (arg.startswith("-") and not arg.startswith("--") and "f" in arg[1:])
        for arg in args
    ):
        FAILURES.append(f"{label}: no --force/-f in the uninstall's call: {logged[0]!r}")


def case_finishing_advice_mentions_a_session_restart() -> None:
    label = "a successful install says what to do if the icon still looks wrong"
    proc, _ = run()
    if proc.returncode != 0:
        FAILURES.append(f"{label}: expected exit 0, got {proc.returncode}\n{proc.stderr}")
        return
    combined = (proc.stdout + proc.stderr).lower()
    if "log out" not in combined and "restart" not in combined and "log back in" not in combined:
        FAILURES.append(
            f"{label}: install finished silent on the session-restart case: {proc.stdout!r}"
        )


def main() -> int:
    if not SCRIPT.exists():
        print(f"missing: {SCRIPT}", file=sys.stderr)
        return 1
    case_install_forces_the_icon_cache_to_rebuild()
    case_installed_files_land_where_the_desktop_entry_names_them()
    case_uninstall_also_forces_a_cache_rebuild()
    case_finishing_advice_mentions_a_session_restart()

    if FAILURES:
        for failure in FAILURES:
            print(f"FAIL: {failure}", file=sys.stderr)
        print(f"{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("install-local icon self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
