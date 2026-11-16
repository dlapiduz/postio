#!/usr/bin/env bash
# Install the pinned `cargo-deny` side by side with its other versions. Idempotent.
#
# Versions live in ~/.local/share/postio-tools/ and `cargo deny` resolves the
# one the current checkout pins; scripts/lib/pinned-tool.sh says why. CI
# installs plain into ~/.cargo/bin.
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
