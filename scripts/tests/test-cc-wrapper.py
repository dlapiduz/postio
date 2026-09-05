#!/usr/bin/env python3
"""Self-test for scripts/cc-wrapper.sh, the C-compile half of the cache.

`scripts/rustc-wrapper.sh` gives every Rust compile the machine-wide sccache,
and ADR 0014 priced the vendored OpenSSL on the assumption the same held for
C ("sccache absorbs it machine-wide"). It did not: the C compiler inside the
openssl-src, libsqlite3-sys and zstd-sys build scripts is invoked by make/cc
directly, which RUSTC_WRAPPER never sees — 77% of a fresh worktree's
postio-storage build was uncached C (#736).

The C half caches through **ccache**, not sccache, and the cases below pin
the reasons that must not drift:

  * The protocol: a RUSTC_WRAPPER is *handed* the compiler as its first
    argument, while `$CC` **is** the compiler — so cc-wrapper.sh stands in
    for cc itself and must prepend both `ccache` and `cc`, and be exactly
    `cc "$@"` on a box without ccache.
  * The configuration: openssl-src builds inside each target directory, so
    without `base_dir`/`hash_dir=false` normalization every worktree's paths
    poison the hash and nothing ever hits across worktrees (sccache has no
    such normalization — 0.17% measured, which is why it is not used here).
    The wrapper must default that configuration and must not override a
    value the session already exported.
  * Cargo points TMPDIR at a `target/tmp` nothing creates (#613); the
    wrapper fronts the C compiler the way rustc-wrapper.sh fronts rustc, so
    it must create it too.

Usage: scripts/tests/test-cc-wrapper.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
WRAPPER = HERE / "cc-wrapper.sh"

# One stub for both ccache and cc: report every argument and the environment
# the wrapper is expected to shape, so each case can ask both "what was run"
# and "what was it handed".
REPORT = """#!/usr/bin/env bash
printf '%s\\n' "$0" "$@"
printf 'CCACHE_BASEDIR=%s\\n' "${CCACHE_BASEDIR:-<unset>}"
printf 'CCACHE_NOHASHDIR=%s\\n' "${CCACHE_NOHASHDIR:-<unset>}"
printf 'CCACHE_SLOPPINESS=%s\\n' "${CCACHE_SLOPPINESS:-<unset>}"
"""

FAILURES: list[str] = []


def case(name: str, condition: bool, detail: str) -> None:
    if condition:
        print(f"ok    {name}")
    else:
        FAILURES.append(f"{name}: {detail}")


def run(
    stub_dir: Path,
    *,
    with_ccache: bool,
    tmpdir: str,
    extra_env: dict[str, str] | None = None,
    args: list[str],
) -> subprocess.CompletedProcess[str]:
    """Run the wrapper with stubs on PATH and hand back what they reported."""
    environment = dict(os.environ)
    # Stubs first and the real toolchain directories dropped, so a real
    # ccache on this machine cannot make a case pass that would fail on a
    # box without one. /usr/bin and /bin stay for bash and `command`.
    environment["PATH"] = f"{stub_dir}:/usr/bin:/bin"
    environment["TMPDIR"] = tmpdir
    for name in ("CCACHE_BASEDIR", "CCACHE_NOHASHDIR", "CCACHE_SLOPPINESS"):
        environment.pop(name, None)
    if extra_env:
        environment.update(extra_env)
    ccache = stub_dir / "ccache"
    if with_ccache:
        ccache.write_text(REPORT)
        ccache.chmod(0o755)
    else:
        ccache.unlink(missing_ok=True)
    return subprocess.run(
        ["bash", str(WRAPPER), *args],
        env=environment,
        capture_output=True,
        text=True,
        timeout=30,
    )


def main() -> int:
    with tempfile.TemporaryDirectory() as raw:
        base = Path(raw)
        stub_dir = base / "stubs"
        stub_dir.mkdir()

        cc = stub_dir / "cc"
        cc.write_text(REPORT)
        cc.chmod(0o755)

        tmp = base / "target" / "tmp"
        tmp.mkdir(parents=True)

        # ── with ccache: `ccache cc <args>`, nothing dropped ──────────────
        got = run(
            stub_dir,
            with_ccache=True,
            tmpdir=str(tmp),
            args=["-O2", "-c", "sqlite3.c", "-o", "sqlite3.o"],
        )
        lines = got.stdout.splitlines()
        case(
            "with ccache the compile goes through the cache",
            bool(lines) and lines[0].endswith("ccache"),
            f"expected the ccache stub to run, got {lines[:1]!r}",
        )
        case(
            "and ccache is handed cc plus the arguments, in order",
            lines[1:6] == ["cc", "-O2", "-c", "sqlite3.c", "-o"],
            f"got {lines[1:6]!r}",
        )
        home = os.environ.get("HOME", "/")
        case(
            "paths are normalized so worktrees share one cache",
            f"CCACHE_BASEDIR={home}" in lines and "CCACHE_NOHASHDIR=1" in lines,
            f"a target-dir path in the hash means 0% hits across worktrees; "
            f"got {[l for l in lines if l.startswith('CCACHE_')]!r}",
        )
        case(
            "re-extracted sources do not re-validate the world",
            any(
                l.startswith("CCACHE_SLOPPINESS=") and "include_file_mtime" in l
                for l in lines
            ),
            f"got {[l for l in lines if l.startswith('CCACHE_SLOPPINESS')]!r}",
        )

        # ── the session's own ccache configuration wins ───────────────────
        preset = run(
            stub_dir,
            with_ccache=True,
            tmpdir=str(tmp),
            extra_env={"CCACHE_BASEDIR": "/somewhere/else"},
            args=["-c", "x.c"],
        )
        case(
            "an exported CCACHE_BASEDIR is not overridden",
            "CCACHE_BASEDIR=/somewhere/else" in preset.stdout.splitlines(),
            f"got {preset.stdout.splitlines()!r}",
        )

        # ── without ccache: exactly `cc <args>` ───────────────────────────
        plain = run(
            stub_dir,
            with_ccache=False,
            tmpdir=str(tmp),
            args=["-O2", "-c", "sqlite3.c"],
        )
        lines = plain.stdout.splitlines()
        case(
            "without ccache the wrapper is exactly cc",
            bool(lines) and lines[0].endswith("/cc"),
            f"expected the cc stub to run, got {lines[:1]!r}",
        )
        case(
            "with the arguments untouched",
            lines[1:4] == ["-O2", "-c", "sqlite3.c"],
            f"got {lines[1:4]!r}",
        )

        # ── cargo's TMPDIR is created before the compiler wants it (#613) ─
        fresh_tmp = base / "fresh" / "target" / "tmp"
        created = run(
            stub_dir,
            with_ccache=False,
            tmpdir=str(fresh_tmp),
            args=["-c", "x.c"],
        )
        case(
            "a TMPDIR nothing created yet is created",
            created.returncode == 0 and fresh_tmp.is_dir(),
            f"exited {created.returncode}; {fresh_tmp} exists: "
            f"{fresh_tmp.is_dir()}",
        )

    if FAILURES:
        print(f"\n{len(FAILURES)} case(s) failed:", file=sys.stderr)
        for failure in FAILURES:
            print(f"- {failure}", file=sys.stderr)
        return 1
    print("cc-wrapper check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
