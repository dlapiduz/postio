#!/usr/bin/env python3
"""Self-test for scripts/install-local.sh's build-dependency check.

Born of #646, when the store was SQLCipher and a release build compiled
OpenSSL from source: six missing perl modules were six failed builds,
several minutes each. The engine is pure Rust now and the perl probe is
gone with OpenSSL, but the shape the incident taught survives in the
pkg-config half: report *every* missing dependency at once, before `cargo`
is started, and stay out of the way on a machine that has everything -- a
probe that blocks a working build would be worse than the problem it
solves.

`pkg-config` and `cargo` are stubbed on PATH: the pkg-config stub fails
for exactly the names a case names missing, and the cargo stub records
whether it was ever called. No compiler, no display, no network, and
nothing installed anywhere real.

Usage: scripts/tests/test-install-local-build-deps.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# The shared dial (#1249). `scripts/lib`, not beside this file, because CI
# runs every `scripts/tests/*.py` it finds as a self-test.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above
import prereq  # noqa: E402

# Linux-only, declared rather than skipped (#1151): this stands down
# with a sentence and exit 77, which `run-self-tests.sh` counts and
# names, so the macOS run does not quietly claim to have covered it.
prereq.only_on("linux", reason=(
    "install-local.sh probes for gtk4/libadwaita/webkitgtk through pkg-config, which is the build this platform deliberately cannot do"
))

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "install-local.sh"

FAILURES: list[str] = []

# Records that it ran, and fakes just enough of a build for the install steps
# that follow it.
CARGO_STUB = """#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$STUB_DIR/cargo-calls"
if [ "$1" = "build" ]; then
    mkdir -p "$CARGO_TARGET_DIR/release"
    printf '#!/bin/sh\\n' > "$CARGO_TARGET_DIR/release/postio"
    chmod +x "$CARGO_TARGET_DIR/release/postio"
fi
exit 0
"""

# Fails for the modules STUB_PERL_MISSING names, the way a perl without them
# does: a `Can't locate` on stderr and a non-zero status.
PKG_CONFIG_STUB = """#!/usr/bin/env bash
if [ "$1" = "--exists" ]; then
    for missing in $STUB_PKGCONFIG_MISSING; do
        if [ "$2" = "$missing" ]; then
            exit 1
        fi
    done
