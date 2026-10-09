#!/usr/bin/env bash
# Install the pinned `cargo-hakari` into ~/.cargo/bin. Idempotent.
#
# `scripts/checks/check-workspace-hack.py` runs it, and skips, saying so, when
# it is absent on a workstation -- the trade `install-machete.sh` describes.
# The pin lives here, once: `ci.yml`'s boundaries job caches
# ~/.cargo/bin/cargo-hakari on this file's hash and calls this. Regenerating
# the hack (`cargo hakari generate`) with a different version can reformat
# it, so a workstation that edits manifests wants this version too.
set -euo pipefail

VERSION="0.9.39"

# `cargo-hakari --version` prints `cargo-hakari 0.9.39`; the last field is
# the version.
if command -v cargo-hakari >/dev/null 2>&1; then
    installed=$(cargo-hakari --version 2>/dev/null | awk '{print $NF}')
    if [ "$installed" = "$VERSION" ]; then
        echo "cargo-hakari $VERSION already installed"
        exit 0
    fi
    echo "cargo-hakari ${installed:-unknown} installed; replacing with $VERSION"
fi

# `--force`: a binary restored from a cache of another version is exactly
# what the branch above found, and cargo refuses to overwrite it otherwise.
cargo install cargo-hakari --version "$VERSION" --locked --force
echo "cargo-hakari $VERSION installed"
