#!/usr/bin/env bash
# Postio's patches to Blitz, as a queue over the pinned crates.io release.
#
#   scripts/blitz-patches.sh verify   vendor/ == release + patches/blitz/series
#   scripts/blitz-patches.sh diff     print vendor/'s changes against the
#                                     release, to save as the next patch
#
# `vendor/blitz-*` is what builds (root Cargo.toml's [patch.crates-io]);
# `patches/blitz/` is what a reviewer reads, and a patch goes when upstream
# takes it. `verify` is the proof the two say the same thing. Each crate is
# read from cargo's own cache when it is there and fetched from crates.io
# otherwise, and is checked against `upstream.toml` either way.
set -euo pipefail

ROOT=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
PATCHES="$ROOT/patches/blitz"
CRATES=(blitz-dom blitz-paint)
VERSION=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$PATCHES/upstream.toml")

checksum_of() {
    awk -v want="[crates.$1]" '$0 == want { on = 1; next } /^\[/ { on = 0 }
        on && /^sha256/ { gsub(/.*= "|"/, ""); print; exit }' "$PATCHES/upstream.toml"
}

# Unpack the pristine release of every crate into $1.
pristine() {
    local into=$1 crate file
    for crate in "${CRATES[@]}"; do
        file=$(ls "${CARGO_HOME:-$HOME/.cargo}"/registry/cache/*/"$crate-$VERSION.crate" 2>/dev/null | head -1 || true)
        if [[ -z $file ]]; then
            file="$into/$crate.crate"
            curl --fail --silent --show-error --location \
                "https://static.crates.io/crates/$crate/$crate-$VERSION.crate" -o "$file"
        fi
        if [[ $(sha256sum "$file" | cut -d' ' -f1) != "$(checksum_of "$crate")" ]]; then
            echo "blitz-patches: $crate $VERSION does not match upstream.toml's checksum" >&2
            exit 1
        fi
        tar -xzf "$file" -C "$into"
        mv "$into/$crate-$VERSION" "$into/$crate"
    done
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

case ${1:-} in
verify)
    pristine "$work"
    while read -r name; do
        [[ -z $name || $name == \#* ]] && continue
        patch --quiet --forward -p0 -d "$work" <"$PATCHES/$name" || {
            echo "blitz-patches: $name does not apply to $VERSION" >&2
            exit 1
        }
    done <"$PATCHES/series"
    for crate in "${CRATES[@]}"; do
        if ! diff -r "$work/$crate" "$ROOT/vendor/$crate" >&2; then
            echo "blitz-patches: vendor/$crate is not $VERSION plus the series;" >&2
            echo "  write the change as a patch (\`$0 diff\`) or drop it" >&2
            exit 1
        fi
    done
    echo "blitz-patches: vendor/ is Blitz $VERSION plus $(grep -cv '^\s*\(#\|$\)' "$PATCHES/series") patches"
    ;;
diff)
    pristine "$work"
    cd "$work"
    for crate in "${CRATES[@]}"; do
        ln -s "$ROOT/vendor/$crate" "patched-$crate"
        # Headers name both sides by the crate and carry no timestamps, so
        # a regenerated patch differs only where the change does.
        diff -ruN "$crate" "patched-$crate/" | sed -E \
            -e "s#^diff -ruN ([^ ]*) patched-$crate/#diff -ruN \1 $crate/#" \
            -e "s#^(---|[+][+][+]) (patched-)?([^\t]*)\t.*#\1 \3#" || true
    done
    ;;
*)
    sed -n '2,/^set/p' "$0" | sed '$d; s/^# \{0,1\}//'
    exit 2
    ;;
esac
