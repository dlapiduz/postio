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

pt_cargo_install
