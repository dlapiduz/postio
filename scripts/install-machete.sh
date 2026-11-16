#!/usr/bin/env bash
# Install the pinned `cargo-machete` side by side with its other versions. Idempotent.
#
# Versions live in ~/.local/share/postio-tools/ and `cargo machete` resolves the
# one the current checkout pins; scripts/lib/pinned-tool.sh says why. CI
# installs plain into ~/.cargo/bin.
#
# `scripts/checks/check-unused-deps.py` runs it and skips, saying so, when it
# is absent -- the same trade `check-dependency-policy.py` makes for
# cargo-deny, because a fresh clone must pass `check.sh` before it has built
# any tool. The pin lives here, once: `ci.yml`'s boundaries job caches
# ~/.cargo/bin/cargo-machete on this file's hash and calls this, so the gate
# is real on every pull request whatever a workstation has installed.
#
# `cargo install` rather than a release tarball: cargo-machete publishes
# source only, and its tree is small (~40 crates, about a minute cold).
set -euo pipefail

VERSION="0.9.2"

# `--version` prints the bare number (`0.9.2`), not `cargo-machete 0.9.2`:
# the last field is the version whichever shape a release chooses.
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
