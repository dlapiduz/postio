#!/usr/bin/env bash
# Install the pinned `cargo-nextest` side by side with its other versions. Idempotent. A branch that
# bumps the pin does not unseat
# everyone else's (scripts/lib/pinned-tool.sh says why); CI installs plain into ~/.cargo/bin.
# A script, not README text: `issue-land.sh` falls open to `cargo test` without nextest and says
# nothing, which hid a 4x slower landing (#1092);
# the pin and checksum live here once.
# The published binary, not `cargo install`, which compiles the tool's ~200 dependencies a second
# time in its own target dir.
# A checksum, not `--locked`: the asset is immutable, so a changed digest means stop. Two checksums,
# one per host, because the
# tooling self-tests run on macOS too and `scripts/lib/prereq.py` fails a missing prerequisite under
# CI.
# To bump: change the version, download both tarballs, and put their `sha256sum` here.
set -euo pipefail

VERSION="0.9.152"
SHA256_LINUX="d039b43c3e0958441b6ad0a39c8884d018b3d2e8fb9928f209074b1100b5e17a"
SHA256_MAC="2fadf57df4243e691f088ee3bff4f4ab9af16da0c95a1db84b5cb1685e098a76"

HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib/pinned-tool.sh
. "$HERE/lib/pinned-tool.sh"
pt_init cargo-nextest "$VERSION" nextest

if pt_have; then
    echo "cargo-nextest $VERSION already installed"
    pt_install_shim
    exit 0
fi

# `sha256sum` is GNU and absent on macOS, where the tool is `shasum -a 256`; both read `<digest>
# <path>` on stdin with `-c`.
case "$(uname -s)" in
Darwin)
    ASSET="mac"
    SHA256="$SHA256_MAC"
    CHECK="shasum -a 256 -c -"
    ;;
Linux)
    ASSET="linux"
    SHA256="$SHA256_LINUX"
    CHECK="sha256sum -c -"
    ;;
*)
    echo "install-nextest: no pinned asset for $(uname -s)" >&2
    exit 1
    ;;
esac

TARBALL=$(mktemp -t nextest-XXXXXX.tar.gz)
trap 'rm -f "$TARBALL"' EXIT

curl -fsSL --retry 3 -o "$TARBALL" "https://get.nexte.st/$VERSION/$ASSET"
echo "$SHA256  $TARBALL" | $CHECK

pt_stage
trap 'rm -f "$TARBALL"; rm -rf "$PT_STAGE"' EXIT
tar -xzf "$TARBALL" -C "$PT_STAGE/bin" cargo-nextest
pt_commit

"$PT_BIN/cargo-nextest" nextest --version
