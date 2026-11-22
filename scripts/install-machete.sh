#!/usr/bin/env bash
# Install the pinned `cargo-machete` side by side with its other versions. Idempotent
# (scripts/lib/pinned-tool.sh says why); CI installs plain into ~/.cargo/bin.
# `check-unused-deps.py` runs it and skips, saying so, when absent, so a fresh clone passes
# `check.sh`; `ci.yml` caches ~/.cargo/bin/cargo-machete on this file's hash.
set -euo pipefail

VERSION="0.9.2"

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib/pinned-tool.sh
. "$HERE/lib/pinned-tool.sh"
pt_init cargo-machete "$VERSION"

if pt_have; then
    echo "cargo-machete $VERSION already installed"
    pt_install_shim
    exit 0
fi

case "$(uname -s)" in Darwin) TRIPLE=aarch64-apple-darwin ;; *) TRIPLE=x86_64-unknown-linux-musl ;; esac
pt_tarball_install \
    "https://github.com/bnjbvr/cargo-machete/releases/download/v$VERSION/cargo-machete-v$VERSION-x86_64-unknown-linux-musl.tar.gz" \
    48200087f54c55aabcd4db4af1e25742b49846c02a1b1bfa134711945b35b2e9 \
    "https://github.com/bnjbvr/cargo-machete/releases/download/v$VERSION/cargo-machete-v$VERSION-aarch64-apple-darwin.tar.gz" \
    63e28fee386d82d33f2d12406c857f98e2d4697f3f7df7f71f34dff07fca0fde \
    "cargo-machete-v$VERSION-$TRIPLE/cargo-machete"
