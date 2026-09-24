#!/usr/bin/env python3
"""Self-test for scripts/measure-package-size.sh (specs/005-tui-frontend SC-004).

The terminal package must be under half the desktop one, form for form: the
standalone binaries with their shared libraries against the desktop binary
with its libraries, and each Flatpak with its runtime. The release workflow
runs the script after both are built and fails when the terminal is not
under half. So what must not drift is:

  * the comparison itself: under half passes, half or more fails, and the
    line it prints names both sizes;
  * a Flatpak is its installed app *and* the runtime it names, because the
    terminal's lighter runtime is most of the difference, and measuring the
    app alone would compare the wrong things;
  * a mistake in how it was called is exit 2, never a pass.

Usage: scripts/tests/test-measure-package-size.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
SCRIPT = HERE / "measure-package-size.sh"

failures: list[str] = []


def case(name: str, ok: bool, detail: str = "") -> None:
    if ok:
        print(f"ok  {name}")
    else:
        failures.append(name)
        print(f"FAIL {name}\n{detail}")


def run(*args: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", str(SCRIPT), *args],
        capture_output=True,
        text=True,
        env=env,
        check=False,
    )


def sized(path: Path, size: int) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b"\0" * size)
    return path


with tempfile.TemporaryDirectory() as scratch:
    root = Path(scratch)

    # Plain files are not ELF, so they have no libraries: the sizes are the
    # files' own.
    tui = sized(root / "bin" / "postio-tui", 2000)
    desktop = sized(root / "bin" / "postio", 5000)
    under = run("binaries", str(tui), "--", str(desktop))
    case(
        "binaries under half pass",
        under.returncode == 0 and "2000" in under.stdout and "5000" in under.stdout,
        under.stdout + under.stderr,
    )

    heavy = sized(root / "bin" / "postio-heavy", 2500)
    over = run("binaries", str(heavy), "--", str(desktop))
    case("binaries at half or more fail", over.returncode == 1, over.stdout + over.stderr)

    # A stub `flatpak` answering `info --show-location` and `--show-runtime`
    # from a table, over directories of known size.
    for directory, size in [
        ("apps/tui", 100),
        ("apps/desktop", 300),
        ("runtimes/freedesktop", 1000),
        ("runtimes/gnome", 4000),
    ]:
        sized(root / directory / "files" / "blob", size)
    stub = root / "stub"
    stub.mkdir()
    (stub / "flatpak").write_text(
        f"""#!/usr/bin/env bash
case "$*" in
  "info --show-runtime dev.postio.PostioTui") echo org.freedesktop.Platform/x86_64/25.08 ;;
  "info --show-runtime dev.postio.Postio") echo org.gnome.Platform/x86_64/50 ;;
  "info --show-location dev.postio.PostioTui") echo {root}/apps/tui ;;
  "info --show-location dev.postio.Postio") echo {root}/apps/desktop ;;
  "info --show-location org.freedesktop.Platform/x86_64/25.08") echo {root}/runtimes/freedesktop ;;
  "info --show-location org.gnome.Platform/x86_64/50") echo {root}/runtimes/gnome ;;
  *) echo "unexpected: $*" >&2; exit 1 ;;
esac
"""
    )
    (stub / "flatpak").chmod(0o755)
    env = dict(os.environ, PATH=f"{stub}:{os.environ['PATH']}")
    flat = run("flatpak", "dev.postio.PostioTui", "dev.postio.Postio", env=env)
    # 1100 against 4300: the runtimes are counted, or it would be 100 against
    # 300 and still pass for the wrong reason -- so the sizes are asserted.
    case(
        "a flatpak is measured with its runtime",
        flat.returncode == 0 and "1100" in flat.stdout and "4300" in flat.stdout,
        flat.stdout + flat.stderr,
    )

    missing = run("flatpak", "dev.postio.Nothing", "dev.postio.Postio", env=env)
    case("an app flatpak cannot find is an error", missing.returncode == 2, missing.stdout + missing.stderr)

    case("no arguments is a usage error", run().returncode == 2)
    case("binaries without a separator is a usage error", run("binaries", str(tui)).returncode == 2)

if failures:
    print(f"{len(failures)} case(s) failed: {', '.join(failures)}")
    sys.exit(1)
