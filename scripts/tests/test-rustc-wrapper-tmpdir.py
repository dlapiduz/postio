#!/usr/bin/env python3
"""Self-test for scripts/rustc-wrapper.sh's temp-directory pinning.

sccache is one persistent daemon shared by every session on the machine, and
it takes its temp directory from `TMPDIR` in the environment of whichever
client invocation happened to *spawn* it. It never re-reads that per request.
`.cargo/config.toml` sets `TMPDIR = target/tmp` relative to the workspace
root, so the daemon inherits one worktree's `target/tmp` and keeps using it
for every session's compiles thereafter.

Two things follow, and #359 is the first of them:

  * `scripts/issue-release.sh` deletes that worktree the moment its issue
    lands, and every build on the machine then fails with
    "Failed to create temp dir ... No such file or directory" naming a path
    with nothing wrong in it;
  * the tmpfs protection `TMPDIR = target/tmp` exists to provide is
    accidental -- it holds only while the spawning worktree's `target/tmp`
    is what the daemon happens to be holding. A daemon started from a plain
    shell gets the real `/tmp`, which on this machine is a 6 GB tmpfs, and
    linking the GTK stack fills it.

So the wrapper pins the daemon's temp directory to somewhere that outlives
any worktree, beside the cache the daemon already owns. `sccache` is stubbed
on PATH and reports the `TMPDIR` it was handed, so this runs anywhere, fast,
and -- importantly -- never starts or stops the real shared daemon, which
other sessions are building against.

Usage: scripts/tests/test-rustc-wrapper-tmpdir.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "lib"))

import patience  # noqa: E402  -- enabled by the sys.path line above

HERE = Path(__file__).resolve().parent.parent
WRAPPER = HERE / "rustc-wrapper.sh"

REPORT_TMPDIR = """#!/usr/bin/env bash
printf '%s' "${TMPDIR:-<unset>}"
"""

FAILURES: list[str] = []


def run(
    stub_dir: Path,
    *,
    tmpdir: str,
    sccache_dir: str | None,
    with_sccache: bool,
    target: Path,
) -> subprocess.CompletedProcess[str]:
    """Run the wrapper and hand back what the stub it exec'd reported."""
    environment = dict(os.environ)
    environment["PATH"] = f"{stub_dir}:/usr/bin:/bin"
    environment["TMPDIR"] = tmpdir
    environment.pop("SCCACHE_DIR", None)
    if sccache_dir is not None:
        environment["SCCACHE_DIR"] = sccache_dir
    if with_sccache:
        (stub_dir / "sccache").write_text(REPORT_TMPDIR)
        (stub_dir / "sccache").chmod(0o755)
    else:
        (stub_dir / "sccache").unlink(missing_ok=True)
    return patience.run(
        ["bash", str(WRAPPER), str(target)],
        env=environment,
        capture_output=True,
        text=True,
        timeout=30,
    )


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)
        stub_dir = base / "stubs"
        stub_dir.mkdir()

        sccache = stub_dir / "sccache"
        sccache.write_text(REPORT_TMPDIR)
        sccache.chmod(0o755)

        rustc = stub_dir / "rustc"
        rustc.write_text(REPORT_TMPDIR)
        rustc.chmod(0o755)

        worktree_tmp = base / "worktrees" / "issue-999" / "target" / "tmp"
        worktree_tmp.mkdir(parents=True)

        cache_dir = base / "cache" / "sccache"
        stable_tmp = cache_dir / "tmp"

        result = run(
            stub_dir,
            tmpdir=str(worktree_tmp),
            sccache_dir=str(cache_dir),
            with_sccache=True,
            target=rustc,
        )
        seen = result.stdout
        case(
            "sccache is not handed a worktree's target/tmp",
            seen != str(worktree_tmp),
            "the daemon would be pinned to a directory issue-release.sh "
            f"deletes; it got {seen!r}",
        )
        case(
            "sccache is handed a temp dir beside its own cache",
            seen == str(stable_tmp),
            f"expected {str(stable_tmp)!r}, got {seen!r}",
        )
        case(
            "that temp dir is created rather than merely named",
            stable_tmp.is_dir(),
            f"{stable_tmp} does not exist, so the first compile still fails",
        )

        home = base / "home"
        home.mkdir()
        environment_home = dict(os.environ)
        environment_home["HOME"] = str(home)
        environment_home["PATH"] = f"{stub_dir}:/usr/bin:/bin"
        environment_home["TMPDIR"] = str(worktree_tmp)
        environment_home.pop("SCCACHE_DIR", None)
        defaulted = patience.run(
            ["bash", str(WRAPPER), str(rustc)],
            env=environment_home,
            capture_output=True,
            text=True,
            timeout=30,
        ).stdout
        case(
            "with no SCCACHE_DIR it still leaves the worktree",
            defaulted != str(worktree_tmp) and defaulted != "<unset>",
            f"got {defaulted!r}",
        )
        case(
            "and lands beside the default cache location",
            defaulted == str(home / ".cache" / "sccache" / "tmp"),
            f"got {defaulted!r}",
        )

        plain = run(
            stub_dir,
            tmpdir=str(worktree_tmp),
            sccache_dir=str(cache_dir),
            with_sccache=False,
            target=rustc,
        )
        case(
            "without sccache the wrapper leaves TMPDIR alone",
            plain.stdout == str(worktree_tmp),
            f"expected the worktree's own tmp, got {plain.stdout!r}",
        )

        sccache.write_text(REPORT_TMPDIR)
        sccache.chmod(0o755)
        blocker = base / "blocker"
        blocker.write_text("not a directory")
        stuck = run(
            stub_dir,
            tmpdir=str(worktree_tmp),
            sccache_dir=str(blocker / "sccache"),
            with_sccache=True,
            target=rustc,
        )
        case(
            "an uncreatable temp dir still compiles",
            stuck.returncode == 0,
            f"exited {stuck.returncode}: {stuck.stderr.strip()!r}",
        )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("rustc-wrapper tmpdir check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
