#!/usr/bin/env bash
# Build and run Postio without touching the shared working tree.
#
# Several Claude sessions edit ~/src/postio continuously, so building
# from it gives you whatever half-finished state happened to be on disk. This
# builds from a git worktree pinned to a commit instead, with its own target
# directory and its own XDG dirs, so:
#
#   * the running app is a known commit, not a moving tree;
#   * `cargo build` here cannot poison the shared target/ — a session once
#     built a copy of the repo elsewhere and left artifacts whose baked-in
#     CARGO_MANIFEST_DIR pointed at a directory that no longer existed, which
#     surfaced as nine unrelated test failures;
#   * the app reads and writes a throwaway store, so nothing here can damage
#     real mail or a real account.
#
# Usage:
#   scripts/run-isolated.sh                 # build and run HEAD
#   scripts/run-isolated.sh <commit>        # build and run a specific commit
#   POSTIO_LOG=debug scripts/run-isolated.sh # with a readable trace of the sync
#   scripts/run-isolated.sh HEAD --inspect  # with the GTK Inspector attached
#   scripts/run-isolated.sh HEAD --shot     # render a PNG instead of opening
#   scripts/run-isolated.sh HEAD --provision  # add a real account to the scratch store
#   scripts/run-isolated.sh HEAD --focus    # run Postio Focus instead (spec 007)
#   scripts/run-isolated.sh HEAD --focus --shot  # render Focus's screen 01 to a PNG
#   scripts/run-isolated.sh HEAD --focus --install-desktop  # also give Focus its dock icon (see below)
#   scripts/run-isolated.sh HEAD --reset-store  # set the scratch store aside, start a fresh one
#   scripts/run-isolated.sh --clean         # discard the worktree and store
#
# The store lives under $ROOT/state and persists between runs, so a synced
# mailbox is still there next time. --clean removes it.
set -euo pipefail

REPO=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
ROOT="${POSTIO_RUN_ROOT:-$HOME/scratch/postio-run}"
TREE="$ROOT/tree"
STATE="$ROOT/state"
TARGET="$ROOT/target"

if [ "${1:-}" = "--clean" ]; then
    git -C "$REPO" worktree remove --force "$TREE" 2>/dev/null || true
    rm -rf "$ROOT"
    echo "removed $ROOT"
    exit 0
fi

COMMIT="${1:-HEAD}"
shift || true
INSPECT=0
SHOT=0
PROVISION=0
FOCUS=0
INSTALL_DESKTOP=0
RESET_STORE=0
for arg in "$@"; do
    case "$arg" in
        --inspect) INSPECT=1 ;;
        --shot) SHOT=1 ;;
        --provision) PROVISION=1 ;;
        --focus) FOCUS=1 ;;
        --install-desktop) INSTALL_DESKTOP=1 ;;
        --reset-store) RESET_STORE=1 ;;
        *) echo "unknown option: $arg" >&2; exit 2 ;;
    esac
done

SHA=$(git -C "$REPO" rev-parse --short "$COMMIT")

mkdir -p "$ROOT"
if [ -d "$TREE" ]; then
    git -C "$TREE" checkout --detach --force "$SHA" >/dev/null 2>&1
else
    git -C "$REPO" worktree add --detach "$TREE" "$SHA" >/dev/null
fi
echo "tree:   $TREE @ $SHA  $(git -C "$REPO" log -1 --format=%s "$SHA")"
echo "state:  $STATE"
echo "target: $TARGET"

mkdir -p "$STATE/data" "$STATE/config"

# Its own target directory. Sharing the repo's would both contend with the
# sessions building there and risk the stale-artifact failure described above.
export CARGO_TARGET_DIR="$TARGET"

# A throwaway store, so nothing the app does can reach a real mailbox, a real
# config, or the state the real Postio keeps. Config is resolved by
# postio-config/src/paths.rs; the state files by glib::user_state_dir() in
# postio-gtk/src/state.rs and postio-gtk/src/reader/allowlist.rs.
export XDG_DATA_HOME="$STATE/data"
export XDG_CONFIG_HOME="$STATE/config"
# XDG_STATE_HOME was missed until #215, and it is not only window geometry:
# $XDG_STATE_HOME/postio/remote-images.ini is the standing "always allow
# images from this sender" list. Clicking that once while looking at the demo
# store wrote a real exception into the real file -- which then decided what
# postio-gtk's tests saw, because a Window builds a Reader that loads it. That
# cost a p1 nobody could bisect, since the cause was never in the tree.
export XDG_STATE_HOME="$STATE/state"

# Observability. POSTIO_LOG takes an EnvFilter directive, so `debug` turns
# everything up and `postio_sync=debug,postio_runtime=debug` turns up just the
# half that talks to a server. `[logging]` in the scratch config.toml does the
# same thing to an already-running instance.
#
# G_MESSAGES_DEBUG=all is deliberately *not* set: it produced two hundred lines
# of Vulkan and portal settings and not one line about mail, which is what made
# the first live run undiagnosable. Set it yourself when the problem is
# actually GTK's.
export POSTIO_LOG="${POSTIO_LOG:-info}"
export RUST_BACKTRACE="${RUST_BACKTRACE:-1}"
export GTK_A11Y="${GTK_A11Y:-none}"     # quiets an at-spi warning on headless
[ "$INSPECT" = 1 ] && export GTK_DEBUG=interactive

