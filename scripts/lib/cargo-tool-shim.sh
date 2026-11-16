#!/usr/bin/env bash
# The `cargo-nextest` / `cargo-hakari` / `cargo-machete` / `cargo-deny` that
# sits in ~/.cargo/bin: not the tool, a resolver for it. It is copied there
# under each tool's name by `scripts/lib/pinned-tool.sh` (the install
# scripts call it), and picks its job from `basename "$0"`.
#
# It reads the pin from the `scripts/install-<tool>.sh` of the repository the
# caller is standing in and runs exactly that version from
# ~/.local/share/postio-tools/<tool>/<version>/bin/, installing it first
# when it is not there yet. Outside a Postio checkout it runs the newest
# version installed. See `scripts/lib/pinned-tool.sh` for why.
set -euo pipefail

name=$(basename "$0")
case "$name" in
cargo-nextest) script=install-nextest.sh ;;
cargo-hakari) script=install-hakari.sh ;;
cargo-machete) script=install-machete.sh ;;
cargo-deny) script=install-cargo-deny.sh ;;
*)
    echo "$name: tool shim installed under a name it does not know" >&2
    exit 127
    ;;
esac

store="${POSTIO_TOOLS_DIR:-$HOME/.local/share/postio-tools}/$name"

# Nearest enclosing checkout that carries the pin.
pin=""
dir=$PWD
while :; do
    if [ -f "$dir/scripts/$script" ]; then
        pin="$dir/scripts/$script"
        break
    fi
    [ "$dir" = / ] && break
    dir=$(dirname "$dir")
done

if [ -n "$pin" ]; then
    version=$(sed -n 's/^VERSION="\([^"]*\)".*/\1/p' "$pin" | head -1)
    if [ -z "$version" ]; then
        echo "$name: no VERSION=\"...\" line in $pin" >&2
        exit 1
    fi
    if [ ! -x "$store/$version/bin/$name" ]; then
        echo "$name: installing $version, pinned by $pin" >&2
        # stdout belongs to the tool's caller (`cargo nextest list` is parsed).
        "$pin" >&2
    fi
else
    version=$(ls "$store" 2>/dev/null | sort -V | tail -1 || true)
    if [ -z "$version" ]; then
        echo "$name: no installed version and no Postio checkout here;" >&2
        echo "  run scripts/$script from a checkout" >&2
        exit 1
    fi
fi

exec "$store/$version/bin/$name" "$@"
