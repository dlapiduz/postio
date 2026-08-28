#!/usr/bin/env bash
# Assemble Postio.app around the binary `macos-build.sh` produced.
#
# SwiftPM cannot emit an application bundle, so this does the four things Xcode
# would: make the directory layout, put the executable in it, write the
# Info.plist, and sign.
#
# The signature is **ad-hoc** (`codesign -s -`), which is what makes building
# Postio need no Apple Developer account (ADR 0019 Q8). It comes with a
# consequence worth knowing rather than discovering: an ad-hoc identity changes
# on every rebuild, so macOS treats each build as a different application and
# the Keychain asks again the first time a new build reads a secret.
#
# Usage:
#   scripts/macos-bundle.sh [--release]
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

# The icon, rendered from the SVG the GTK frontend already ships rather than
# from a set of PNGs checked in beside it -- the rule the design tokens follow,
# for the same reason: a copy is correct on the day it is made.
#
# AppKit reads that SVG as a vector, so each size is a real render rather than
# an upscale of the 128px PNG, and `swift` and `iconutil` are on every Mac. A
# rasterizer from Homebrew would be a build dependency contributors do not have.
ICON_SVG=crates/postio-gtk/data/icons/scalable/apps/dev.postio.Postio.svg
ICONSET=$(mktemp -d)/Postio.iconset
if swift scripts/macos-icon.swift "$ICON_SVG" "$ICONSET" \
    && iconutil -c icns -o "$APP/Contents/Resources/Postio.icns" "$ICONSET"; then
    :
else
    # Not fatal. An application with no icon is worse-looking and entirely
    # usable, and failing the whole bundle over artwork would be the wrong
    # trade for someone trying to run the thing.
    echo "could not build the icon; the bundle has none." >&2
fi
rm -rf "$(dirname "$ICONSET")"

# `-s -` is an ad-hoc signature: no identity, no team, no account. The
# entitlements are the local ones, which disable library validation -- with no
# team to compare against, validation refuses code a contributor's own build
# legitimately contains.
codesign --force --sign - \
    --entitlements macos/Resources/PostioReleaseLocal.entitlements \
    "$APP" >/dev/null 2>&1 || {
        echo "codesign failed; the bundle is built but not signed." >&2
        echo "It will still run, with more Gatekeeper friction." >&2
    }

echo "$APP"
echo
echo "Run it with:  open $APP"
