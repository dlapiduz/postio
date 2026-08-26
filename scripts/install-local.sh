#!/usr/bin/env bash
# Build Postio from this checkout and install it for the current user:
# the binary on $PATH, the desktop entry in the app grid, the icons in the
# hicolor theme. No root, no packaging — the counterpart for people who just
# built from source and want to launch it like any other app.
#
#   scripts/install-local.sh              # build --release and install
#   scripts/install-local.sh --uninstall  # remove exactly what was installed
#
# Everything lands under ~/.local (or $XDG_DATA_HOME/$PREFIX if set), which
# every desktop following the XDG spec already searches. For the sandboxed
# route, see flatpak/README.md instead.
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
prefix="${PREFIX:-$HOME/.local}"

bin="$prefix/bin/postio"
desktop="$data_home/applications/dev.postio.Postio.desktop"
icon_png="$data_home/icons/hicolor/128x128/apps/dev.postio.Postio.png"
icon_svg="$data_home/icons/hicolor/scalable/apps/dev.postio.Postio.svg"
metainfo="$data_home/metainfo/dev.postio.Postio.metainfo.xml"

refresh_caches() {
    # Best-effort: the spec says these caches are optional, and a desktop
    # without the tools still picks the files up on next login.
    command -v update-desktop-database >/dev/null &&
        update-desktop-database "$data_home/applications" 2>/dev/null || true
    command -v gtk-update-icon-cache >/dev/null &&
        gtk-update-icon-cache -qt "$data_home/icons/hicolor" 2>/dev/null || true
}

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -f "$bin" "$desktop" "$icon_png" "$icon_svg" "$metainfo"
    refresh_caches
    echo "Postio removed from $prefix and $data_home."
    exit 0
fi

echo "Building postio (release) — the first build takes a while..."
cargo build --release --package postio-app --bin postio \
    --manifest-path "$here/Cargo.toml"

# Respect a redirected target dir; cargo names the default one otherwise.
target="${CARGO_TARGET_DIR:-$here/target}"

install -Dm755 "$target/release/postio" "$bin"
install -Dm644 "$here/crates/postio-gtk/data/dev.postio.Postio.desktop" "$desktop"
install -Dm644 "$here/crates/postio-gtk/data/icons/128x128/apps/dev.postio.Postio.png" "$icon_png"
install -Dm644 "$here/crates/postio-gtk/data/icons/scalable/apps/dev.postio.Postio.svg" "$icon_svg"
install -Dm644 "$here/crates/postio-gtk/data/dev.postio.Postio.metainfo.xml" "$metainfo"
refresh_caches

echo "Installed: $bin"
echo "Postio is in your app grid. If \$PATH lacks $prefix/bin, add it or run $bin directly."
