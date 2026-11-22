#!/usr/bin/env bash
# Build and run Postio without touching the shared working tree: a worktree pinned to a commit, with
# its own target directory and XDG dirs, so the running app is a known commit, `cargo build` cannot
# poison the shared target/ (stale artifacts with a baked-in CARGO_MANIFEST_DIR once caused nine
# unrelated test failures), and the app reads and writes a throwaway store.
#
# Usage:
#   scripts/run-isolated.sh                 # build and run HEAD
#   scripts/run-isolated.sh <commit>        # build and run a specific commit
#   POSTIO_LOG=debug scripts/run-isolated.sh # with a readable trace of the sync
#   scripts/run-isolated.sh HEAD --inspect  # with the GTK Inspector attached
#   scripts/run-isolated.sh HEAD --shot     # render a PNG instead of opening
#   scripts/run-isolated.sh HEAD --provision  # add a real account to the scratch store
#   scripts/run-isolated.sh HEAD --install-desktop  # also give Postio its dock icon (see below)
#   scripts/run-isolated.sh HEAD --reset-store  # set the scratch store aside, start a fresh one
#   scripts/run-isolated.sh --clean         # discard the worktree and store
#
# The store lives under $ROOT/state and persists between runs, so a synced mailbox is still there
# next time. --clean removes it.
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
INSTALL_DESKTOP=0
RESET_STORE=0
for arg in "$@"; do
    case "$arg" in
        --inspect) INSPECT=1 ;;
        --shot) SHOT=1 ;;
        --provision) PROVISION=1 ;;
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

export CARGO_TARGET_DIR="$TARGET"

# A throwaway store, so nothing can reach a real mailbox, config or state (paths:
# postio-config/src/paths.rs, postio-widgets/src/state.rs, postio-ui/src/allowlist.rs).
export XDG_DATA_HOME="$STATE/data"
export XDG_CONFIG_HOME="$STATE/config"
# XDG_STATE_HOME was missed until #215: `remote-images.ini` there is the standing "always allow
# images" list, and a click while viewing the demo store wrote a real exception that changed what
# the GTK tests saw.
export XDG_STATE_HOME="$STATE/state"

# POSTIO_LOG takes an EnvFilter directive (`debug`, or `postio_sync=debug,postio_runtime=debug`);
# `[logging]` in the scratch config.toml does the same to a running instance.
# G_MESSAGES_DEBUG=all is not set: it produced hundreds of lines of Vulkan and portal settings and
# none about mail.
export POSTIO_LOG="${POSTIO_LOG:-info}"
export RUST_BACKTRACE="${RUST_BACKTRACE:-1}"
export GTK_A11Y="${GTK_A11Y:-none}"     # quiets an at-spi warning on headless
[ "$INSPECT" = 1 ] && export GTK_DEBUG=interactive

cd "$TREE"
[ -x scripts/install-shims.sh ] && scripts/install-shims.sh
[ -x scripts/jobserver.sh ] && eval "$(scripts/jobserver.sh env 2>/dev/null || true)"
if [ "$PROVISION" = 1 ]; then
    # Writes into $STATE, not your real store, because XDG_DATA_HOME is set above; the password
    # comes from the environment and is never echoed.
    if [ -z "${POSTIO_ADDRESS:-}" ] || [ -z "${POSTIO_APP_PASSWORD:-}" ]; then
        echo "set POSTIO_ADDRESS, and read the app password without putting it in" >&2
        echo "your shell history:  read -rs POSTIO_APP_PASSWORD && export POSTIO_APP_PASSWORD" >&2
        exit 2
    fi
    exec cargo run --release -p postio-session --bin postio-provision
fi

# A scratch store an earlier build wrote at a schema this one cannot carry forward: set it aside
# (state/data/postio/set-aside/<when>/) and start fresh with the accounts carried across.
# `postio-store status` says first whether this is needed.
if [ "$RESET_STORE" = 1 ]; then
    cargo run --release -p postio-session --bin postio-store -- status
    exec cargo run --release -p postio-session --bin postio-store -- reset
fi

{
    if [ "$SHOT" = 1 ]; then
        OUT="$ROOT/shot-$SHA.png"
        cargo run --release -p postio-gtk --example shot -- "$OUT" 01
        echo "wrote $OUT"
        exit 0
    fi
    # --install-desktop (T217): a Wayland dock shows the icon of the desktop entry matched to the
    # window's app id, and a plain cargo build installs none.
    # This writes Postio's entry and icon under your home with Exec= on the isolated binary. Opt-in:
    # it writes outside $ROOT.
    if [ "$INSTALL_DESKTOP" = 1 ]; then
        DATA="${HOME}/.local/share"
        APPS="$DATA/applications"
        ICONS="$DATA/icons/hicolor"
        ID=dev.postio.Postio
        mkdir -p "$APPS" "$ICONS/scalable/apps" "$ICONS/symbolic/apps"
        sed "s|^Exec=.*|Exec=$TARGET/release/postio %U|" \
            "$TREE/crates/postio-gtk/data/$ID.desktop" > "$APPS/$ID.desktop"
        install -m644 "$TREE/crates/postio-widgets/data/icons/scalable/apps/dev.postio.Postio.svg" \
            "$ICONS/scalable/apps/dev.postio.Postio.svg"
        install -m644 "$TREE/crates/postio-widgets/data/icons/scalable/apps/dev.postio.Postio-symbolic.svg" \
            "$ICONS/symbolic/apps/dev.postio.Postio-symbolic.svg"
        command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" || true
        command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$ICONS" || true
        echo "installed $APPS/$ID.desktop and the Postio icon under $ICONS"
        echo "undo: rm $APPS/$ID.desktop $ICONS/scalable/apps/dev.postio.Postio.svg $ICONS/symbolic/apps/dev.postio.Postio-symbolic.svg"
    fi
    echo "building Postio (first run compiles GTK deps; later runs are incremental)…"
    # The attachment text helper is found beside the executable and cargo builds only a package's
    # own binaries; without it every PDF is recorded `skipped` (#1824).
    cargo build --release -p postio-gtk --bin postio \
        -p postio-session --bin postio-extract-helper
    echo "running — Ctrl-C to stop"
    exec "$TARGET/release/postio"
}
