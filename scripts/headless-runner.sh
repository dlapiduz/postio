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

# "The runner ran." Exported before anything can bail out, and on every path
# including the fail-open ones, because the question this answers is not "did a
# compositor start" but "did cargo route this binary through here at all".
#
# Those are different failures and they want different answers (#551). A
# contributor with no mutter should get the skips; a target `.cargo/config.toml`
# forgot to name should get a hard failure. Without a marker the two are
# indistinguishable from inside the test binary, which is how the aarch64 gap
# stayed green. `gtk_display_required.rs` is the reader.
export POSTIO_TEST_RUNNER=headless-runner

# `.cargo/config.toml` points TMPDIR at `target/tmp`, relative to the workspace
# root, to keep rustc's and the linker's scratch off the tmpfs -- and nothing
# creates it. `tempfile` opens a file inside `$TMPDIR` and reports what the OS
# said, so the first temp file in a fresh tree fails with `NotFound` naming a
# `.tmpXXXXXX` nobody wrote, under a `target/` that plainly exists, three
# directories from the config that pointed there. It reads as a bug in whatever
# you just ran (#613).
#
# `issue-claim.sh` already did this for the worktrees it makes (#178, #219).
# Here it covers the trees it does not: a plain `git clone`, and a hand-made
# `git worktree add`.
#
# Up here with the marker, before the bail-outs below, and for the same reason:
# this has to happen on every path including the fail-open ones, because a
# binary that reaches the real display still wants somewhere to write. Fail
# open like the rest of the file -- a directory that cannot be created costs
# the pinning, never the run.
if [ -n "${TMPDIR:-}" ]; then
    mkdir -p "$TMPDIR" 2>/dev/null || true
fi

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
UNAVAILABLE="$XDG_RUNTIME_DIR/$DISPLAY_NAME.unavailable"

# Whether a compositor is actually behind the socket, rather than a socket
# file being present.
#
# These are different questions and the difference is not academic: mutter can
# bind the socket and then exit -- on a machine with no DRM device, which is
# every GitHub-hosted runner -- leaving a file that passes `-S` with nothing
# listening. Committing to that is worse than never starting one, because the
# lines below unset DISPLAY and force GDK_BACKEND=wayland, so the fallback
# display is thrown away too and *every* GTK test skips itself for want of a
# display it actually had. That is what #781 spent a day discovering, and it
# is the opposite of this script's promise to exec the binary unchanged when
# anything is wrong.
# Asked of the socket, not of the process table. `pgrep -f
# wayland-display=<name>` looks like the obvious check and is wrong twice
# over: a compositor for a *different* XDG_RUNTIME_DIR can share the display
# name, and `-f` happily matches any shell whose command line mentions it.
# Both were observed while writing this.
#
# `ss` prints a line only when something is actually listening on that exact
# path. If ss is missing, this cannot tell, and says so by keeping the old
# behaviour rather than inventing an answer.
compositor_alive() {
    command -v ss >/dev/null 2>&1 || return 0
    [ -n "$(ss -lxH src "$SOCKET" 2>/dev/null)" ]
}

# One binary learning the compositor will not come up saves the next twenty
# from waiting ten seconds each to learn it again.
[ -e "$UNAVAILABLE" ] && exec_target "$@"

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

# A socket with nothing behind it is the same situation wearing a disguise.
# Clear it and say so once, so the rest of the run stops paying for the
# attempt, then fall back to the session's own display -- which on CI is the
# Xvfb the workflow started and verified.
if ! compositor_alive; then
    : > "$UNAVAILABLE" 2>/dev/null || true
    rm -f "$SOCKET" 2>/dev/null || true
    echo "postio runner: no compositor behind $SOCKET; using the session's display" >&2
    # Mutter's own account of why, which is written to a log nobody has ever
    # read: the failure is silent by construction, and "the compositor did not
    # come up" is not a diagnosis. Bounded, because a compositor that failed
    # noisily should not bury the test output that follows it.
    if [ -s "$XDG_RUNTIME_DIR/$DISPLAY_NAME.log" ]; then
        echo "postio runner: mutter said --" >&2
        tail -n 15 "$XDG_RUNTIME_DIR/$DISPLAY_NAME.log" | sed 's/^/  /' >&2
    fi
    exec_target "$@"
fi

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
gtk_reader-*|e2e-*|gtk_suite-*|app_suite-*|gtk_editable_dialect-*|gtk_editor*|gtk_composer*|gtk_signature*)
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
