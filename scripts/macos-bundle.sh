#!/usr/bin/env bash
# Assemble Postio.app around the binary `macos-build.sh` produced: SwiftPM cannot emit a bundle. The
# signature is ad-hoc by default (`codesign -s -`, no Apple account; ADR 0019 Q8), so each rebuild
# is a different application to the Keychain and "Always Allow" does not stick.
# `POSTIO_CODESIGN_IDENTITY` signs with a real certificate (any code-signing one, even self-signed),
# which binds the Keychain ACL to the identity; opt-in, so the artifact is not changed by whatever
# certificate is in the keychain.
#
# Usage:
#   scripts/macos-bundle.sh [--release]
#   POSTIO_CODESIGN_IDENTITY="Apple Development: you@example.com (TEAMID)" \
#       scripts/macos-bundle.sh
set -euo pipefail

REPO_ROOT=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
cd "$REPO_ROOT"

CONFIG=debug
for arg in "$@"; do
    case "$arg" in
        --release) CONFIG=release ;;
        *) echo "unknown argument: $arg" >&2; exit 2 ;;
    esac
done

BINARY="macos/.build/$CONFIG/Postio"
if [ ! -x "$BINARY" ]; then
    echo "no executable at $BINARY." >&2
    echo "Run scripts/macos-build.sh${CONFIG:+ ${CONFIG/debug/}} first." >&2
    exit 1
fi

APP="macos/build/Postio.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BINARY" "$APP/Contents/MacOS/Postio"
cp macos/Resources/Info.plist "$APP/Contents/Info.plist"

# The attachment-text helper is read in its own process, killed at its deadline
# (specs/010-focus-search D28); the indexer looks beside its own executable.
# Fatal when missing: the app would refuse PDFs.
HELPER="${CARGO_TARGET_DIR:-target}/$CONFIG/postio-extract-helper"
if [ ! -x "$HELPER" ]; then
    echo "no extraction helper at $HELPER." >&2
    echo "Run scripts/macos-build.sh${CONFIG:+ ${CONFIG/debug/}} first." >&2
    exit 1
fi
cp "$HELPER" "$APP/Contents/MacOS/postio-extract-helper"

# The agent entry point (ADR 0010): a person pastes Postio.app/Contents/MacOS/postio-mcp into a
# client's config. Fatal when missing.
MCP="${CARGO_TARGET_DIR:-target}/$CONFIG/postio-mcp"
if [ ! -x "$MCP" ]; then
    echo "no postio-mcp at $MCP." >&2
    echo "Run scripts/macos-build.sh${CONFIG:+ ${CONFIG/debug/}} first." >&2
    exit 1
fi
cp "$MCP" "$APP/Contents/MacOS/postio-mcp"

# The icon is rendered from the SVG the GTK frontend ships, as the design tokens are: a checked-in
# copy is right only on the day it is made.
ICON_SVG=crates/postio-widgets/data/icons/scalable/apps/dev.postio.Postio.svg
ICONSET=$(mktemp -d)/Postio.iconset
if swift scripts/macos-icon.swift "$ICON_SVG" "$ICONSET" \
    && iconutil -c icns -o "$APP/Contents/Resources/Postio.icns" "$ICONSET"; then
    :
else
    # Not fatal: an app with no icon is usable.
    echo "could not build the icon; the bundle has none." >&2
fi
rm -rf "$(dirname "$ICONSET")"

# The executable loads `libpostio_ffi.dylib` from the cargo target directory by absolute path, so a
# bundle would launch only on the machine that built it (#1723). Copy the library in, point the
# executable at it, and fail if anything still reaches outside.
scripts/macos-self-contained.sh "$APP"

# `-s -` is an ad-hoc signature with the local entitlements, which disable library validation. Sign
# nested libraries first, then the bundle:
# signing the bundle does not sign inside it, and the step above invalidated the linker's signature.
# Fatal: Apple Silicon kills a mismatched binary.
IDENTITY="${POSTIO_CODESIGN_IDENTITY:--}"
if [ -d "$APP/Contents/Frameworks" ]; then
    for library in "$APP"/Contents/Frameworks/*; do
        codesign --force --sign "$IDENTITY" "$library"
    done
fi
# The helper is nested code: signed with the same identity before the bundle, under its own
# identifier, with no entitlements (it loads only system libraries; if the app is ever sandboxed it
# needs `com.apple.security.inherit`).
codesign --force --sign "$IDENTITY" \
    --identifier dev.postio.Postio.extract-helper \
    "$APP/Contents/MacOS/postio-extract-helper"
# postio-mcp reads the Keychain when it opens the store itself, so it is signed with the same
# identity, under its own identifier.
codesign --force --sign "$IDENTITY" \
    --identifier dev.postio.Postio.mcp \
    "$APP/Contents/MacOS/postio-mcp"
codesign --force --sign "$IDENTITY" \
    --entitlements macos/Resources/PostioReleaseLocal.entitlements \
    "$APP"
codesign --verify --deep --strict "$APP"

if [ "$IDENTITY" = "-" ]; then
    available=$(security find-identity -v -p codesigning 2>/dev/null \
        | sed -n 's/.*"\(.*\)"/\1/p' | head -1)
    if [ -n "$available" ]; then
        echo
        echo "Signed ad-hoc, so the Keychain will ask again after every rebuild."
        echo "To make \"Always Allow\" stick, sign with a stable identity:"
        echo
        echo "  export POSTIO_CODESIGN_IDENTITY=\"$available\""
        echo
    fi
fi

echo "$APP"
echo
echo "Run it with:  open $APP"
