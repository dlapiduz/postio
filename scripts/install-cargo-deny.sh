#!/usr/bin/env bash
# Install the pinned `cargo-deny` side by side with its other versions. Idempotent. Versions live in
# ~/.local/share/postio-tools/ (scripts/lib/pinned-tool.sh says why); CI installs plain into
# ~/.cargo/bin. `check-dependency-policy.py` and two self-test cases need it; `ci.yml` caches
# ~/.cargo/bin/cargo-deny on this file's hash (#1290).
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

case "$(uname -s)" in Darwin) TRIPLE=aarch64-apple-darwin ;; *) TRIPLE=x86_64-unknown-linux-musl ;; esac
pt_tarball_install \
    "https://github.com/EmbarkStudios/cargo-deny/releases/download/$VERSION/cargo-deny-$VERSION-x86_64-unknown-linux-musl.tar.gz" \
    9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f \
    "https://github.com/EmbarkStudios/cargo-deny/releases/download/$VERSION/cargo-deny-$VERSION-aarch64-apple-darwin.tar.gz" \
    fe67d82a10d8597a3549364cb733a3f9cc1bfff9031b7ae46384a9f2a72090c3 \
    "cargo-deny-$VERSION-$TRIPLE/cargo-deny"
