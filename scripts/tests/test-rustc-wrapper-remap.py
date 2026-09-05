#!/usr/bin/env python3
"""Self-test for scripts/rustc-wrapper.sh's OUT_DIR path remapping (#1106).

A build script that *generates Rust source the crate `include!`s* puts the
generated file's absolute path into the compiled artifact:

    $ strings target/debug/deps/libserde_core-*.rmeta | grep out/private.rs
    /home/.../issue-1141/target/debug/build/serde_core-<hash>/out/private.rs

That path names this worktree, so `libserde_core.rmeta` differs byte for byte
between two trees at the same commit. sccache hashes `--extern` inputs by
*content*, so every crate downstream of serde then misses too -- which in this
workspace is nearly all of them. Measured on a two-tree harness, serde_core and
serde were the only artifacts embedding a path, and everything else that
differed was downstream of them (#1106's first comment).

The wrapper therefore hands rustc a `--remap-path-prefix` for this crate's
`OUT_DIR`, so the artifact records a constant instead of a worktree.

# Why it is conditional, and why the condition is "any .rs under OUT_DIR"

The flag necessarily *contains* the per-worktree path, and sccache does not
normalise `--remap-path-prefix` out of its key -- measured: passing one
unconditionally through `RUSTFLAGS` took a second tree from 10 cache hits to
zero. So it may only go to compiles that already miss across trees, which is
exactly the ones whose build script generated Rust source. A build script that
only prints `cargo:rustc-cfg` (proc-macro2, num_traits) leaves no path in the
artifact, already hits, and must not be given the flag.

`sccache` is stubbed on PATH and reports the arguments it was handed, so this
runs anywhere, fast, and never starts or stops the real shared daemon.

Usage: scripts/tests/test-rustc-wrapper-remap.py
Exit status: 0 all cases behaved, 1 otherwise.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
WRAPPER = HERE / "rustc-wrapper.sh"

REPORT_ARGV = """#!/usr/bin/env bash
printf '%s\\n' "$@"
"""

FAILURES: list[str] = []


def run(stub_dir: Path, out_dir: Path | None) -> list[str]:
    """Run the wrapper with `OUT_DIR` set as cargo would, report the argv."""
    environment = dict(os.environ)
    # The stub directory first and the real toolchain dropped, so a real
    # sccache on this machine cannot make a case pass that would fail without
    # one. /usr/bin and /bin stay because the wrapper is a bash script.
    environment["PATH"] = f"{stub_dir}:/usr/bin:/bin"
    environment.pop("OUT_DIR", None)
    if out_dir is not None:
        environment["OUT_DIR"] = str(out_dir)
    result = subprocess.run(
        ["bash", str(WRAPPER), "rustc", "--crate-name", "probe", "src/lib.rs"],
        env=environment,
        capture_output=True,
        text=True,
        timeout=30,
    )
    return result.stdout.splitlines()


def remaps(argv: list[str]) -> list[str]:
    return [a for a in argv if a.startswith("--remap-path-prefix")]


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
        sccache.write_text(REPORT_ARGV)
        sccache.chmod(0o755)

        # A build script that generated source the crate includes -- serde_core.
        generating = base / "worktrees" / "issue-999" / "target" / "debug" / "build"
        generating = generating / "serde_core-abc123" / "out"
        generating.mkdir(parents=True)
        (generating / "private.rs").write_text("// generated\n")

        argv = run(stub_dir, generating)
        found = remaps(argv)
        case(
            "a generated .rs under OUT_DIR gets the path remapped",
            len(found) == 1,
            "without it the worktree's own path is compiled into the "
            f"artifact and every dependant misses; got {found!r}",
        )
        if found:
            case(
                "the remap names this crate's OUT_DIR as the prefix",
                found[0].startswith(f"--remap-path-prefix={generating}="),
                f"expected the OUT_DIR as the left-hand side, got {found[0]!r}",
            )
            case(
                "and rewrites it to something with no worktree in it",
                str(generating) not in found[0].split("=", 2)[2],
                "a replacement carrying the worktree path remaps nothing; "
                f"got {found[0]!r}",
            )
        case(
            "the compiler still gets its own arguments",
            argv[:3] == ["rustc", "--crate-name", "probe"],
            f"the wrapper must pass the invocation through; got {argv!r}",
        )

        # A build script that only emitted cfgs -- proc-macro2, num_traits.
        # These already hit across worktrees, and the flag would cost that.
        cfg_only = base / "worktrees" / "issue-999" / "target" / "debug" / "build"
        cfg_only = cfg_only / "num_traits-def456" / "out"
        cfg_only.mkdir(parents=True)
        (cfg_only / "probe.ll").write_text("; not rust source\n")

        case(
            "a build script that generated no Rust source gets no flag",
            remaps(run(stub_dir, cfg_only)) == [],
            "the flag carries a per-worktree path and sccache does not "
            "normalise it out, so an unconditional one turns hits into misses",
        )
        case(
            "an OUT_DIR that does not exist gets no flag",
            remaps(run(stub_dir, base / "gone")) == [],
            "nothing was generated, so there is no path to remap",
        )
        case(
            "a crate with no build script gets no flag",
            remaps(run(stub_dir, None)) == [],
            "most compiles have no OUT_DIR at all and must be left alone",
        )

    for failure in FAILURES:
        print(f"FAIL  {failure}", file=sys.stderr)
    return 1 if FAILURES else 0


if __name__ == "__main__":
    sys.exit(main())
