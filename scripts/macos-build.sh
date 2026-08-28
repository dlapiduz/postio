#!/usr/bin/env bash
# Build the macOS application.
#
# Five joins, each of which fails in its own way: cargo builds a staticlib,
# the in-workspace `uniffi-bindgen` reads its metadata, the generated Swift and
# module map land where SwiftPM expects them, `swift build` links the lot, and
# `macos-bundle.sh` assembles a `.app` around it. Doing them by hand in the
# wrong order produces a Swift file that compiles against nothing, which is
# why this exists.
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
# A plain string rather than an array: macOS ships bash 3.2, where expanding an
# empty array under `set -u` is an error rather than nothing. Word-splitting is
# safe here because the only values are fixed flags.
CARGO_PROFILE_ARGS=""
for arg in "$@"; do
    case "$arg" in
        --release)   PROFILE=release; CARGO_PROFILE_ARGS="--release" ;;
        --lib-only)  LIB_ONLY=1 ;;
        *) echo "unknown argument: $arg" >&2; exit 2 ;;
    esac
done

if ! command -v swift >/dev/null 2>&1; then
    echo "no swift on PATH: this builds the macOS application and needs Xcode." >&2
    exit 2
fi

TARGET_DIR="${CARGO_TARGET_DIR:-target}"

echo "--- cargo: postio-ffi ---"
# shellcheck disable=SC2086  # deliberate: see CARGO_PROFILE_ARGS above
cargo build -p postio-ffi $CARGO_PROFILE_ARGS

# The bindings are generated every time rather than tracked, so the generator
# and the `uniffi` runtime are the same version by construction (#571). They
# land in two targets because SwiftPM cannot have a module map and Swift
# sources in one, and because nothing hand-written should share a directory
# with something a build step overwrites.
echo "--- bindings ---"
GENERATED=$(mktemp -d)
trap 'rm -rf "$GENERATED"' EXIT
scripts/ffi-bindgen.sh "$GENERATED" >/dev/null

C_TARGET=macos/Sources/postio_ffiFFI
SWIFT_TARGET=macos/Sources/PostioFFI
rm -rf "$C_TARGET" "$SWIFT_TARGET"
mkdir -p "$C_TARGET" "$SWIFT_TARGET"
cp "$GENERATED/postio_ffiFFI.h" "$C_TARGET/"
# SwiftPM requires the file to be named `module.modulemap`; uniffi names it
# after the module. Copying rather than renaming in place keeps the generator's
# output untouched and this rule visible.
cp "$GENERATED/postio_ffiFFI.modulemap" "$C_TARGET/module.modulemap"
cp "$GENERATED/postio_ffi.swift" "$SWIFT_TARGET/"
echo "  -> $C_TARGET, $SWIFT_TARGET"

if [ "$LIB_ONLY" = 1 ]; then
    echo "library and bindings only, as asked."
    exit 0
fi

echo "--- swift build ---"
# The library search path lives here rather than in Package.swift: putting it
# in the manifest would need `.unsafeFlags`, which pins an absolute path and
# bars the package from ever being a dependency.
LIB_DIR="$REPO_ROOT/$TARGET_DIR/$PROFILE"
SWIFT_CONFIG=$([ "$PROFILE" = release ] && echo release || echo debug)
(cd macos && swift build -c "$SWIFT_CONFIG" \
    -Xlinker -L"$LIB_DIR" \
    -Xlinker -lpostio_ffi)

echo
echo "built. Next:"
echo "  scripts/macos-bundle.sh${CARGO_PROFILE_ARGS:+ --release}   # assemble Postio.app"
