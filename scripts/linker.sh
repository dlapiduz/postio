#!/usr/bin/env bash
# The linker for every binary on Linux x86_64 and macOS (`.cargo/config.toml`): on Linux mold when
# present, else rustc's own lld.
# Why mold: not faster (1.16s vs 1.235s, noise) but its peak RSS is ~265 MB lower (469 vs 734 MB),
# the scarce resource when several sessions link.
# Why a wrapper: `-fuse-ld=mold` is a hard link error without mold, and `-B<dir>` does not select
# it, because rustc appends its own
# `-B <sysroot>/.../gcc-ld -fuse-ld=lld` even with `-C linker-flavor=gcc`. gcc honours the LAST
# `-fuse-ld`, so `"$@"` goes first and ours last.
# Verify with `readelf -p .comment <binary>`, which names the linker that actually ran.
# The no-mold path needs nothing: `"$@"` already selects lld. Fail open, like
# `scripts/rustc-wrapper.sh` and `scripts/cc-wrapper.sh`.
# Test binaries are stripped, and only those: close to a third is the symbol table (`ffi_suite` 124
# MB to 85 MB) and nothing reads it but a
# RUST_BACKTRACE dump. `POSTIO_KEEP_SYMBOLS=1` keeps them. Done here, not `[profile.test] strip`,
# which gives every crate a second build.
# A test binary is one built with `CARGO_TARGET_TMPDIR` set or linking libtest.
# On macOS this is also the linker, for the strip alone: ld64 cannot drop global symbols, so `strip`
# runs after the link and re-signs (Apple silicon requires it).
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

if [ "$test_binary" = 1 ]; then
    set -- "$@" -Wl,--strip-all
fi

if command -v mold >/dev/null 2>&1; then
    exec cc "$@" -fuse-ld=mold
fi

exec cc "$@"
