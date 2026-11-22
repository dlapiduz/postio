#!/usr/bin/env bash
# Build Postio from this checkout and install it for the current user: the binary on $PATH, the
# desktop entry, the icons in the hicolor theme.
# No root; everything lands under ~/.local (or $XDG_DATA_HOME/$PREFIX). For the sandboxed route see
# flatpak/README.md.
#
#   scripts/install-local.sh              # build --release and install
#   scripts/install-local.sh --uninstall  # remove exactly what was installed
#
# It names every missing build dependency before compiling; POSTIO_SKIP_DEP_CHECK=1 goes ahead
# regardless.
set -euo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
prefix="${PREFIX:-$HOME/.local}"

bin="$prefix/bin/postio"
desktop="$data_home/applications/dev.postio.Postio.desktop"
icon_svg="$data_home/icons/hicolor/scalable/apps/dev.postio.Postio.svg"
icon_symbolic="$data_home/icons/hicolor/scalable/apps/dev.postio.Postio-symbolic.svg"
# Only the sizes that earn a file: 16 and 32 are drawn (the mark thickens as it shrinks), 128
# satisfies Flatpak's export validation; the scalable SVG serves the rest.
icon_sizes=(16 32 128)
metainfo="$data_home/metainfo/dev.postio.Postio.metainfo.xml"

refresh_caches() {
    # Best-effort. `-f` matters: without it gtk-update-icon-cache keeps a stale icon-theme.cache
    # (#427).
    command -v update-desktop-database >/dev/null &&
        update-desktop-database "$data_home/applications" 2>/dev/null || true
    command -v gtk-update-icon-cache >/dev/null &&
        gtk-update-icon-cache -qtf "$data_home/icons/hicolor" 2>/dev/null || true
}

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -f "$bin" "$desktop" "$icon_svg" "$icon_symbolic" "$metainfo"
    for size in "${icon_sizes[@]}"; do
        rm -f "$data_home/icons/hicolor/${size}x${size}/apps/dev.postio.Postio.png"
    done
    refresh_caches
    echo "Postio removed from $prefix and $data_home."
    exit 0
fi

check_build_dependencies() {
    local missing_libraries=() lines=() library

    if ! command -v pkg-config >/dev/null 2>&1; then
        lines+=("  pkg-config — nothing can find the GTK libraries without it")
    else
        for library in "${PKG_CONFIG_LIBS[@]}"; do
            pkg-config --exists "$library" || missing_libraries+=("$library")
        done
    fi

    if [ ${#missing_libraries[@]} -gt 0 ]; then
        lines+=("  libraries: ${missing_libraries[*]}")
    fi
    if [ ${#lines[@]} -eq 0 ]; then
        return 0
    fi

    {
        echo "Postio cannot be built here yet. Missing:"
        echo
        printf '%s\n' "${lines[@]}"
        echo
        if [ ${#missing_libraries[@]} -gt 0 ]; then
            echo "On Fedora, each library is a \`-devel\` package: see README.md."
        fi
        echo "Elsewhere, install your distribution's package for each name above."
        echo
        echo "README.md (\"System dependencies\") lists the whole set, for Fedora"
        echo "and for Debian/Ubuntu."
        echo
        echo "Set POSTIO_SKIP_DEP_CHECK=1 to build anyway if this check is wrong"
        echo "about your machine."
    } >&2
    exit 1
}

# The three top-level libraries the app links; each names glib, pango and gdk-pixbuf in its
# `Requires:`, so pkg-config resolves those
# transitively. Nothing else: the store engine is pure Rust, so no system sqlite3 or OpenSSL is
# involved (#646).
PKG_CONFIG_LIBS=(gtk4 libadwaita-1 webkitgtk-6.0)

if [ -z "${POSTIO_SKIP_DEP_CHECK:-}" ]; then
    check_build_dependencies
fi

# `.cargo/config.toml` names `postio-linker` and `postio-cc` as bare programs (#1101); a fresh clone
# has neither on PATH.
"$here/scripts/install-shims.sh"

echo "Building postio (release) — the first build takes a while..."
# About says the commit beside the version when told it (postio-gtk's app::COMMIT); a copy without
# .git builds without one.
POSTIO_COMMIT="$(git -C "$here" rev-parse --short=9 HEAD 2>/dev/null || true)" \
cargo build --release --package postio-gtk --bin postio \
    --manifest-path "$here/Cargo.toml"

target="${CARGO_TARGET_DIR:-$here/target}"

install -Dm755 "$target/release/postio" "$bin"
install -Dm644 "$here/crates/postio-gtk/data/dev.postio.Postio.desktop" "$desktop"
install -Dm644 "$here/crates/postio-widgets/data/icons/scalable/apps/dev.postio.Postio.svg" "$icon_svg"
install -Dm644 "$here/crates/postio-widgets/data/icons/scalable/apps/dev.postio.Postio-symbolic.svg" "$icon_symbolic"
for size in "${icon_sizes[@]}"; do
    install -Dm644 \
        "$here/crates/postio-widgets/data/icons/${size}x${size}/apps/dev.postio.Postio.png" \
        "$data_home/icons/hicolor/${size}x${size}/apps/dev.postio.Postio.png"
done
install -Dm644 "$here/crates/postio-gtk/data/dev.postio.Postio.metainfo.xml" "$metainfo"
refresh_caches

echo "Installed: $bin"
echo "Postio is in your app grid. If \$PATH lacks $prefix/bin, add it or run $bin directly."
echo "If the app grid still shows a generic icon, some shells cache icon"
echo "lookups per session and need a fresh login (or a shell restart) to"
echo "pick up a newly installed one."
