#!/usr/bin/env bash
# The C compiler cargo hands to build scripts, wired up in .cargo/config.toml
# as `[env] CC`: put a machine-wide compile cache in front of every
# build-script C compile, and get out of the way when the machine has none.
#
# scripts/rustc-wrapper.sh gives every *Rust* compile the shared sccache, and
# ADR 0014 priced the vendored OpenSSL on the assumption the same held for C
# ("sccache absorbs it machine-wide"). It did not: the C compiler inside the
# openssl-src, libsqlite3-sys and zstd-sys build scripts is invoked by
# make/cc directly, which RUSTC_WRAPPER never sees. Measured on a fresh
# worktree target, 77% of a `cargo build -p postio-storage` was uncached C —
# ~4 minutes at the pinned `jobs = 2`, paid again by every worktree, on the
# critical path of the gate chain (#736).
#
# ccache, NOT sccache, and that is measured rather than preferred: openssl-src
# extracts and compiles its sources inside each target directory, so every
# include path and `#line` directive carries that target dir's absolute path.
# sccache does no path normalization for C — 0.17% hit rate across two fresh
# target dirs, pure overhead. ccache exists for exactly this: with `base_dir`
# and `hash_dir = false` it rewrites the paths out of the hash, and the same
# two-build experiment hit on 1193 of 1196 compiles (#736). The env defaults
# below are that configuration; a value already exported wins, like
# everything else here.
#
# The protocol differs from rustc-wrapper.sh's, which is why this is a second
# script rather than one shared one: a RUSTC_WRAPPER is *handed* the compiler
# as $1, while `$CC` **is** the compiler. So this stands in for cc itself and
# prepends `ccache cc`. `cc`, not a hardcoded gcc/clang: it is the platform's
# own default-compiler name on both Linux and macOS, and the one thing the
# openssl-src/cc-crate machinery agrees on resolving.
#
# Fail open, like rustc-wrapper.sh: no ccache on the box and this is exactly
# `cc "$@"`. A session's own CC in the environment wins over cargo's `[env]`
# (cargo never overrides what is already set), so a build that needs a bare
# or different compiler still gets one by exporting it.
#
# ccache runs in-process — no shared daemon — so the sccache daemon-TMPDIR
# hazard (#359) does not exist on this path. Cargo's own TMPDIR still needs
# creating (#613), same as in rustc-wrapper.sh.
if [ -n "${TMPDIR:-}" ]; then
    mkdir -p "$TMPDIR" 2>/dev/null || true
fi

if command -v ccache >/dev/null 2>&1; then
    # Everything under $HOME — the worktrees, the shared checkout — hashes
    # relative, so one worktree's OpenSSL is every worktree's.
    export CCACHE_BASEDIR="${CCACHE_BASEDIR:-${HOME:-/}}"
    export CCACHE_NOHASHDIR="${CCACHE_NOHASHDIR:-1}"
    # openssl-src re-extracts its sources per target dir, so their mtimes are
    # always fresh; without the sloppiness every worktree's first build would
    # re-validate the whole cache the slow way.
    export CCACHE_SLOPPINESS="${CCACHE_SLOPPINESS:-time_macros,include_file_mtime,include_file_ctime}"
    exec ccache cc "$@"
fi
exec cc "$@"
