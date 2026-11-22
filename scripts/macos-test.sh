#!/usr/bin/env bash
# Run the Swift tests with the Rust library where the linker can find it: the staticlib is in
# cargo's target directory, and Package.swift cannot name it without barring the package from being
# a dependency.
set -euo pipefail

REPO_ROOT=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
cd "$REPO_ROOT"

# The tests that drive the real controller open a demo store, which needs the demo feature; this
# library is the tests' alone.
export POSTIO_FFI_FEATURES="${POSTIO_FFI_FEATURES:-demo}"
scripts/macos-build.sh --lib-only

TARGET_DIR="${CARGO_TARGET_DIR:-target}"
cd macos
exec swift test \
    -Xlinker -L"$REPO_ROOT/$TARGET_DIR/debug" \
    -Xlinker -lpostio_ffi "$@"
