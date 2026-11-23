#!/usr/bin/env bash
# Cargo's test runner: put every test binary on a compositor of its own. Wired up as `runner` in
# .cargo/config.toml. It sits in front of EVERY binary cargo executes for this target, so it must be
# invisible: if anything is wrong (no mutter, no XDG_RUNTIME_DIR, a compositor that will not start)
# it execs the binary unchanged. POSTIO_HEADLESS=0 bypasses it. Only test and bench binaries (16-hex
# metadata suffix, like deps/gtk_list-0123456789abcdef) go to the compositor; `cargo run` and
# examples get the real display (#315).
set -uo pipefail

# "The runner ran." Exported on every path, fail-open ones included: it answers whether cargo routed
# this binary here at all, not whether a compositor started (#551). `widgets_suite`'s
# `display_required` reads it.
export POSTIO_TEST_RUNNER=headless-runner

# `.cargo/config.toml` points TMPDIR at `target/tmp` and nothing creates it, so the first `tempfile`
# in a fresh tree fails with an unexplained `NotFound` (#613). Done before the bail-outs, on every
# path; a directory that cannot be created costs the pinning, never the run.
if [ -n "${TMPDIR:-}" ]; then
    mkdir -p "$TMPDIR" 2>/dev/null || true
fi

exec_target() { exec "$@"; }

[ "${POSTIO_HEADLESS:-1}" = "0" ] && exec_target "$@"
[ -n "${XDG_RUNTIME_DIR:-}" ]     || exec_target "$@"
command -v mutter >/dev/null 2>&1 || exec_target "$@"

# `shot` and `surface` render a PNG and exit; on the session display they depended on whether the
# screen was awake (#809), so they use the private compositor.
case "$(basename "${1:-}")" in
shot|surface) ;;
*)
    # Only test and bench binaries (16-hex metadata suffix) belong on the hidden compositor;
    # everything else is exec'd unchanged.
    SUFFIX="${1##*-}"
    case "$SUFFIX" in
    ????????????????)
        case "$SUFFIX" in
        *[!0-9a-f]*) exec_target "$@" ;;   # 16 chars, but not a hash
        esac
        ;;
    *) exec_target "$@" ;;
    esac
    ;;
esac

# One compositor per nextest slot unless a display is pinned (#2027): a shared one ran at 85% of a
# core and its queue caused the suite's flakes; per slot the same run took 191 s instead of 278 s.
DISPLAY_NAME="${POSTIO_TEST_DISPLAY:-postio-headless${NEXTEST_TEST_GLOBAL_SLOT:+-$NEXTEST_TEST_GLOBAL_SLOT}}"
SOCKET="$XDG_RUNTIME_DIR/$DISPLAY_NAME"
UNAVAILABLE="$XDG_RUNTIME_DIR/$DISPLAY_NAME.unavailable"

# Whether a compositor is behind the socket, not just a socket file: mutter can bind and exit (no
# DRM device, as on every GitHub runner), and committing to that unsets DISPLAY so every GTK test
# skips (#781). Asked of the socket, not the process table: `pgrep -f` matches other
# XDG_RUNTIME_DIRs and any shell mentioning the name. Without `ss` the old behaviour is kept.
compositor_alive() {
    command -v ss >/dev/null 2>&1 || return 0
    [ -n "$(ss -lxH src "$SOCKET" 2>/dev/null)" ]
}

# One binary learning the compositor will not come up saves the next twenty from waiting. The marker
# expires: XDG_RUNTIME_DIR lives as long as the login session, and a permanent one would demote
# every later suite to the session's display.
if [ -e "$UNAVAILABLE" ]; then
    # `find -mmin +N` prints the file only when it is older than N minutes.
    if [ -z "$(find "$UNAVAILABLE" -mmin +"${POSTIO_UNAVAILABLE_MINUTES:-5}" 2>/dev/null)" ]; then
        exec_target "$@"
    fi
    rm -f "$UNAVAILABLE" 2>/dev/null || true
fi

if [ ! -S "$SOCKET" ]; then
    # A lock so twenty test binaries starting at once bring up one compositor
    # between them rather than twenty racing to bind the same socket.
    LOCK="$XDG_RUNTIME_DIR/$DISPLAY_NAME.startlock"
    if mkdir "$LOCK" 2>/dev/null; then
        # `GTK_A11Y`/`NO_AT_BRIDGE` here, since the export below runs after mutter starts: the
        # a11y-bus warning is the compositor's, once per window.
        # --no-x11: mutter will not start once it has tried 50 X display lock files in use.
        GTK_A11Y=none NO_AT_BRIDGE=1 \
        setsid mutter --headless --no-x11 --wayland-display="$DISPLAY_NAME" \
            --virtual-monitor "${POSTIO_TEST_GEOMETRY:-1280x800}" \
            >"$XDG_RUNTIME_DIR/$DISPLAY_NAME.log" 2>&1 </dev/null &
        # Ten seconds idle, scaled by POSTIO_TEST_PATIENCE: a slow mutter start is ordinary, and
        # giving up early skips every test for want of a display.
        TICKS=$(awk -v factor="${POSTIO_TEST_PATIENCE:-1}" \
            'BEGIN { if (factor + 0 <= 0) factor = 1; printf "%d", 40 * factor }')
        for _ in $(seq 1 "$TICKS"); do [ -S "$SOCKET" ] && break; sleep 0.25; done
        rmdir "$LOCK" 2>/dev/null || true
    else
        WAIT=$(awk -v factor="${POSTIO_TEST_PATIENCE:-1}" \
            'BEGIN { if (factor + 0 <= 0) factor = 1; printf "%d", 60 * factor }')
        for _ in $(seq 1 "$WAIT"); do [ -S "$SOCKET" ] && break; sleep 0.25; done
    fi