fi
exit 0
"""

NOOP_STUB = """#!/usr/bin/env bash
exit 0
"""

# Everything install-local.sh and the stubs above reach for outside the
# shell's builtins. Symlinked rather than inherited from PATH, so a case can
# ask what happens when `perl` is not on the machine at all.
# `cmp` and `cp` are scripts/install-shims.sh's, which install-local.sh now
# runs first so a fresh clone's build can find `postio-linker`.
BORROWED = ["bash", "sh", "dirname", "install", "rm", "mkdir", "chmod", "env", "printf", "cat", "cmp", "cp"]

# Resolved before any case narrows PATH down to the stubs.
BASH = shutil.which("bash") or "/bin/bash"


def _borrow(bin_dir: Path) -> None:
    for tool in BORROWED:
        found = None
        for directory in ("/usr/bin", "/bin", "/usr/local/bin"):
            candidate = Path(directory) / tool
            if candidate.exists():
                found = candidate
                break
        if found is not None:
            (bin_dir / tool).symlink_to(found)


def run(
    *args: str,
    pkgconfig_missing: str = "",
    with_pkg_config: bool = True,
) -> subprocess.CompletedProcess:
    """Run install-local.sh against stubs, and hand back the finished process.

    The temporary directory is kept alive on the returned object, because a
    case reads files out of it after this returns.
    """
    tmp = tempfile.TemporaryDirectory()
    stub_dir = Path(tmp.name) / "stub"
    bin_dir = stub_dir / "bin"
    bin_dir.mkdir(parents=True)
    _borrow(bin_dir)

    for name, body in (
        ("cargo", CARGO_STUB),
        ("gtk-update-icon-cache", NOOP_STUB),
        ("update-desktop-database", NOOP_STUB),
    ):
        (bin_dir / name).write_text(body, encoding="utf-8")
        (bin_dir / name).chmod(0o755)
    if with_pkg_config:
        (bin_dir / "pkg-config").write_text(PKG_CONFIG_STUB, encoding="utf-8")
        (bin_dir / "pkg-config").chmod(0o755)
    (stub_dir / "cargo-calls").write_text("", encoding="utf-8")

    data_home = Path(tmp.name) / "data-home"
    prefix = Path(tmp.name) / "prefix"
    target = Path(tmp.name) / "target"
    for directory in (data_home, prefix, target):
        directory.mkdir()

    # PATH is *only* the stub directory: a case that says "no perl here" has
    # to mean it, and the machine running this test has a real perl.
    env = dict(os.environ)
    env["PATH"] = str(bin_dir)
    env["STUB_DIR"] = str(stub_dir)
    env["STUB_PKGCONFIG_MISSING"] = pkgconfig_missing
    env["XDG_DATA_HOME"] = str(data_home)
    env["PREFIX"] = str(prefix)
    env["CARGO_TARGET_DIR"] = str(target)
    env.pop("POSTIO_SKIP_DEP_CHECK", None)

    proc = patience.run(
        [BASH, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        env=env,
        stdin=subprocess.DEVNULL,
        timeout=60,
    )
    proc._tmp = tmp  # keep the tempdir alive until the caller is done with it
    proc._cargo_calls = stub_dir / "cargo-calls"
    return proc


def cargo_ran(proc: subprocess.CompletedProcess) -> bool:
    return bool(proc._cargo_calls.read_text(encoding="utf-8").strip())


def case_a_missing_library_is_reported_too() -> None:
    label = "a missing pkg-config library stops the build and is named"
    proc = run(pkgconfig_missing="webkitgtk-6.0")
    if proc.returncode == 0:
        FAILURES.append(f"{label}: expected a non-zero exit, got 0\n{proc.stdout}")
        return
    if cargo_ran(proc):
        FAILURES.append(f"{label}: cargo was started with a library missing")
    combined = proc.stdout + proc.stderr
    if "webkitgtk-6.0" not in combined:
        FAILURES.append(f"{label}: the missing library is unnamed: {combined!r}")
    if "README" not in combined:
        FAILURES.append(f"{label}: nothing points at the documented list: {combined!r}")


def case_a_complete_machine_builds_exactly_as_before() -> None:
    label = "a machine with every dependency present builds, and says nothing new"
    proc = run()
    if proc.returncode != 0:
        FAILURES.append(f"{label}: expected exit 0, got {proc.returncode}\n{proc.stderr}")
        return
    if not cargo_ran(proc):
        FAILURES.append(f"{label}: the build never started")
    if proc.stderr.strip():
        FAILURES.append(f"{label}: a healthy machine got output on stderr: {proc.stderr!r}")


def case_uninstall_never_asks_for_a_compiler_s_dependencies() -> None:
    label = "--uninstall works on a machine that could not build"
    proc = run("--uninstall", pkgconfig_missing="gtk4")
    if proc.returncode != 0:
        FAILURES.append(
            f"{label}: removing an installed copy needs no build dependencies, "
            f"but exited {proc.returncode}\n{proc.stderr}"
        )


def case_the_check_can_be_stepped_over() -> None:
    label = "POSTIO_SKIP_DEP_CHECK gets past a probe that is wrong about this machine"
    tmp = tempfile.TemporaryDirectory()
    stub_dir = Path(tmp.name) / "stub"
    bin_dir = stub_dir / "bin"
    bin_dir.mkdir(parents=True)
    _borrow(bin_dir)
    for name, body in (
        ("cargo", CARGO_STUB),
        ("gtk-update-icon-cache", NOOP_STUB),
        ("update-desktop-database", NOOP_STUB),
        ("pkg-config", PKG_CONFIG_STUB),
    ):
        (bin_dir / name).write_text(body, encoding="utf-8")
        (bin_dir / name).chmod(0o755)
    (stub_dir / "cargo-calls").write_text("", encoding="utf-8")
    for directory in ("data-home", "prefix", "target"):
        (Path(tmp.name) / directory).mkdir()

    env = dict(os.environ)
    env["PATH"] = str(bin_dir)
    env["STUB_DIR"] = str(stub_dir)
    env["STUB_PKGCONFIG_MISSING"] = "gtk4"
    env["XDG_DATA_HOME"] = str(Path(tmp.name) / "data-home")
    env["PREFIX"] = str(Path(tmp.name) / "prefix")
    env["CARGO_TARGET_DIR"] = str(Path(tmp.name) / "target")
    env["POSTIO_SKIP_DEP_CHECK"] = "1"

    proc = patience.run(
        [BASH, str(SCRIPT)],
        capture_output=True,
        text=True,
        env=env,
        stdin=subprocess.DEVNULL,
        timeout=60,
    )
    if proc.returncode != 0:
        FAILURES.append(f"{label}: expected exit 0, got {proc.returncode}\n{proc.stderr}")
        return
    if not (stub_dir / "cargo-calls").read_text(encoding="utf-8").strip():
        FAILURES.append(f"{label}: the build was skipped anyway")


def main() -> int:
    if not SCRIPT.exists():
        print(f"missing: {SCRIPT}", file=sys.stderr)
        return 1
    case_a_missing_library_is_reported_too()
    case_a_complete_machine_builds_exactly_as_before()
    case_uninstall_never_asks_for_a_compiler_s_dependencies()
    case_the_check_can_be_stepped_over()

    if FAILURES:
        for failure in FAILURES:
            print(f"FAIL: {failure}", file=sys.stderr)
        print(f"{len(FAILURES)} case(s) failed.", file=sys.stderr)
        return 1
    print("install-local build-dependency self-test passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