cd "$TREE"
# The linker and CC in .cargo/config.toml are names on PATH, not paths (#1101).
[ -x scripts/install-shims.sh ] && scripts/install-shims.sh
# Draw compile jobs from the machine-wide pool rather than `jobs = 2` (#1104).
[ -x scripts/jobserver.sh ] && eval "$(scripts/jobserver.sh env 2>/dev/null || true)"
if [ "$PROVISION" = 1 ]; then
    # Writes into $STATE, not your real store, because XDG_DATA_HOME is set
    # above. The password comes from the environment and is never echoed.
    if [ -z "${POSTIO_ADDRESS:-}" ] || [ -z "${POSTIO_APP_PASSWORD:-}" ]; then
        echo "set POSTIO_ADDRESS, and read the app password without putting it in" >&2
        echo "your shell history:  read -rs POSTIO_APP_PASSWORD && export POSTIO_APP_PASSWORD" >&2
        exit 2
    fi
    exec cargo run --release -p postio-session --bin postio-provision
fi

# A scratch store an earlier build wrote at a schema this one cannot carry
# forward: set it aside (state/data/postio/set-aside/<when>/, not deleted)
# and start a fresh one with the accounts carried across. config.toml and the
# keyring are untouched; the next run syncs the mail again. `postio-store
# status` says first whether this is needed -- a store a migration reaches
# is carried forward on open, and needs nothing.
if [ "$RESET_STORE" = 1 ]; then
    cargo run --release -p postio-session --bin postio-store -- status
    exec cargo run --release -p postio-session --bin postio-store -- reset
fi

# Postio Focus (spec 007) is a second app on the same store: the same
# scratch store and XDG dirs, so an account provisioned above is there too,
# and one app holds the store at a time (ADR 0041).
if [ "$FOCUS" = 1 ]; then
    if [ "$SHOT" = 1 ]; then
        OUT="$ROOT/focus-shot-$SHA.png"
        cargo run --release -p postio-focus --example shot -- "$OUT" 01
        echo "wrote $OUT"
        exit 0
    fi
    # --- --install-desktop (T217) -----------------------------------------
    # The binary draws the Postio icon in its own window (a bundled icon theme
    # and a default icon name), but a dock or window switcher on Wayland
    # (COSMIC, GNOME) shows the icon of the *desktop entry* the compositor
    # matches to the window's app id, and a plain cargo build installs none.
    # This puts Focus's entry and the package's icon where the session looks,
    # under your home only, with Exec= on the isolated binary. Opt-in: it
    # writes outside $ROOT. Remove with the two rm lines it prints.
    if [ "$INSTALL_DESKTOP" = 1 ]; then
        DATA="${HOME}/.local/share"
        APPS="$DATA/applications"
        ICONS="$DATA/icons/hicolor"
        ID=dev.postio.Postio.Focus
        mkdir -p "$APPS" "$ICONS/scalable/apps" "$ICONS/symbolic/apps"
        sed "s|^Exec=.*|Exec=$TARGET/release/postio-focus %U|" \
            "$TREE/crates/postio-focus/data/$ID.desktop" > "$APPS/$ID.desktop"
        install -m644 "$TREE/crates/postio-widgets/data/icons/scalable/apps/dev.postio.Postio.svg" \
            "$ICONS/scalable/apps/dev.postio.Postio.svg"
        install -m644 "$TREE/crates/postio-widgets/data/icons/scalable/apps/dev.postio.Postio-symbolic.svg" \
            "$ICONS/symbolic/apps/dev.postio.Postio-symbolic.svg"
        command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" || true
        command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$ICONS" || true
        echo "installed $APPS/$ID.desktop and the Postio icon under $ICONS"
        echo "undo: rm $APPS/$ID.desktop $ICONS/scalable/apps/dev.postio.Postio.svg $ICONS/symbolic/apps/dev.postio.Postio-symbolic.svg"
    fi
    # --- end --install-desktop ---------------------------------------------
    echo "building Postio Focus (first run compiles GTK deps; later runs are incremental)…"
    cargo build --release -p postio-focus
    echo "running — Ctrl-C to stop"
    exec "$TARGET/release/postio-focus"
fi

if [ "$SHOT" = 1 ]; then
    OUT="$ROOT/shot-$SHA.png"
    cargo run --release -p postio-app --example shot -- "$OUT" demo
    echo "wrote $OUT"
    exit 0
fi

echo "building (first run compiles GTK deps; later runs are incremental)…"
cargo build --release -p postio-app
echo "running — Ctrl-C to stop"
exec "$TARGET/release/postio"
