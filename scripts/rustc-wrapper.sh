#!/usr/bin/env bash
# Cargo's RUSTC_WRAPPER (`.cargo/config.toml`): use sccache when the machine has it, get out of the
# way when it does not (`mise use -g sccache` installs it).
# An explicit RUSTC_WRAPPER in the environment wins.
# Why TMPDIR is set here (#359): sccache is ONE daemon shared by every session and takes its temp
# directory from the `TMPDIR` of whichever client spawned it,
# ignoring later clients. `.cargo/config.toml`'s `TMPDIR = target/tmp` is per worktree, so the first
# cargo after a restart donated its worktree's `target/tmp`,
# and when `issue-release.sh` deleted it every build on the machine failed with "Failed to create
# temp dir". Pinning it beside sccache's own cache outlives
# every worktree and stays on disk, not the 6 GB tmpfs. Only on the sccache path: without it,
# cargo's per-worktree TMPDIR keeps scratch off the tmpfs.
# Cargo's own `target/tmp` is created here, since nothing else does and the first temp file in a
# fresh clone fails with `NotFound` (#613); done before the
# branch replaces TMPDIR, on every platform. Fail open, like everything else in this file.
if [ -n "${TMPDIR:-}" ]; then
    mkdir -p "$TMPDIR" 2>/dev/null || true
fi

# Sizing: sccache's default 10 GiB cache was full on this box, so every compile evicted somebody
# else's entry. 60G now (#1199); 30G filled too (72% hits, ~18,000 misses at ~5 s).
# `SCCACHE_IDLE_TIMEOUT=0` keeps the server up (the default stops it after ten idle minutes, costing
# a cold start).
# `SCCACHE_ERROR_LOG` says where log records go and `SCCACHE_LOG=info` decides whether there are any
# (#1184): the error log alone is created empty.
# `info` is four lines per server start and nothing per compile; `:-` lets a session chasing a wedge
# ask for `debug`.
# These are read when the server STARTS: after changing them run `sccache --stop-server`; `sccache
# --show-stats` prints "Max cache size". An explicit value in the environment wins.
# A --remap-path-prefix goes in conditionally (#1106): a build script that generates Rust source the
# crate `include!`s leaves the generated file's absolute path in the
# artifact (serde_core, serde), so every downstream crate misses across worktrees. The flag itself
# contains the per-worktree path and sccache does not normalise it out
# of its key (unconditionally, a second tree went from 10 hits to zero), and it cannot be injected
# below sccache. So it applies only when the build script generated Rust
# source; one that only prints `cargo:rustc-cfg` leaves no path and must not get it. The replacement
# is a constant. Held by `scripts/tests/test-rustc-wrapper-remap.py`.
if [ -n "${OUT_DIR:-}" ]; then
    # `compgen -G`, not a glob test, so an OUT_DIR that is missing or holds no Rust source answers
    # no; a shell without it costs the remapping, never the build.
    if compgen -G "${OUT_DIR}/*.rs" >/dev/null 2>&1; then
        set -- "$@" --remap-path-prefix="${OUT_DIR}=/postio-out"
    fi
fi

if command -v sccache >/dev/null 2>&1; then
    export SCCACHE_CACHE_SIZE="${SCCACHE_CACHE_SIZE:-60G}"
    export SCCACHE_IDLE_TIMEOUT="${SCCACHE_IDLE_TIMEOUT:-0}"
    export SCCACHE_ERROR_LOG="${SCCACHE_ERROR_LOG:-${SCCACHE_DIR:-${HOME:-}/.cache/sccache}/sccache.log}"
    export SCCACHE_LOG="${SCCACHE_LOG:-info}"
    scratch="${SCCACHE_DIR:-${HOME:-}/.cache/sccache}/tmp"
    if mkdir -p "$scratch" 2>/dev/null; then
        export TMPDIR="$scratch"
    fi
    exec sccache "$@"
fi
exec "$@"
