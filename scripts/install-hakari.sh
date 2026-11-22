#!/usr/bin/env bash
# Install the pinned `cargo-hakari` side by side with its other versions. Idempotent
# (scripts/lib/pinned-tool.sh says why).
# `check-workspace-hack.py` runs it and skips, saying so, when it is absent; `ci.yml` caches
# ~/.cargo/bin/cargo-hakari on this file's hash.
# Regenerating the hack with a different version can reformat it, so a workstation that edits
# manifests wants this version too.
set -euo pipefail

VERSION="0.9.39"

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib/pinned-tool.sh
. "$HERE/lib/pinned-tool.sh"
pt_init cargo-hakari "$VERSION"

if pt_have; then
    echo "cargo-hakari $VERSION already installed"
    pt_install_shim
    exit 0
fi

pt_tarball_install \
    "https://github.com/guppy-rs/guppy/releases/download/cargo-hakari-$VERSION/cargo-hakari-$VERSION-x86_64-unknown-linux-gnu.tar.gz" \
    699a3193a9bd1a8d914b970d3f50f3385f24fbd8b1294d0c0ff31fbe7c2c9f48 \
    "https://github.com/guppy-rs/guppy/releases/download/cargo-hakari-$VERSION/cargo-hakari-$VERSION-universal-apple-darwin.tar.gz" \
    c1f84026b805cc7fb31c271e8b0720abadb8d9416d126c76d6fc50a2a147d7f9 \
    "cargo-hakari"
