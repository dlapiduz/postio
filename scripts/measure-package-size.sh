#!/usr/bin/env bash
# Is the terminal package under half the desktop one? (specs/005-tui-frontend
# SC-004)
#
# "Smaller" is measured form for form, from one commit (the spec's
# Assumptions):
#
#   binaries  the standalone download's executables with the shared libraries
#             they load, against the desktop executable with its libraries.
#             Each library is counted once per side.
#   flatpak   each installed Flatpak with the runtime it names. The terminal
#             package's runtime is the plain freedesktop one and the
#             desktop's is GNOME's, which is most of the difference; the app
#             alone would compare the wrong things.
#
# The release workflow runs both after the packages are built, and a
# terminal package at half its desktop counterpart or more fails the release.
# `scripts/tests/test-measure-package-size.py` is what tests this.
#
# Usage:
#   scripts/measure-package-size.sh binaries TUI_BIN... -- DESKTOP_BIN...
#   scripts/measure-package-size.sh flatpak TUI_APP_ID DESKTOP_APP_ID
#
# Exit status: 0 under half, 1 half or more, 2 called wrongly or something
# could not be measured.
set -euo pipefail

usage() {
    sed -n '/^# Usage:/,/^# Exit/p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

# The bytes of every file under the given paths, symlinks followed, and a
# file reached more than once -- a hard link, which is how a Flatpak's store
# shares files between installs -- counted once.
#
# Not `du -b`: macOS's `du` has no `-b`, and the self-test runs there. `stat`
# answers everywhere, in one of two dialects: GNU's `-c`, BSD's `-f`.
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

# The executables and every shared library they load, each counted once.
closure() {
    local libraries
    libraries=$(
        for executable in "$@"; do
            # Not ELF, or static: no libraries, and that is an answer.
            ldd "$executable" 2>/dev/null | awk '
                $2 == "=>" && $3 ~ /^\// { print $3 }
                $1 ~ /^\// { print $1 }
            ' || true
        done | sort -u
    )
    # shellcheck disable=SC2086 # one path per line, none with spaces
    bytes_of "$@" $libraries
}

# An installed Flatpak and the runtime it names.
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

compare() {
    local form=$1 terminal=$2 desktop=$3
    if [ "$desktop" -le 0 ]; then
        echo "$form: the desktop package measured as nothing" >&2
        exit 2
    fi
    local percent=$(( terminal * 100 / desktop ))
    echo "$form: terminal $terminal bytes, desktop $desktop bytes ($percent%)"
    if [ $(( terminal * 2 )) -ge "$desktop" ]; then
        echo "$form: the terminal package is not under half the desktop one (SC-004)" >&2
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
        compare binaries "$(closure "${terminal[@]}")" "$(closure "$@")"
        ;;
    flatpak)
        [ $# -eq 2 ] || usage
        tui=$(installed "$1")
        desktop=$(installed "$2")
        compare flatpak "$tui" "$desktop"
        ;;
    *)
        usage
        ;;
esac
