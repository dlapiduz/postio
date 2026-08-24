#!/usr/bin/env bash
# Run tests on a display of their own, so GTK stops throwing windows at you.
#
# crates/postio-gtk has ~20 test binaries that present real windows, and on a
# live GNOME session every one of them flashes onto your desktop, steals focus
# mid-keystroke, and leaves you unable to use the machine while the suite runs.
#
# The fix is a second compositor, not a second machine. mutter --headless is
# already installed here (it is GNOME's own), so this costs no new dependency
# and -- unlike Xvfb, which would run the app under XWayland -- the tests keep
# running on Wayland against the same compositor the application targets. A
# test that passes here passes for the same reasons it passes in production.
#
# Usage:
#   scripts/test-headless.sh cargo test -p postio-gtk
#   scripts/test-headless.sh cargo test --workspace --no-fail-fast
#   scripts/test-headless.sh --stop        # shut the compositor down
#   scripts/test-headless.sh --status
#
# The compositor is started on demand and left running, because starting one
# costs about a second and every later invocation reuses it. --stop when you
# are done for the day; nothing breaks if you never do.
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
        rm -f "$SOCKET"
        exit 0 ;;
    --status)
        running && echo "up: $SOCKET" || echo "down"
        exit 0 ;;
    "") echo "usage: scripts/test-headless.sh <command...>" >&2; exit 2 ;;
esac

if ! running; then
    command -v mutter >/dev/null || {
        echo "mutter is not installed; it normally ships with GNOME." >&2
        echo "Fallback: sudo dnf install xorg-x11-server-Xvfb, then" >&2
        echo "  xvfb-run -a env GDK_BACKEND=x11 cargo test -p postio-gtk" >&2
        exit 1
    }
    # setsid so the compositor is not in this shell's process group and
    # survives the Ctrl-C that interrupts a test run.
    setsid mutter --headless --wayland-display="$DISPLAY_NAME" \
        --virtual-monitor "$GEOMETRY" >"$LOG" 2>&1 < /dev/null &
    for _ in $(seq 1 40); do running && break; sleep 0.25; done
    running || { echo "the headless compositor did not come up; see $LOG" >&2; exit 1; }
    echo "started a headless compositor on wayland-display '$DISPLAY_NAME' ($GEOMETRY)"
fi

# GDK_BACKEND pins Wayland so a stray DISPLAY cannot pull the tests back onto
# the real session through XWayland -- which would put every window back on
# your desktop while looking like it had worked.
export WAYLAND_DISPLAY="$DISPLAY_NAME"
export GDK_BACKEND=wayland
unset DISPLAY
export GTK_A11Y="${GTK_A11Y:-none}"    # quiets an at-spi warning with no bus

exec "$@"
