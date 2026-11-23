#!/usr/bin/env bash
# Run tests on a display of their own, so GTK stops throwing windows at you: a second compositor
# (`mutter --headless`, already installed), on Wayland like production, unlike Xvfb.
#
# Usage:
#   scripts/test-headless.sh cargo test -p postio-widgets
#   scripts/test-headless.sh cargo test --workspace --no-fail-fast
#   scripts/test-headless.sh --stop        # shut the compositor down
#   scripts/test-headless.sh --status
#
# The compositor is started on demand and left running; --stop when done.
set -euo pipefail

DISPLAY_NAME="${POSTIO_TEST_DISPLAY:-postio-headless}"
SOCKET="${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is not set}/$DISPLAY_NAME"
LOG="${XDG_RUNTIME_DIR}/$DISPLAY_NAME.log"
GEOMETRY="${POSTIO_TEST_GEOMETRY:-1280x800}"

running() { [ -S "$SOCKET" ]; }

case "${1:-}" in
    --stop)
        pkill -f "mutter --headless --wayland-display=$DISPLAY_NAME" 2>/dev/null \
            && echo "stopped the headless compositor" \
            || echo "no headless compositor was running"
        rm -f "$SOCKET" "$SOCKET"-[0-9]*
        # And the runner's "do not bother trying" marker: --stop is what someone runs when the
        # compositor is wedged, so it must leave nothing telling the next run to skip.
        rm -f "$XDG_RUNTIME_DIR/$DISPLAY_NAME.unavailable" \
            "$XDG_RUNTIME_DIR/$DISPLAY_NAME"-[0-9]*.unavailable
        exit 0 ;;
    --status)
        running && echo "up: $SOCKET" || echo "down"
        exit 0 ;;
    "") echo "usage: scripts/test-headless.sh <command...>" >&2; exit 2 ;;
esac

if ! running; then
    command -v mutter >/dev/null || {
        echo "mutter is not installed; it normally ships with GNOME." >&2
        echo "  sudo dnf install mutter" >&2
        # Not Xvfb: X11 is not a supported configuration, so a suite passing under XWayland proves
        # something we do not ship.
        exit 1
    }
    # setsid so the compositor survives the Ctrl-C that interrupts a test run; --no-x11 as in
    # headless-runner.sh.
    setsid mutter --headless --no-x11 --wayland-display="$DISPLAY_NAME" \
        --virtual-monitor "$GEOMETRY" >"$LOG" 2>&1 < /dev/null &
    for _ in $(seq 1 40); do running && break; sleep 0.25; done
    running || { echo "the headless compositor did not come up; see $LOG" >&2; exit 1; }
    echo "started a headless compositor on wayland-display '$DISPLAY_NAME' ($GEOMETRY)"
fi

# GDK_BACKEND pins Wayland so a stray DISPLAY cannot pull the tests back onto the real session
# through XWayland.
export WAYLAND_DISPLAY="$DISPLAY_NAME"
export GDK_BACKEND=wayland
unset DISPLAY
export GTK_A11Y="${GTK_A11Y:-none}"    # quiets an at-spi warning with no bus

# RUSTUP_TOOLCHAIN in the environment beats rust-toolchain.toml; this wrapper fronts every `cargo
# test` a session runs, so the guarantee lives here (#112).
unset RUSTUP_TOOLCHAIN

exec "$@"
