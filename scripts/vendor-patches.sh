#!/usr/bin/env bash
# Postio's patches to upstream crates, as queues over pinned crates.io releases.
#
#   scripts/vendor-patches.sh verify [queue...]  vendor/ == release + patches/<queue>/series,
#                                                for every queue or the ones named
#   scripts/vendor-patches.sh diff <queue>       print vendor/'s changes against the
#                                                release, to save as the next patch
#
# A queue is a directory under `patches/`: `upstream.toml` names the release
# and the crates it carries, `series` the patches in order. `vendor/<crate>`
# is what builds (root Cargo.toml's [patch.crates-io]); `patches/<queue>/` is
# what a reviewer reads, and a patch goes when upstream takes it. `verify` is
# the proof the two say the same thing. Each crate is read from cargo's own
# cache when it is there and fetched from crates.io otherwise, and is checked
# against `upstream.toml` either way.
#
# `include = [...]` in `upstream.toml` carries only those top-level paths of
# the release (its library, not its test corpus); without it, all of it.
set -euo pipefail

ROOT=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)

usage() {
    sed -n '2,/^set/p' "$0" | sed '$d; s/^# \{0,1\}//'
    exit 2
}

# The crates a queue carries: every `[crates.<name>]` table in its manifest.
crates_of() {
    sed -n 's/^\[crates\.\(.*\)\]$/\1/p' "$1/upstream.toml"
}

version_of() {
    sed -n 's/^version = "\(.*\)"$/\1/p' "$1/upstream.toml"
}

# The top-level paths a queue carries, one per line; nothing means all.
include_of() {
    sed -n 's/^include = \[\(.*\)\]$/\1/p' "$1/upstream.toml" |
        tr ',' '\n' | sed 's/^ *"//; s/" *$//; /^$/d'
}

checksum_of() {
    awk -v want="[crates.$2]" '$0 == want { on = 1; next } /^\[/ { on = 0 }
        on && /^sha256/ { gsub(/.*= "|"/, ""); print; exit }' "$1/upstream.toml"
}

# Unpack the pristine release of every crate of queue $1 into $2.
pristine() {
    local queue=$1 into=$2 crate file version keep entry
    version=$(version_of "$queue")
    keep=$(include_of "$queue")
    for crate in $(crates_of "$queue"); do
        file=$(ls "${CARGO_HOME:-$HOME/.cargo}"/registry/cache/*/"$crate-$version.crate" 2>/dev/null | head -1 || true)
        if [[ -z $file ]]; then
            file="$into/$crate.crate"
            curl --fail --silent --show-error --location \
                "https://static.crates.io/crates/$crate/$crate-$version.crate" -o "$file"
        fi
        if [[ $(sha256sum "$file" | cut -d' ' -f1) != "$(checksum_of "$queue" "$crate")" ]]; then
            echo "vendor-patches: $crate $version does not match $(basename "$queue")/upstream.toml's checksum" >&2
            exit 1
        fi
        tar -xzf "$file" -C "$into"
        mv "$into/$crate-$version" "$into/$crate"
        if [[ -n $keep ]]; then
            for entry in "$into/$crate"/* "$into/$crate"/.[!.]*; do
                [[ -e $entry ]] || continue
                grep -qxF "$(basename "$entry")" <<<"$keep" || rm -r -- "$entry"
            done
        fi
    done
}

queue_dir() {
    local dir="$ROOT/patches/$1"
    [[ -f $dir/upstream.toml ]] || {
        echo "vendor-patches: no queue patches/$1" >&2
        exit 2
    }
    echo "$dir"
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

verify() {
    local queue=$1 name crate into
    name=$(basename "$queue")
    into="$work/$name"
    mkdir "$into"
    pristine "$queue" "$into"
    while read -r patch_name; do
        [[ -z $patch_name || $patch_name == \#* ]] && continue
        patch --quiet --forward -p0 -d "$into" <"$queue/$patch_name" || {
            echo "vendor-patches: $name/$patch_name does not apply to $(version_of "$queue")" >&2
            exit 1
        }
    done <"$queue/series"
    for crate in $(crates_of "$queue"); do
        if ! diff -r "$into/$crate" "$ROOT/vendor/$crate" >&2; then
            echo "vendor-patches: vendor/$crate is not $(version_of "$queue") plus patches/$name/series;" >&2
            echo "  write the change as a patch (\`$0 diff $name\`) or drop it" >&2
            exit 1
        fi
    done
    echo "vendor-patches: $name is $(version_of "$queue") plus $(grep -cv '^\s*\(#\|$\)' "$queue/series") patches"
}

case ${1:-} in
verify)
    shift
    if [[ $# -eq 0 ]]; then
        # shellcheck disable=SC2046 # queue names are directory names
        set -- $(ls "$ROOT/patches")
    fi
    for name in "$@"; do
        verify "$(queue_dir "$name")"
    done
    ;;
diff)
    [[ $# -eq 2 ]] || usage
    queue=$(queue_dir "$2")
    pristine "$queue" "$work"
    cd "$work"
    for crate in $(crates_of "$queue"); do
        ln -s "$ROOT/vendor/$crate" "patched-$crate"
        # Headers name both sides by the crate and carry no timestamps, so
        # a regenerated patch differs only where the change does.
        diff -ruN "$crate" "patched-$crate/" | sed -E \
            -e "s#^diff -ruN ([^ ]*) patched-$crate/#diff -ruN \1 $crate/#" \
            -e "s#^(---|[+][+][+]) (patched-)?([^\t]*)\t.*#\1 \3#" || true
    done
    ;;
*)
    usage
    ;;
esac
