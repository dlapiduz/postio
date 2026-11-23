#!/usr/bin/env bash
# Build the macOS application: cargo builds a staticlib, `uniffi-bindgen` reads its metadata, the
# generated Swift and module map land where
# SwiftPM expects them, `swift build` links, and `macos-bundle.sh` assembles a `.app`. The order
# matters.
#
# Usage:
#   scripts/macos-build.sh              # library, bindings, swift build
#   scripts/macos-build.sh --lib-only   # stop after the bindings
#   scripts/macos-build.sh --release
set -euo pipefail

REPO_ROOT=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
cd "$REPO_ROOT"

PROFILE=debug
LIB_ONLY=0
WITH_TESTS=0
# A plain string, not an array: macOS ships bash 3.2, where expanding an empty array under `set -u`
# is an error.
CARGO_PROFILE_ARGS=""
for arg in "$@"; do
    case "$arg" in
        --release)   PROFILE=release; CARGO_PROFILE_ARGS="--release" ;;
        --lib-only)  LIB_ONLY=1 ;;
        --with-tests) WITH_TESTS=1 ;;
        *) echo "unknown argument: $arg" >&2; exit 2 ;;
    esac
done

if ! command -v swift >/dev/null 2>&1; then
    echo "no swift on PATH: this builds the macOS application and needs Xcode." >&2
    exit 2
fi

TARGET_DIR="${CARGO_TARGET_DIR:-target}"

# `.cargo/config.toml` names `postio-cc` and `postio-linker` as bare programs (#1101); a session
# that only runs this script never meets the scripts that install them.
scripts/install-shims.sh

echo "--- cargo: postio-ffi ---"
# `POSTIO_FFI_FEATURES=demo` builds the demo stores in, for `macos-shot.sh`. Never for a shipped
# bundle.
FEATURE_ARGS=""
if [ -n "${POSTIO_FFI_FEATURES:-}" ]; then
    FEATURE_ARGS="--features $POSTIO_FFI_FEATURES"
fi
# shellcheck disable=SC2086  # deliberate: see CARGO_PROFILE_ARGS above
cargo build -p postio-ffi $CARGO_PROFILE_ARGS $FEATURE_ARGS

# The attachment indexer runs in a process of its own beside the executable; without it the app
# refuses to read PDFs.
echo "--- cargo: postio-extract-helper ---"
# shellcheck disable=SC2086  # deliberate: see CARGO_PROFILE_ARGS above
cargo build -p postio-session --bin postio-extract-helper $CARGO_PROFILE_ARGS

# The agent's entry point (ADR 0010): an MCP client launches `postio-mcp`, which hands the call to
# the app over agents/mcp.sock (#1845).
echo "--- cargo: postio-mcp ---"
# shellcheck disable=SC2086  # deliberate: see CARGO_PROFILE_ARGS above
cargo build -p postio-mcp --bin postio-mcp $CARGO_PROFILE_ARGS

# Bindings are generated every time, so generator and `uniffi` runtime match (#571); two targets
# because SwiftPM cannot have a module map and Swift sources in one.
echo "--- bindings ---"
GENERATED=$(mktemp -d)
trap 'rm -rf "$GENERATED"' EXIT
# shellcheck disable=SC2086  # deliberate: see CARGO_PROFILE_ARGS above
scripts/ffi-bindgen.sh $CARGO_PROFILE_ARGS "$GENERATED" >/dev/null

C_TARGET=macos/Sources/postio_ffiFFI
SWIFT_TARGET=macos/Sources/PostioFFI
mkdir -p "$C_TARGET" "$SWIFT_TARGET"
# Only a file whose bytes changed is written: a rewrite bumps the mtime and SwiftPM rebuilds
# everything downstream.
place() { # <generated> <destination>
    if ! cmp -s "$1" "$2"; then
        cp "$1" "$2"
    fi
}
find "$C_TARGET" "$SWIFT_TARGET" -type f \
    ! \( -name postio_ffiFFI.h -o -name module.modulemap -o -name postio_ffi.swift \) -delete
place "$GENERATED/postio_ffiFFI.h" "$C_TARGET/postio_ffiFFI.h"
# SwiftPM requires the file to be named `module.modulemap`; uniffi names it after the module.
place "$GENERATED/postio_ffiFFI.modulemap" "$C_TARGET/module.modulemap"
place "$GENERATED/postio_ffi.swift" "$SWIFT_TARGET/postio_ffi.swift"
echo "  -> $C_TARGET, $SWIFT_TARGET"

# The design tokens, generated from the parsed source the GTK frontend uses (#661).
echo "--- tokens ---"
DESIGN=${POSTIO_DESIGN_SYSTEM:-$(ls -d Design/_ds/industry-*/styles.css 2>/dev/null | head -1)}
if [ -n "$DESIGN" ] && [ -f "$DESIGN" ]; then
    mkdir -p macos/Sources/PostioKit/Generated
    cargo run -q -p postio-ui --bin postio-tokens -- \
        "$DESIGN" "$GENERATED/Tokens.swift"
    place "$GENERATED/Tokens.swift" macos/Sources/PostioKit/Generated/Tokens.swift
else
    echo "  no design system found; skipping (the application will not build)" >&2
fi

if [ "$LIB_ONLY" = 1 ]; then
    # After the tokens: they are an input to `swift build` like the bindings, and `macos-test.sh`
    # could not build the Swift tests in a fresh worktree without them.
    echo "library, bindings and tokens only, as asked."
    exit 0
fi

echo "--- swift build ---"
# The library search path is here, not in Package.swift: the manifest would need `.unsafeFlags`,
# which bars the package from being a dependency.
LIB_DIR="$REPO_ROOT/$TARGET_DIR/$PROFILE"
SWIFT_CONFIG=$([ "$PROFILE" = release ] && echo release || echo debug)
# `--with-tests` builds the test bundle in the same pass so `scripts/macos-test.sh` only runs it.
BUILD_TESTS=()
if [ "$WITH_TESTS" = 1 ]; then
    BUILD_TESTS=(--build-tests)
fi
(cd macos && swift build -c "$SWIFT_CONFIG" ${BUILD_TESTS[@]+"${BUILD_TESTS[@]}"} \
    -Xlinker -L"$LIB_DIR" \
    -Xlinker -lpostio_ffi)

echo
echo "built. Next:"
echo "  scripts/macos-bundle.sh${CARGO_PROFILE_ARGS:+ --release}   # assemble Postio.app"
