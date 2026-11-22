#!/usr/bin/env bash
# The C compiler cargo hands to build scripts (`[env] CC`): puts ccache in front of every
# build-script C compile, and is plain `cc` without it.
# ccache, not sccache, measured: openssl-src compiles inside each target dir so paths differ;
# sccache hit 0.17%, ccache with
# `base_dir` and `hash_dir = false` hit 1193 of 1196 (#736).
# Unlike a RUSTC_WRAPPER, `$CC` is the compiler rather than handed it, so this stands in for `cc`
# and prepends `ccache cc`.
# An exported CC wins over cargo's `[env]`. ccache runs in-process, so there is no daemon-TMPDIR
# hazard (#359); cargo's TMPDIR still needs creating (#613).
if [ -n "${TMPDIR:-}" ]; then
    mkdir -p "$TMPDIR" 2>/dev/null || true
fi

if command -v ccache >/dev/null 2>&1; then
    # Paths under $HOME hash relative, so one worktree's cache is every worktree's.
    export CCACHE_BASEDIR="${CCACHE_BASEDIR:-${HOME:-/}}"
    export CCACHE_NOHASHDIR="${CCACHE_NOHASHDIR:-1}"
    # openssl-src re-extracts per target dir, so mtimes are always fresh; without this each
    # worktree's first build revalidates slowly.
    export CCACHE_SLOPPINESS="${CCACHE_SLOPPINESS:-time_macros,include_file_mtime,include_file_ctime}"
    exec ccache cc "$@"
fi
exec cc "$@"
