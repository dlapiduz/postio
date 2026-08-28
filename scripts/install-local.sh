#!/usr/bin/env bash
# Build Postio from this checkout and install it for the current user:
# the binary on $PATH, the desktop entry in the app grid, the icons in the
# hicolor theme. No root, no packaging — the counterpart for people who just
# built from source and want to launch it like any other app.
#
#   scripts/install-local.sh              # build --release and install
#   scripts/install-local.sh --uninstall  # remove exactly what was installed
#
# It checks the build dependencies before starting the compile and names every
# missing one at once; POSTIO_SKIP_DEP_CHECK=1 goes ahead regardless.
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
    #
    # `-f`/`--force` matters: without it, gtk-update-icon-cache treats an
    # already-present icon-theme.cache as done rather than rebuilding it, so
    # a cache left over from an earlier run (or another app sharing this
    # user's hicolor theme) went on serving whatever it indexed before
    # Postio's icon existed there (#427). `-t` still skips demanding an
    # index.theme in this directory, which a user theme override never has.
    command -v update-desktop-database >/dev/null &&
        update-desktop-database "$data_home/applications" 2>/dev/null || true
    command -v gtk-update-icon-cache >/dev/null &&
        gtk-update-icon-cache -qtf "$data_home/icons/hicolor" 2>/dev/null || true
}

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -f "$bin" "$desktop" "$icon_png" "$icon_svg" "$metainfo"
    refresh_caches
    echo "Postio removed from $prefix and $data_home."
    exit 0
fi

check_build_dependencies() {
    local missing_modules=() missing_libraries=() lines=() module library

    if ! command -v perl >/dev/null 2>&1; then
        lines+=("  perl itself — OpenSSL's \`Configure\` is a perl program")
    else
        for module in "${PERL_MODULES[@]}"; do
            perl "-M$module" -e1 >/dev/null 2>&1 || missing_modules+=("$module")
        done
    fi

    if ! command -v pkg-config >/dev/null 2>&1; then
        lines+=("  pkg-config — nothing can find the GTK libraries without it")
    else
        for library in "${PKG_CONFIG_LIBS[@]}"; do
            pkg-config --exists "$library" || missing_libraries+=("$library")
        done
    fi

    if [ ${#missing_modules[@]} -gt 0 ]; then
        lines+=("  perl modules: ${missing_modules[*]}")
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
        if [ ${#missing_modules[@]} -gt 0 ]; then
            echo "The perl modules are for OpenSSL: the store is SQLCipher and its"
            echo "OpenSSL is compiled from source (ADR 0014), by a \`Configure\`"
            echo "written in perl. Distributions that split the perl standard"
            echo "library into packages ship none of them by default."
            echo
            # Fedora's package for a perl module is its name with `::`
            # hyphenated, which is mechanical enough to print. Every other
            # distribution has its own spelling, so this says whose command it is.
            local packages=()
            for module in "${missing_modules[@]}"; do
                packages+=("perl-${module//::/-}")
            done
            echo "On Fedora: sudo dnf install ${packages[*]}"
        fi
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

# Checked here rather than discovered by the compiler, because the compiler
# discovers them one build at a time: OpenSSL's `Configure` stops at the first
# `Can't locate X.pm in @INC` it hits, several minutes into a release build,
# and the next module is only found by the next build. Six modules were six
# failed builds (#646). Probing costs milliseconds and answers in one go.
PERL_MODULES=(FindBin IPC::Cmd Pod::Html Digest::SHA Text::Template Time::Piece)
# The three top-level libraries the app links. Each names glib, pango and
# gdk-pixbuf in its own `Requires:`, so pkg-config resolves those transitively
# — listing them here would only add names a distribution might spell
# differently. SQLite is deliberately absent: rusqlite bundles SQLCipher, so
# no system sqlite3 is involved in a build.
PKG_CONFIG_LIBS=(gtk4 libadwaita-1 webkitgtk-6.0)

if [ -z "${POSTIO_SKIP_DEP_CHECK:-}" ]; then
    check_build_dependencies
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
echo "If the app grid still shows a generic icon, some shells cache icon"
echo "lookups per session and need a fresh login (or a shell restart) to"
echo "pick up a newly installed one."
