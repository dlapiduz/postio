#!/usr/bin/env bash
# Install the pinned `cargo-machete` into ~/.cargo/bin. Idempotent.
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

if command -v cargo-machete >/dev/null 2>&1; then
    installed=$(cargo-machete --version 2>/dev/null | awk '{print $2}')
    if [ "$installed" = "$VERSION" ]; then
        echo "cargo-machete $VERSION already installed"
        exit 0
    fi
    echo "cargo-machete $installed installed; replacing with $VERSION"
fi

cargo install cargo-machete --version "$VERSION" --locked
echo "cargo-machete $VERSION installed"
