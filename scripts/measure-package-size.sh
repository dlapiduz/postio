#!/usr/bin/env bash
# Is the terminal package small enough beside the desktop one? (docs/archive/specs/005-tui-frontend
# SC-004: binaries under half, Flatpak under two-thirds)
# Measured form for form from one commit: binaries with the shared libraries they load (each counted
# once per side), and each installed Flatpak
# with the runtime it names (the app alone would compare the wrong things). The release workflow
# runs both after the packages are built.
# `scripts/tests/test-measure-package-size.py` is what tests this.
#
# Usage:
#   scripts/measure-package-size.sh binaries TUI_BIN... -- DESKTOP_BIN...
#   scripts/measure-package-size.sh flatpak TUI_APP_ID DESKTOP_APP_ID
#
# Exit status: 0 under the bar, 1 at or over it, 2 called wrongly or something
# could not be measured.
set -euo pipefail

usage() {
    sed -n '/^# Usage:/,/^# Exit/p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

# The bytes of every file under the given paths, symlinks followed, a hard-linked file counted once.
# Not `du -b`: macOS's `du` has no `-b`; `stat` answers everywhere in GNU `-c` or BSD `-f` dialects.
bytes_of() {
    local format
    if stat -c '%s' / >/dev/null 2>&1; then
        format=(-c '%d:%i %s')
    else
        format=(-f '%d:%i %z')
    fi
    find -L "$@" -type f -exec stat "${format[@]}" {} + |
        awk '!seen[$1]++ { total += $2 } END { print total + 0 }'
}

closure() {
    local libraries
    libraries=$(
        for executable in "$@"; do
            ldd "$executable" 2>/dev/null | awk '
                $2 == "=>" && $3 ~ /^\// { print $3 }
                $1 ~ /^\// { print $1 }
            ' || true
        done | sort -u
    )
    # shellcheck disable=SC2086 # one path per line, none with spaces
    bytes_of "$@" $libraries
}

installed() {
    local app=$1 location runtime runtime_location
    location=$(flatpak info --show-location "$app" 2>/dev/null) ||
        { echo "no installed Flatpak $app" >&2; exit 2; }
    runtime=$(flatpak info --show-runtime "$app" 2>/dev/null) ||
        { echo "$app names no runtime" >&2; exit 2; }
    runtime_location=$(flatpak info --show-location "$runtime" 2>/dev/null) ||
        { echo "the runtime $runtime of $app is not installed" >&2; exit 2; }
    bytes_of "$location" "$runtime_location"
}

# Under `numerator/denominator` of the desktop's size passes: half for binaries; two-thirds for a
# Flatpak, whose runtime is most of it and already the smallest Flathub has (first dry run: 58%).
compare() {
    local form=$1 terminal=$2 desktop=$3 numerator=$4 denominator=$5 bar=$6
    if [ "$desktop" -le 0 ]; then
        echo "$form: the desktop package measured as nothing" >&2
        exit 2
    fi
    local percent=$(( terminal * 100 / desktop ))
    echo "$form: terminal $terminal bytes, desktop $desktop bytes ($percent%)"
    if [ $(( terminal * denominator )) -ge $(( desktop * numerator )) ]; then
        echo "$form: the terminal package is not under $bar of the desktop one (SC-004)" >&2
        exit 1
    fi
}

[ $# -ge 1 ] || usage
mode=$1
shift
case "$mode" in
    binaries)
        terminal=()
        while [ $# -gt 0 ] && [ "$1" != "--" ]; do
            terminal+=("$1")
            shift
        done
        [ $# -ge 2 ] && [ ${#terminal[@]} -ge 1 ] || usage
        shift
        for path in "${terminal[@]}" "$@"; do
            [ -e "$path" ] || { echo "no such file: $path" >&2; exit 2; }
        done
        compare binaries "$(closure "${terminal[@]}")" "$(closure "$@")" 1 2 half
        ;;
    flatpak)
        [ $# -eq 2 ] || usage
        tui=$(installed "$1")
        desktop=$(installed "$2")
        compare flatpak "$tui" "$desktop" 2 3 two-thirds
        ;;
    *)
        usage
        ;;
esac
