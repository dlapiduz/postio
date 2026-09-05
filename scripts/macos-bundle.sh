#!/usr/bin/env bash
# Assemble Postio.app around the binary `macos-build.sh` produced.
#
# SwiftPM cannot emit an application bundle, so this does the four things Xcode
# would: make the directory layout, put the executable in it, write the
# Info.plist, and sign.
#
# The signature is **ad-hoc** by default (`codesign -s -`), which is what makes
# building Postio need no Apple Developer account (ADR 0019 Q8). It comes with
# a consequence worth knowing rather than discovering: an ad-hoc identity
# changes on every rebuild, so macOS treats each build as a different
# application and the Keychain asks again the first time a new build reads a
# secret. **"Always Allow" does not stick**, because the thing it was allowed
# for no longer exists.
#
# `POSTIO_CODESIGN_IDENTITY` signs with a real certificate instead, and that
# is the whole cure: a Keychain ACL is bound to the signing identity rather
# than to the binary's hash, so it survives every rebuild. Any code-signing
# certificate does -- an Apple Development one, or a self-signed one made in
# Keychain Access (Certificate Assistant -> Create a Certificate, type "Code
# Signing"). No account, no team, no notarization.
#
# Opt-in rather than automatic on purpose: picking up whatever certificate
# happens to be in the keychain would change what the artifact *is* without
# anybody asking for it. `security find-identity -v -p codesigning` lists what
# is available, and this prints the hint when it finds one.
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
IDENTITY="${POSTIO_CODESIGN_IDENTITY:--}"
codesign --force --sign "$IDENTITY" \
    --entitlements macos/Resources/PostioReleaseLocal.entitlements \
    "$APP" >/dev/null 2>&1 || {
        echo "codesign failed; the bundle is built but not signed." >&2
        echo "It will still run, with more Gatekeeper friction." >&2
    }

if [ "$IDENTITY" = "-" ]; then
    # Only when there is something to suggest. A contributor with no
    # certificate is not doing anything wrong and should not be told off for
    # it -- ad-hoc is the supported path.
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
