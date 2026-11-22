#!/usr/bin/env bash
# Install the pinned `cargo-machete` side by side with its other versions. Idempotent
# (scripts/lib/pinned-tool.sh says why); CI installs plain into ~/.cargo/bin.
# `check-unused-deps.py` runs it and skips, saying so, when absent, so a fresh clone passes
# `check.sh`; `ci.yml` caches ~/.cargo/bin/cargo-machete on this file's hash.
# `cargo install`, not a tarball: cargo-machete publishes source only, and its tree is small.
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

pt_cargo_install
