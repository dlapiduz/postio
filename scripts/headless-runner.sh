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
# Note this also fronts `cargo run`. For the application itself use
# scripts/run-isolated.sh, which executes the built binary directly and never
# comes through here.
set -uo pipefail

exec_target() { exec "$@"; }

[ "${POSTIO_HEADLESS:-1}" = "0" ] && exec_target "$@"
[ -n "${XDG_RUNTIME_DIR:-}" ]     || exec_target "$@"
command -v mutter >/dev/null 2>&1 || exec_target "$@"

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
exec "$@"
