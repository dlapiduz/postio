#!/usr/bin/env bash
# The linker for every binary on Linux x86_64 and macOS, wired up in
# `.cargo/config.toml`: on Linux, mold when the machine has
# it, and rustc's own lld when it does not.
#
# # Why mold, when it is not faster
#
# It is not faster here, and that is not the reason it is wired in. Measured
# interleaved on an idle box, six rounds, timing the whole `rustc` invocation
# for the desktop app's test binary -- the largest in the repository:
#
#     rust-lld  median 1.235s   maxRSS 734 MB
#     mold      median 1.16s    maxRSS 469 MB
#
# The time difference is inside the noise, because there is only ~1.2s of
# compile-and-link to contest (docs/archive/engineering-notes.md). The memory is not:
# mold's peak is ~265 MB below lld's. On a workstation where four sessions
# link concurrently, that is the scarce resource -- it is the whole reason
# `jobs = 2` and the linker thread cap in `.cargo/config.toml` exist.
#
# # Why a wrapper and not a flag
#
# Selecting mold from cargo config alone cannot fail open, and the obvious
# spelling does not even work:
#
#   * `-C link-arg=-fuse-ld=mold` is a hard link error on a box without mold,
#     which is every CI runner here.
#   * `-C link-arg=-B<dir containing ld>` does not select mold *at all*.
#     rustc's default flavour for this target is `gnu-lld-cc`, and it appends
#     its own `-B` and `-fuse-ld=lld` after yours. The build succeeds, is
#     fast, and is linked by LLD. This was hit while measuring, and it is
#     silent by construction.
#
# **Verify with `readelf -p .comment <binary>`.** It names the linker that
# actually ran, and it is the only thing that separates a real result from a
# plausible one.
#
# # `-fuse-ld=mold` goes LAST, and that is the whole trick
#
# rustc appends its own linker selection to every invocation on this target,
# **even with `-C linker-flavor=gcc`**:
#
#     -B <sysroot>/lib/rustlib/x86_64-unknown-linux-gnu/bin/gcc-ld
#     -fuse-ld=lld
#
# `gcc` honours the *last* `-fuse-ld`, so a wrapper that runs
# `cc -fuse-ld=mold "$@"` is silently overridden by the `-fuse-ld=lld` that
# arrives inside `"$@"`. It builds, it is fast, and LLD linked it. This was
# only caught by checking `readelf -p .comment` on the output; there is no
# error, no warning, and no difference in behaviour to notice.
#
# So: `"$@"` first, ours last.
#
# It also means the no-mold path needs nothing at all -- rustc's own `-B` and
# `-fuse-ld=lld` are already in `"$@"`, so plain `cc` reproduces exactly
# today's behaviour. Fail open, like `scripts/rustc-wrapper.sh` and
# `scripts/cc-wrapper.sh`.
#
# # Test binaries are stripped, and only test binaries
#
# Each test binary links the crate's whole dependency stack, and close to a
# third of it is the symbol table: `ffi_suite` is 124 MB as linked and 85 MB
# stripped, on macOS, 2026-10-22. Nothing reads those symbols but a
# `RUST_BACKTRACE` dump; a failing test prints its `file:line` from data
# compiled in, not from symbols. Set `POSTIO_KEEP_SYMBOLS=1` for a run whose
# backtraces you want named.
#
# Here and not in `[profile.test] strip`, which was measured: a test-profile
# setting gives every workspace crate a second build, one for `cargo build`
# and one for `cargo test`, which today share theirs. The linker sees only
# the link, so nothing else changes.
#
# A test binary is one cargo builds with `CARGO_TARGET_TMPDIR` set, which it
# does only for integration tests and benches, or one that links libtest,
# which is every unit-test binary. An application or library is untouched.
#
# On macOS this is also the linker (`.cargo/config.toml`), for the strip
# alone: ld64 cannot drop global symbols, so `strip` runs on the output after
# the link, as rustc's own `strip = "symbols"` does there. It re-signs the
# binary, which Apple silicon requires.
set -euo pipefail

test_binary=0
if [ "${POSTIO_KEEP_SYMBOLS:-0}" != 1 ]; then
    if [ -n "${CARGO_TARGET_TMPDIR:-}" ]; then
        test_binary=1
    else
        for arg in "$@"; do
            case "$arg" in
                */libtest-*.rlib) test_binary=1; break ;;
            esac
        done
    fi
fi

if [ "$(uname -s)" = Darwin ]; then
    if [ "$test_binary" = 0 ]; then
        exec cc "$@"
    fi
    output=""
    previous=""
    for arg in "$@"; do
        if [ "$previous" = -o ]; then
            output="$arg"
        fi
        previous="$arg"
    done
    cc "$@"
    if [ -n "$output" ]; then
        strip "$output"
    fi
    exit 0
fi

# `-fuse-ld=mold` stays last, after the strip flag, for the reason above.
if [ "$test_binary" = 1 ]; then
    set -- "$@" -Wl,--strip-all
fi

if command -v mold >/dev/null 2>&1; then
    exec cc "$@" -fuse-ld=mold
fi

# No mold: rustc's own arguments already select the lld inside the pinned
# toolchain, so there is nothing to restore.
exec cc "$@"
