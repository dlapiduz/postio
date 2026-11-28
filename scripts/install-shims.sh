#!/usr/bin/env bash
# Put the two programs `.cargo/config.toml` names on PATH:
#
#     postio-linker   = scripts/linker.sh       (mold when present, else cc)
#     postio-cc       = scripts/cc-wrapper.sh   (ccache when present, else cc)
#
# Named rather than pathed: a path differs per worktree, and sccache hashes it and cargo folds it
# into fingerprints (2 hits against 178 misses) (#1101).
# `$CARGO_HOME/bin` because rustup puts it on every PATH that can run cargo. Copies, not symlinks: a
# symlink into a worktree dies with `issue-release.sh`.
# Idempotent and cheap, so the claim, land and test scripts run it before every gate; a stale copy
# is overwritten on the next run.
#
# Usage: scripts/install-shims.sh        # prints one line per shim it changed
set -euo pipefail

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
BIN="${CARGO_HOME:-$HOME/.cargo}/bin"
mkdir -p "$BIN"

install_shim() { # name source
    local target="$BIN/$1"
    if [ -f "$target" ] && cmp -s "$HERE/$2" "$target"; then
        return 0
    fi
    cp "$HERE/$2" "$target.tmp.$$"
    chmod 755 "$target.tmp.$$"
    mv -f "$target.tmp.$$" "$target"
    echo "installed $target (from scripts/$2)"
}

install_shim postio-linker linker.sh
install_shim postio-cc cc-wrapper.sh
