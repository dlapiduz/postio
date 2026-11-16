#!/usr/bin/env bash
# Install the pinned `cargo-hakari` side by side with its other versions. Idempotent.
#
# Versions live in ~/.local/share/postio-tools/ and `cargo hakari` resolves the
# one the current checkout pins; scripts/lib/pinned-tool.sh says why. CI
# installs plain into ~/.cargo/bin.
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
