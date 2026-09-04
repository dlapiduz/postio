#!/usr/bin/env bash
# Install the pinned `cargo-nextest` into ~/.cargo/bin. Idempotent.
#
# # Why this exists as a script
#
# `issue-land.sh` prefers `cargo nextest` for the integration tiers and falls
# open to `cargo test` when it is absent, and `ci.yml` runs the whole
# workspace through it. The fallback is the right design and it is invisible:
# a workstation without nextest lands successfully, just four times slower,
# and nothing in the output says which runner ran. That is how this box spent
# every landing on `cargo test` without anyone noticing (#1092).
#
# The obvious fix -- put the install command in the README -- makes the
# version and the checksum true in two places, which is the bug
# `check-toolchain-pinned.py` exists to prevent for the compiler. So the pin
# lives here, once, and both `ci.yml` and the README call this.
#
# # Why the published binary and not `cargo install`
#
# `cargo install cargo-nextest` builds the tool's own dependency tree -- clap,
# serde, regex and ~200 others -- in its own target directory, so every crate
# it shares with this workspace is compiled twice and neither copy is usable
# by the other. The release tarball is one file and ~2 seconds.
#
# # Why a checksum and not `--locked`
#
# It fixes the exact bytes rather than a dependency graph that resolves at
# build time. The asset is immutable, so a changed digest means the artifact
# changed and this should stop rather than install it.
#
# To bump: change both values below, download the tarball, and put its
# `sha256sum` here. Nothing else names them.
set -euo pipefail

VERSION="0.9.143"
SHA256="66786b9abe23920d022a182d1416b1bbc8130dd4872a9553d76985a1708dcd1e"

# `--version` rather than `command -v`: a cached or hand-installed copy at the
# wrong version is the case that matters, and it looks identical on PATH.
if cargo nextest --version 2>/dev/null | grep -qw "$VERSION"; then
    echo "cargo-nextest $VERSION already installed"
    exit 0
fi

TARBALL=$(mktemp -t nextest-XXXXXX.tar.gz)
trap 'rm -f "$TARBALL"' EXIT

curl -fsSL --retry 3 -o "$TARBALL" "https://get.nexte.st/$VERSION/linux"
echo "$SHA256  $TARBALL" | sha256sum -c -

mkdir -p "$HOME/.cargo/bin"
tar -xzf "$TARBALL" -C "$HOME/.cargo/bin" cargo-nextest

cargo nextest --version
