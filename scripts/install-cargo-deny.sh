#!/usr/bin/env bash
# Install the pinned `cargo-deny` into ~/.cargo/bin. Idempotent.
#
# `scripts/checks/check-dependency-policy.py` runs it when present, and the
# two self-test cases that exercise that check need it too. They had never
# run in CI on either platform: the `supply-chain` job runs cargo-deny inside
# its own action's container, and the jobs that run `scripts/tests/` never
# had the tool on PATH, so the cases skipped and the job was green (#1290).
# The pin lives here, once; `ci.yml` caches ~/.cargo/bin/cargo-deny on this
# file's hash and calls this in both self-test jobs, the same shape as
# `scripts/install-machete.sh`.
#
# `cargo install` rather than a release tarball, for the same reason as
# machete: one script for every platform the self-tests run on.
set -euo pipefail

VERSION="0.20.2"

if command -v cargo-deny >/dev/null 2>&1; then
    installed=$(cargo-deny --version 2>/dev/null | awk '{print $NF}')
    if [ "$installed" = "$VERSION" ]; then
        echo "cargo-deny $VERSION already installed"
        exit 0
    fi
    echo "cargo-deny ${installed:-unknown} installed; replacing with $VERSION"
fi

# `--force`: a binary restored from a cache of another version is exactly
# what the branch above found, and cargo refuses to overwrite it otherwise.
cargo install cargo-deny --version "$VERSION" --locked --force
echo "cargo-deny $VERSION installed"
