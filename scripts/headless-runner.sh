#!/usr/bin/env bash
# Cargo's test runner: put every test binary on a compositor of its own.
#
# Wired up as `runner` in .cargo/config.toml, so plain `cargo test` is headless
# without anyone remembering a wrapper. postio-gtk has ~20 test binaries that
# present real windows; on a live session they land on the maintainer's desktop
# and steal focus, and one of them opened a save dialog that outlived the run.
#
# This sits in front of EVERY binary cargo executes for this target, so its
# first duty is to be invisible:
#
#   * If anything is wrong -- no mutter, no XDG_RUNTIME_DIR, a compositor that
#     will not start -- it execs the binary unchanged. A broken runner must
#     never be a broken test suite.
#   * POSTIO_HEADLESS=0 bypasses it entirely, for when you want to watch a run.
#   * It reuses one compositor across every binary in a run rather than
#     starting twenty.
#
# It fronts `cargo run` too, but passes it through: only cargo's test and
# bench binaries -- the ones named with the 16-hex metadata suffix, like
# deps/gtk_list-0123456789abcdef -- are sent to the compositor. A plain-named
# binary (`cargo run -p postio-app`, an example) is someone launching a
# program to look at it, and gets the real display. #315.
set -uo pipefail

exec_target() { exec "$@"; }

[ "${POSTIO_HEADLESS:-1}" = "0" ] && exec_target "$@"
[ -n "${XDG_RUNTIME_DIR:-}" ]     || exec_target "$@"
command -v mutter >/dev/null 2>&1 || exec_target "$@"

# Only test and bench binaries belong on the hidden compositor. Cargo names
# them with a 16-hex metadata suffix; everything else -- the application via
# `cargo run`, examples -- is exec'd unchanged so it reaches the real display.
SUFFIX="${1##*-}"
case "$SUFFIX" in
????????????????)
    case "$SUFFIX" in
    *[!0-9a-f]*) exec_target "$@" ;;   # 16 chars, but not a hash
    esac
    ;;
*) exec_target "$@" ;;
esac

DISPLAY_NAME="${POSTIO_TEST_DISPLAY:-postio-headless}"
SOCKET="$XDG_RUNTIME_DIR/$DISPLAY_NAME"

if [ ! -S "$SOCKET" ]; then
    # A lock so twenty test binaries starting at once bring up one compositor
    # between them rather than twenty racing to bind the same socket.
    LOCK="$XDG_RUNTIME_DIR/$DISPLAY_NAME.startlock"
    if mkdir "$LOCK" 2>/dev/null; then
        setsid mutter --headless --wayland-display="$DISPLAY_NAME" \
            --virtual-monitor "${POSTIO_TEST_GEOMETRY:-1280x800}" \
            >"$XDG_RUNTIME_DIR/$DISPLAY_NAME.log" 2>&1 </dev/null &
        for _ in $(seq 1 40); do [ -S "$SOCKET" ] && break; sleep 0.25; done
        rmdir "$LOCK" 2>/dev/null || true
    else
        # Someone else is starting it; wait for them rather than racing.
        for _ in $(seq 1 60); do [ -S "$SOCKET" ] && break; sleep 0.25; done
    fi
fi

# Still nothing? Then run on whatever the session has. A test that needs a
# display will skip itself; one that does not is unaffected.
[ -S "$SOCKET" ] || exec_target "$@"

export WAYLAND_DISPLAY="$DISPLAY_NAME"
export GDK_BACKEND=wayland
export GTK_A11Y="${GTK_A11Y:-none}"
unset DISPLAY            # or GDK falls back through XWayland to the real session

# WebKit's DMA-BUF renderer negotiates GPU buffers with the compositor, and
# under a nested headless mutter on a loaded machine that handshake has
# wedged at 0% CPU (#272). Tests do not need GPU-accelerated web rendering;
# the documented escape hatch pins WebKit to its software path. Harmless for
# every binary that never loads WebKit.
export WEBKIT_DISABLE_DMABUF_RENDERER="${WEBKIT_DISABLE_DMABUF_RENDERER:-1}"

case "$(basename "${1:-}")" in
gtk_reader-*|e2e-*|gtk_suite-*|app_suite-*)
    # The binaries that talk to WebKit directly: gtk_reader has hung at
    # least four times, and postio-app's e2e suite builds a full window --
    # reader included -- around a live engine, so it inherits the risk.
    # (#272), holding a gate run hostage until a human killed it. Run
    # it under a watchdog: its own process group (so the WebProcess and
    # NetworkProcess children die with it), a hard deadline, and a dump of
    # kernel-side state before the kill so a hang leaves a diagnosis rather
    # than a mystery.
    LIMIT="${POSTIO_TEST_WATCHDOG:-300}"
    setsid "$@" &
    GROUP=$!
    trap 'kill -9 -"$GROUP" 2>/dev/null' INT TERM
    START=$(date +%s)
    while kill -0 "$GROUP" 2>/dev/null; do
        if [ $(( $(date +%s) - START )) -gt "$LIMIT" ]; then
            {
                echo "postio watchdog: $(basename "$1") exceeded ${LIMIT}s; process tree at kill:"
                for pid in $(pgrep -g "$GROUP"); do
                    echo "  pid $pid $(cat /proc/$pid/comm 2>/dev/null): state=$(awk '{print $3}' /proc/$pid/stat 2>/dev/null) wchan=$(cat /proc/$pid/wchan 2>/dev/null)"
                    for t in /proc/$pid/task/*; do
                        echo "    thread $(basename "$t"): state=$(awk '{print $3}' "$t/stat" 2>/dev/null) wchan=$(cat "$t/wchan" 2>/dev/null)"
                    done
                done
                echo "postio watchdog: killing the process group; raise POSTIO_TEST_WATCHDOG if this was real work."
            } >&2
            kill -9 -"$GROUP" 2>/dev/null
            exit 124
        fi
        sleep 1
    done
    wait "$GROUP"; RC=$?
    # Reap anything that outlived the harness -- the save-dialog and orphaned
    # WebProcess pattern CLAUDE.md records. An empty group is the normal case.
    kill -9 -"$GROUP" 2>/dev/null
    exit "$RC"
    ;;
esac

exec "$@"