fi

# Still nothing: run on the session's display, out loud, with a marker so the next twenty binaries
# do not each wait. A test needing a display skips itself.
if [ ! -S "$SOCKET" ]; then
    : > "$UNAVAILABLE" 2>/dev/null || true
    echo "postio runner: mutter never bound $SOCKET; using the session's display" >&2
    if [ -s "$XDG_RUNTIME_DIR/$DISPLAY_NAME.log" ]; then
        echo "postio runner: mutter said --" >&2
        tail -n 15 "$XDG_RUNTIME_DIR/$DISPLAY_NAME.log" | sed 's/^/  /' >&2
    fi
    exec_target "$@"
fi

# A socket with nothing behind it: clear it, say so once, and fall back to the session's display.
if ! compositor_alive; then
    : > "$UNAVAILABLE" 2>/dev/null || true
    rm -f "$SOCKET" 2>/dev/null || true
    echo "postio runner: no compositor behind $SOCKET; using the session's display" >&2
    # Mutter's own log says why; bounded so it cannot bury the test output.
    if [ -s "$XDG_RUNTIME_DIR/$DISPLAY_NAME.log" ]; then
        echo "postio runner: mutter said --" >&2
        tail -n 15 "$XDG_RUNTIME_DIR/$DISPLAY_NAME.log" | sed 's/^/  /' >&2
    fi
    exec_target "$@"
fi

export WAYLAND_DISPLAY="$DISPLAY_NAME"
export GDK_BACKEND=wayland
# A shader cache that outlives the case (#2027): the Focus suite re-executes with a fresh
# XDG_CACHE_HOME, so each case recompiled Mesa's shaders (~0.55 s of 1.5 s).
export MESA_SHADER_CACHE_DIR="${MESA_SHADER_CACHE_DIR:-$XDG_RUNTIME_DIR/postio-mesa-shader-cache}"
export GTK_A11Y="${GTK_A11Y:-none}"
unset DISPLAY            # or GDK falls back through XWayland to the real session

# WebKit's DMA-BUF renderer has wedged at 0% CPU against a nested headless mutter (#272); software
# rendering is harmless for tests.
export WEBKIT_DISABLE_DMABUF_RENDERER="${WEBKIT_DISABLE_DMABUF_RENDERER:-1}"

case "$(basename "${1:-}")" in
e2e-*|widgets_suite-*|focus_suite-*)
    # The WebKit binaries (shared widgets' suite, desktop app) run under a watchdog: own process
    # group, hard deadline, kernel-state dump before the kill (#272). 900s, scaled by
    # POSTIO_TEST_PATIENCE (#842) with `awk` (POSIX sh, fractional). It bounds a hang, which never
    # finishes, so a generous limit costs only how long a wedged run takes to fail; 300 killed a
    # healthy suite at 142 cases.
    LIMIT="${POSTIO_TEST_WATCHDOG:-900}"
    LIMIT=$(awk -v base="$LIMIT" -v factor="${POSTIO_TEST_PATIENCE:-1}" \
        'BEGIN { if (factor + 0 <= 0) factor = 1; printf "%d", base * factor }')
    setsid "$@" &
    GROUP=$!
    trap 'kill -9 -"$GROUP" 2>/dev/null' INT TERM
    # The deadline is a timer beside the child, so `wait` returns the moment the child does (a poll
    # loop cost every binary up to a second).
    # The timer counts whole-second sleeps with descriptors closed and is stopped by SIGKILL; the
    # marker file tells this shell the timer, not the test, ended the run (137 cannot).
    FIRED="${TMPDIR:-/tmp}/postio-watchdog.$$"
    rm -f "$FIRED"
    (
        for _ in $(seq 1 "$LIMIT"); do sleep 1 </dev/null >/dev/null 2>&1; done
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
        : > "$FIRED"
        kill -9 -"$GROUP" 2>/dev/null
    ) &
    TIMER=$!
    while :; do
        wait "$GROUP"; RC=$?
        kill -0 "$GROUP" 2>/dev/null || break
    done
    kill -9 "$TIMER" 2>/dev/null
    wait "$TIMER" 2>/dev/null
    if [ -e "$FIRED" ]; then
        rm -f "$FIRED"
        kill -9 -"$GROUP" 2>/dev/null
        exit 124
    fi
    kill -9 -"$GROUP" 2>/dev/null
    exit "$RC"
    ;;
esac

exec "$@"
