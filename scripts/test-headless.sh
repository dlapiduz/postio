#!/usr/bin/env bash
# Run tests on a display of their own, so GTK stops throwing windows at you: a second compositor
# (`mutter --headless`, already installed), on Wayland like production, unlike Xvfb.
#
# Usage:
#   scripts/test-headless.sh cargo test -p postio-widgets
#   scripts/test-headless.sh cargo test --workspace --no-fail-fast
#   scripts/test-headless.sh --stop        # shut the compositor down
#   scripts/test-headless.sh --status
#   scripts/test-headless.sh --own <command...>   # a compositor that ends with <command>
#   scripts/test-headless.sh --sweep       # stop the own compositors whose runner is gone
#
# The shared compositor is started on demand and left running; --stop when done. An own one, for a
# runner named by its pid (storyboards.sh, the determinism case), ends with it: each leaked one holds
# an inotify instance, and enough of them fail every file-watching test with EMFILE (#2248).
set -euo pipefail

DISPLAY_NAME="${POSTIO_TEST_DISPLAY:-postio-headless}"
SOCKET="${XDG_RUNTIME_DIR:?XDG_RUNTIME_DIR is not set}/$DISPLAY_NAME"
LOG="${XDG_RUNTIME_DIR}/$DISPLAY_NAME.log"
GEOMETRY="${POSTIO_TEST_GEOMETRY:-1280x800}"

running() { [ -S "$SOCKET" ]; }

# Only `postio-storyboard-<pid>[-<n>]` and `postio-determinism-<pid>[-<n>]` on this runtime dir whose
# <pid> is gone: `postio-headless*` belongs to the suites. Asked of mutter's own argv and environment,
# not `pkill -f`, which also matches any shell that mentions the name.
sweep() {
    local pid arg name owner
    for pid in $(pgrep -u "$(id -u)" -x mutter || true); do
        tr '\0' '\n' < "/proc/$pid/environ" 2>/dev/null \
            | grep -qxF "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR" || continue
        for arg in $(tr '\0' '\n' < "/proc/$pid/cmdline" 2>/dev/null); do
            [[ "$arg" =~ ^--wayland-display=(postio-(storyboard|determinism)-([0-9]+)(-[0-9]+)?)$ ]] \
                || continue
            owner="${BASH_REMATCH[3]}"
            [ -d "/proc/$owner" ] || kill -TERM "$pid" 2>/dev/null || true
        done
    done
    for name in "$XDG_RUNTIME_DIR"/postio-storyboard-* "$XDG_RUNTIME_DIR"/postio-determinism-*; do
        [[ "$(basename "$name")" =~ ^postio-(storyboard|determinism)-([0-9]+) ]] || continue
        [ -d "/proc/${BASH_REMATCH[2]}" ] || rm -f "$name"
    done
}

OWN=0
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
    --sweep) sweep; exit 0 ;;
    --own) OWN=1; shift ;;
esac
[ $# -gt 0 ] || { echo "usage: scripts/test-headless.sh [--own] <command...>" >&2; exit 2; }

if ! running; then
    command -v mutter >/dev/null || {
        echo "mutter is not installed; it normally ships with GNOME." >&2
        echo "  sudo dnf install mutter" >&2
        # Not Xvfb: X11 is not a supported configuration, so a suite passing under XWayland proves
        # something we do not ship.
        exit 1
    }
    if [ "$OWN" = 1 ]; then
        sweep
        # Its parent is this shell, which execs into <command> below, so the kernel signals it when
        # <command> ends however it ends, SIGKILL included; a trap cannot see that.
        setpriv --pdeathsig TERM mutter --headless --no-x11 --wayland-display="$DISPLAY_NAME" \
            --virtual-monitor "$GEOMETRY" >"$LOG" 2>&1 < /dev/null &
    else
        # setsid so the compositor survives the Ctrl-C that interrupts a test run; --no-x11 as in
        # headless-runner.sh.
        setsid mutter --headless --no-x11 --wayland-display="$DISPLAY_NAME" \
            --virtual-monitor "$GEOMETRY" >"$LOG" 2>&1 < /dev/null &
    fi
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
