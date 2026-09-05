#!/usr/bin/env bash
# Put the two programs `.cargo/config.toml` names on PATH:
#
#     postio-linker   = scripts/linker.sh       (mold when present, else cc)
#     postio-cc       = scripts/cc-wrapper.sh   (ccache when present, else cc)
#
# They are named rather than pathed on purpose. The config used to say
# `linker = "scripts/linker.sh"`, which cargo resolves against the config's
# own directory, so every worktree passed rustc a different
# `-C linker=/home/.../postio-worktrees/issue-N/scripts/linker.sh`. That
# string is hashed by sccache and folded into cargo's fingerprints, and it
# made the machine-wide compile cache a per-worktree one: 2 hits against 178
# misses, measured, and a `target/` copied from a sibling rebuilt all of it.
# `CC = scripts/cc-wrapper.sh` had the same shape for the C build scripts,
# which the `cc` crate reruns whenever the value of CC changes. #1101.
#
# A bare name is the same string everywhere; this is what makes it resolve.
# `$CARGO_HOME/bin` because rustup already puts it on every PATH that can
# run cargo at all, on a developer box and on a CI runner alike. Copies,
# not symlinks: a symlink into a worktree dies with `issue-release.sh`.
#
# Idempotent and cheap, so the claim, land and test scripts run it before
# every gate. A tree that never ran any of them gets cargo's own
#     error: linker `postio-linker` not found
# which names exactly the thing to run. Nothing in the repository depends on
# *where* the shims came from: `scripts/` stays the source of truth and a
# stale copy is overwritten on the next run.
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
    # Write beside and rename, so a link running right now never sees a
    # half-copied script.
    cp "$HERE/$2" "$target.tmp.$$"
    chmod 755 "$target.tmp.$$"
    mv -f "$target.tmp.$$" "$target"
    echo "installed $target (from scripts/$2)"
}

install_shim postio-linker linker.sh
install_shim postio-cc cc-wrapper.sh
