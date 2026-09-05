#!/usr/bin/env bash
# The linker for every binary on this target, wired up in `.cargo/config.toml`
# as `[target.x86_64-unknown-linux-gnu] linker`: use mold when the machine has
# it, and rustc's own lld when it does not.
#
# # Why mold, when it is not faster
#
# It is not faster here, and that is not the reason it is wired in. Measured
# interleaved on an idle box, six rounds, timing the whole `rustc` invocation
# for the `app_suite` test binary -- the largest in the repository:
#
#     rust-lld  median 1.235s   maxRSS 734 MB
#     mold      median 1.16s    maxRSS 469 MB
#
# The time difference is inside the noise, because there is only ~1.2s of
# compile-and-link to contest (docs/engineering-notes.md). The memory is not:
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
set -euo pipefail

if command -v mold >/dev/null 2>&1; then
    exec cc "$@" -fuse-ld=mold
fi

# No mold: rustc's own arguments already select the lld inside the pinned
# toolchain, so there is nothing to restore.
exec cc "$@"
