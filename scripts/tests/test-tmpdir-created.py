#!/usr/bin/env python3
"""Self-test: the scripts that front cargo create the TMPDIR it points at.

`.cargo/config.toml` sets ``TMPDIR = { value = "target/tmp", relative = true }``
to keep rustc's and the linker's scratch off the tmpfs, and nothing creates
that directory. ``tempfile`` opens a file inside ``$TMPDIR`` and reports what
the OS said, so in a fresh clone or worktree the first temp file fails with
``NotFound`` -- naming a ``.tmpXXXXXX`` nobody wrote, under a ``target/`` that
plainly exists, three directories from the config that pointed there. It reads
as a bug in whatever was just run. Issue #613.

Two scripts front cargo, and both have to do it:

  * ``headless-runner.sh`` is cargo's ``runner``, so it fronts every binary
    cargo executes for this target -- tests, benches, examples, ``cargo run``.
    It must create the directory *before* its fail-open bail-outs, or a
    contributor with no compositor gets the original failure back.
  * ``rustc-wrapper.sh`` is ``RUSTC_WRAPPER``, so it runs for every compile on
    every platform, including the ones with no ``runner``. It must create
    cargo's directory *before* the sccache branch replaces ``TMPDIR`` with the
    daemon's, or what gets made is the wrong one.

Both must stay fail-open: a directory that cannot be created costs the pinning,
never the build.

Usage: scripts/tests/test-tmpdir-created.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent
RUNNER = SCRIPTS / "headless-runner.sh"
WRAPPER = SCRIPTS / "rustc-wrapper.sh"

FAILURES: list[str] = []


def check(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok: {name}")
    else:
        print(f"FAIL: {name} -- {detail}")
        FAILURES.append(name)


def stub(directory: Path, name: str, body: str) -> None:
    path = directory / name
    path.write_text(body)
    path.chmod(path.stat().st_mode | stat.S_IEXEC)


def run(script: Path, tmpdir: Path, root: Path, *, with_sccache: bool) -> int:
    """Run `script` over /bin/true with TMPDIR pointed at a missing directory."""
    stubs = root / "stubs"
    stubs.mkdir(exist_ok=True)
    if with_sccache:
        # Reports rather than compiles: this must never touch the real daemon,
        # which other sessions are building against.
        stub(stubs, "sccache", "#!/usr/bin/env bash\nexec \"$@\"\n")
    environment = dict(os.environ)
    environment["PATH"] = f"{stubs}:/usr/bin:/bin"
    environment["TMPDIR"] = str(tmpdir)
    # No compositor and no runtime dir: the runner takes a fail-open path, which
    # is exactly where the directory still has to be made.
    environment["POSTIO_HEADLESS"] = "0"
    environment.pop("XDG_RUNTIME_DIR", None)
    if not with_sccache:
        environment["SCCACHE_DIR"] = str(root / "unused-sccache")
    return subprocess.run(
        [str(script), "/bin/true"],
        env=environment,
        capture_output=True,
        text=True,
    ).returncode


with tempfile.TemporaryDirectory() as raw:
    root = Path(raw)

    # ── the runner, on its fail-open path ────────────────────────────────
    missing = root / "tree-a" / "target" / "tmp"
    code = run(RUNNER, missing, root, with_sccache=False)
    check(
        "the runner creates a missing TMPDIR",
        missing.is_dir(),
        f"{missing} still does not exist (exit {code})",
    )
    check("the runner still ran the binary", code == 0, f"exit {code}")

    # ── the wrapper, without sccache ─────────────────────────────────────
    missing = root / "tree-b" / "target" / "tmp"
    code = run(WRAPPER, missing, root, with_sccache=False)
    check(
        "the wrapper creates a missing TMPDIR with no sccache",
        missing.is_dir(),
        f"{missing} still does not exist (exit {code})",
    )

    # ── the wrapper, with sccache: cargo's directory, not the daemon's ───
    # The branch below replaces TMPDIR with sccache's own scratch, so this is
    # the case that catches the mkdir being written on the wrong side of it.
    missing = root / "tree-c" / "target" / "tmp"
    code = run(WRAPPER, missing, root, with_sccache=True)
    check(
        "the wrapper creates cargo's TMPDIR even on the sccache path",
        missing.is_dir(),
        f"{missing} still does not exist (exit {code})",
    )

    # ── fail open: an uncreatable TMPDIR must not stop the run ───────────
    blocker = root / "blocker"
    blocker.write_text("not a directory")
    code = run(RUNNER, blocker / "target" / "tmp", root, with_sccache=False)
    check(
        "the runner fails open when TMPDIR cannot be created",
        code == 0,
        f"a TMPDIR under a regular file stopped the run (exit {code})",
    )

if FAILURES:
    print(f"\n{len(FAILURES)} case(s) failed: {', '.join(FAILURES)}")
    sys.exit(1)
print("\ntmpdir-created check passed.")
