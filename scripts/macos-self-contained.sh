#!/usr/bin/env bash
# Make a .app bundle carry every non-system library it loads, and refuse one that still reaches
# outside itself (#1714, #1723).
# `postio-ffi` builds a cdylib and a staticlib, ld64 prefers the `.dylib`, and cargo gives it its
# absolute build path as install name,
# so the bundle would launch only on the machine that built it. Copying the linked library beats
# reproducing every native dependency by hand.
# Each load command in a Mach-O under Contents/MacOS and Contents/Frameworks that is neither a
# system path nor bundle-relative must name an
# existing file; it is copied into Contents/Frameworks with an @rpath install name, the reference
# rewritten, and brought-in libraries walked in turn.
# Then everything is checked again; anything still outside is a failure.
# It does not sign: rewriting a load command invalidates the signature, so the caller
# (`macos-bundle.sh`) signs afterwards.
#
# Usage: scripts/macos-self-contained.sh <path/to/App.app>
# Exit status: 0 self-contained, 1 something outside the bundle could not be
# brought in (each named), 2 usage.
set -euo pipefail

APP="${1:-}"
if [ -z "$APP" ] || [ ! -d "$APP/Contents/MacOS" ]; then
    echo "usage: scripts/macos-self-contained.sh <App.app>" >&2
    exit 2
fi
FRAMEWORKS="$APP/Contents/Frameworks"

is_macho() { file -b "$1" | grep -q '^Mach-O'; }

# The libraries a Mach-O loads, one per line; `otool -L` prints the file's own name and, for a
# dylib, its install name first, both skipped (the second against `otool -D`).
loads() {
    local own
    own=$(otool -D "$1" 2>/dev/null | sed -n '2p')
    otool -L "$1" | sed -n '2,$p' | sed 's/^[[:space:]]*//; s/ (compatibility.*$//' |
        { if [ -n "$own" ]; then grep -vxF -- "$own" || true; else cat; fi; }
}

inside_or_system() {
    case "$1" in
        /usr/lib/* | /System/* | @executable_path/* | @loader_path/* | @rpath/*) return 0 ;;
        *) return 1 ;;
    esac
}

binaries() {
    find "$APP/Contents/MacOS" "$FRAMEWORKS" -type f 2>/dev/null |
        while IFS= read -r candidate; do
            if is_macho "$candidate"; then printf '%s\n' "$candidate"; fi
        done
}

changed=1
while [ "$changed" = 1 ]; do
    changed=0
    while IFS= read -r binary; do
        while IFS= read -r library; do
            [ -n "$library" ] || continue
            inside_or_system "$library" && continue
            if [ ! -f "$library" ]; then
                continue    # reported by the check below
            fi
            name=$(basename "$library")
            mkdir -p "$FRAMEWORKS"
            if [ ! -f "$FRAMEWORKS/$name" ]; then
                cp "$library" "$FRAMEWORKS/$name"
                chmod u+w "$FRAMEWORKS/$name"
                install_name_tool -id "@rpath/$name" "$FRAMEWORKS/$name"
                echo "bundled $library -> Contents/Frameworks/$name"
            fi
            case "$binary" in
                "$FRAMEWORKS"/*) target="@loader_path/$name" ;;
                *) target="@executable_path/../Frameworks/$name" ;;
            esac
            install_name_tool -change "$library" "$target" "$binary"
            changed=1
        done < <(loads "$binary")
    done < <(binaries)
done

outside=0
while IFS= read -r binary; do
    while IFS= read -r library; do
        [ -n "$library" ] || continue
        inside_or_system "$library" && continue
        echo "${binary#"$APP"/} loads $library, which is outside the bundle and not on this machine" >&2
        outside=1
    done < <(loads "$binary")
done < <(binaries)

if [ "$outside" = 1 ]; then
    echo "not self-contained: it would not launch on another Mac (#1723)." >&2
    exit 1
fi
echo "self-contained: every library is the system's or inside $(basename "$APP")."
