#!/usr/bin/env bash
# Make a .app bundle carry every non-system library it loads, and refuse one
# that still reaches outside itself.
#
# `postio-ffi` builds a cdylib as well as a staticlib, ld64 prefers the
# `.dylib` when both are on the search path, and cargo gives that dylib its
# absolute build path as its install name. So the `Postio.app` the first
# release dry run built (#1714) loaded
#
#     /Users/runner/work/postio/postio/target/release/deps/libpostio_ffi.dylib
#
# and would have launched on the machine that built it and nowhere else
# (#1723). Linking the staticlib instead would mean reproducing every native
# library and framework its dependencies ask for, by hand, on a platform no
# Linux session can build; copying the library the build already linked and
# tested is the smaller change, and the check below is what makes it safe.
#
# For every Mach-O under Contents/MacOS and Contents/Frameworks, each load
# command that is neither a system path (/usr/lib, /System) nor relative to
# the bundle (@executable_path, @loader_path, @rpath) must name a file that
# exists. That file is copied into Contents/Frameworks, given an
# @rpath-relative install name, and the reference is rewritten to find it
# there. Libraries brought in are themselves walked, so a dependency of a
# dependency comes too. Then everything is checked again, and anything still
# pointing outside the bundle is a failure.
#
# It does not sign. Rewriting a load command invalidates a signature, and an
# Apple Silicon Mac kills a binary whose signature is invalid, so the caller
# signs afterwards -- the nested libraries first, then the bundle, which is
# what `scripts/macos-bundle.sh` does.
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

# The libraries a Mach-O loads, one per line. `otool -L` prints the file's
# own name first (with a colon) and, for a dylib, its install name next --
# both skipped, the second by comparing against `otool -D`.
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

# Bring in until a pass finds nothing new: each copied library is itself
# walked on the next pass.
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

# Whatever is left pointing outside the bundle is a library the build machine
# does not have either; the bundle cannot be made to work, so say which.
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
