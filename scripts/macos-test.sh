#!/usr/bin/env bash
# Run the Swift tests, with the Rust library where the linker can find it.
#
# A bare `swift test` fails to link: the staticlib lives in cargo's target
# directory, which SwiftPM knows nothing about, and the manifest deliberately
# does not name it (an absolute path there would bar the package from ever
# being a dependency).
set -euo pipefail

REPO_ROOT=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
cd "$REPO_ROOT"

# The tests that drive the real controller (the thread bar's keyboard path)
# open a demo store, which only a build with the demo feature has. Never
# for a bundle anyone ships: this library is the tests' alone.
export POSTIO_FFI_FEATURES="${POSTIO_FFI_FEATURES:-demo}"
scripts/macos-build.sh --lib-only

TARGET_DIR="${CARGO_TARGET_DIR:-target}"
cd macos
exec swift test \
    -Xlinker -L"$REPO_ROOT/$TARGET_DIR/debug" \
    -Xlinker -lpostio_ffi "$@"
