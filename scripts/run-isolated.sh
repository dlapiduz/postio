#!/usr/bin/env bash
# Build and run Postio without touching the shared working tree.
#
# Several Claude sessions edit /home/diego/src/postio continuously, so building
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
#   scripts/run-isolated.sh HEAD --inspect  # with the GTK Inspector attached
#   scripts/run-isolated.sh HEAD --shot     # render a PNG instead of opening
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
for arg in "$@"; do
    case "$arg" in
        --inspect) INSPECT=1 ;;
        --shot) SHOT=1 ;;
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

# A throwaway store. The app resolves both of these (see postio-app/src/paths.rs),
# so nothing it does can reach a real mailbox or a real config.
export XDG_DATA_HOME="$STATE/data"
export XDG_CONFIG_HOME="$STATE/config"

# What passes for observability until postio-b9t.3 lands. There is no
# tracing subscriber yet, so RUST_LOG does nothing — GLib's own channel and a
# backtrace are what we have.
export RUST_BACKTRACE="${RUST_BACKTRACE:-1}"
export G_MESSAGES_DEBUG="${G_MESSAGES_DEBUG:-all}"
export GTK_A11Y="${GTK_A11Y:-none}"     # quiets an at-spi warning on headless
[ "$INSPECT" = 1 ] && export GTK_DEBUG=interactive

cd "$TREE"
if [ "$SHOT" = 1 ]; then
    OUT="$ROOT/shot-$SHA.png"
    cargo run --release -p postio-gtk --example shot -- "$OUT" demo
    echo "wrote $OUT"
    exit 0
fi

echo "building (first run compiles GTK deps; later runs are incremental)…"
cargo build --release -p postio-app
echo "running — Ctrl-C to stop"
exec ./target/release/postio
