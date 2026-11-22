#!/usr/bin/env bash
# Install the pinned `cargo-deny` side by side with its other versions. Idempotent. Versions live in
# ~/.local/share/postio-tools/
# (scripts/lib/pinned-tool.sh says why); CI installs plain into ~/.cargo/bin.
# `check-dependency-policy.py` and two self-test cases need it; `ci.yml` caches
# ~/.cargo/bin/cargo-deny on this file's hash (#1290).
# `cargo install`, not a tarball: one script for every platform.
set -euo pipefail

VERSION="0.20.2"

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib/pinned-tool.sh
. "$HERE/lib/pinned-tool.sh"
pt_init cargo-deny "$VERSION"

if pt_have; then
    echo "cargo-deny $VERSION already installed"
    pt_install_shim
    exit 0
fi

pt_cargo_install
