#!/usr/bin/env bash
# One cargo jobserver for the whole machine.
#
# `.cargo/config.toml` capped every cargo at `jobs = 2` because four sessions
# once shared this eight-core box and starved it. The transcripts say four is
# the rare case: while anything was building, ONE session was building 60% of
# the time and two 26%, so most of the day six cores sat idle -- and only 3
# commands in the whole history ever raised `-j`. #1104.
#
# A jobserver is the fix cargo already understands. GNU make invented it: a
# pipe holding N tokens, and every build process that wants to run a job
# reads one first and writes it back when done. Cargo joins one when
# `MAKEFLAGS=--jobserver-auth=fifo:<path>` is in its environment, hands it
# down to rustc (codegen units) and to build scripts, and ignores `jobs`
# while it has one. So a lone session gets the whole box, four sessions
# share the same ceiling as before, and the ceiling is on the *machine*, not
# on sessions times two -- which is also what bounds memory.
#
# Verified on cargo 1.98 before this was written: a six-crate build went
# 13.3 s -> 4.7 s with seven tokens, overriding `-j2`; with the fifo missing
# cargo prints a warning and falls back to `jobs`. Fail-open, then, like the
# wrappers in .cargo/config.toml.
#
# Three things the pipe needs that make cannot provide here, because there
# is no make process living for the duration of "every session on this box":
#
#   * **A holder.** A fifo keeps its buffered bytes only while something has
#     it open. Between two cargo runs nothing does, and the tokens would
#     vanish. `ensure` starts one long-lived `sleep` with the fifo open and
#     records its pid.
#   * **A refill.** A cargo killed mid-build -- 79 tool timeouts in the
#     transcripts -- never returns the tokens it held, so the pool would
#     shrink for ever. When no cargo or rustc is running every token is by
#     definition free, and `ensure` resets the pool to N. While one *is*
#     running it must not: a token that is out is somebody's live job.
#   * **Somebody to run `ensure`.** The repo scripts do before their own
#     gates; the PreToolUse hook does before any command that mentions
#     cargo; `MAKEFLAGS` itself comes from .claude/settings.json for every
#     session, and from `eval "$(scripts/jobserver.sh env)"` in a shell.
#
# N defaults to two below the core count: memory is the thing four sessions
# actually ran out of, and each cargo also holds one implicit token of its
# own on top of what it draws from the pool.
#
# Usage:
#   scripts/jobserver.sh ensure     # create or repair; prints nothing when fine
#   scripts/jobserver.sh env        # ensure, then print the export line
#   scripts/jobserver.sh status     # where it is, how many tokens are free
#   scripts/jobserver.sh stop       # kill the holder and remove the fifo
#
# Environment:
#   POSTIO_JOBSERVER_DIR      where the fifo lives   (/tmp/postio-jobserver)
#   POSTIO_JOBSERVER_TOKENS   pool size              (nproc - 2, at least 2)
#   POSTIO_JOBSERVER_IDLE     1|0 overrides the "is anything building" check
#                             (the self-test sets it; other sessions on this
#                             machine really are compiling while it runs)
set -euo pipefail

DIR="${POSTIO_JOBSERVER_DIR:-/tmp/postio-jobserver}"
FIFO="$DIR/fifo"
PID_FILE="$DIR/holder.pid"

cores() {
    nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4
}
TOKENS="${POSTIO_JOBSERVER_TOKENS:-$(( $(cores) - 2 ))}"
[ "$TOKENS" -ge 2 ] || TOKENS=2

holder_alive() {
    local pid
    pid=$(cat "$PID_FILE" 2>/dev/null) || return 1
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
}

# Nothing that could be holding a token is running. Names, not `pgrep -f`,
# so a shell whose command line merely mentions cargo does not count.
idle() {
    case "${POSTIO_JOBSERVER_IDLE:-}" in
        1) return 0 ;;
        0) return 1 ;;
    esac
    local name
    for name in cargo rustc rustdoc clippy-driver cargo-nextest cargo-clippy; do
        pgrep -x "$name" >/dev/null 2>&1 && return 1
    done
    return 0
}

# Read whatever is buffered (non-blocking) and put exactly N tokens back.
# Python rather than dd: `dd iflag=nonblock` is GNU-only, and the count has
# to be exact.
refill() {
    FIFO="$FIFO" TOKENS="$TOKENS" python3 - <<'PY'
import os
fd = os.open(os.environ["FIFO"], os.O_RDWR | os.O_NONBLOCK)
try:
    while True:
        try:
            if not os.read(fd, 4096):
                break
        except BlockingIOError:
            break
    os.write(fd, b"+" * int(os.environ["TOKENS"]))
finally:
    os.close(fd)
PY
}

start() {
    mkdir -p "$DIR"
    rm -f "$FIFO" "$PID_FILE"
    mkfifo -m 600 "$FIFO"
    # The holder: opens the fifo read+write (so the open never blocks and
    # the buffer outlives every client), writes its pid, then becomes a
    # sleep. nohup + disown rather than setsid, which macOS does not have.
    #
    # `sleep 2147483647`, not `sleep infinity`. The word is a GNU extension
    # and BSD sleep rejects it outright -- so on macOS the holder exited
    # instantly, `ensure` reported "the holder did not start", no pool ever
    # existed, and every cargo on the machine silently fell back to
    # `jobs = 2` in `.cargo/config.toml` (#1144). A cold build took most of
    # an hour on a box with cores to spare, and nothing said why.
    #
    # INT_MAX seconds is 68 years, which outlives any machine this runs on,
    # and both sleeps accept it.
    nohup bash -c 'exec 3<>"$1"; printf "%s\n" "$$" > "$2"; exec sleep 2147483647' \
        _ "$FIFO" "$PID_FILE" >/dev/null 2>&1 &
    disown 2>/dev/null || true
    local waited=0
    while ! holder_alive && [ "$waited" -lt 50 ]; do
        sleep 0.1; waited=$((waited + 1))
    done
    holder_alive || { echo "jobserver: the holder did not start" >&2; return 1; }
    refill
}

ensure() {
    if holder_alive && [ -p "$FIFO" ]; then
        # Repair only: every token is free when nothing is building, so
        # resetting to N is exact then and wrong at any other time.
        if idle; then refill; fi
        return 0
    fi
    start
}

free_tokens() {
    FIFO="$FIFO" python3 - <<'PY'
import os
fd = os.open(os.environ["FIFO"], os.O_RDWR | os.O_NONBLOCK)
held = b""
try:
    while True:
        try:
            chunk = os.read(fd, 4096)
        except BlockingIOError:
            break
        if not chunk:
            break
        held += chunk
    if held:
        os.write(fd, held)
finally:
    os.close(fd)
print(len(held))
PY
}

case "${1:-ensure}" in
    ensure) ensure ;;
    env)
        ensure
        printf 'export MAKEFLAGS="--jobserver-auth=fifo:%s"\n' "$FIFO"
        ;;
    status)
        if holder_alive && [ -p "$FIFO" ]; then
            echo "jobserver: $FIFO, $(free_tokens) of $TOKENS tokens free, holder pid $(cat "$PID_FILE")"
        else
            echo "jobserver: not running (scripts/jobserver.sh ensure starts it)"
        fi
        ;;
    stop)
        if pid=$(cat "$PID_FILE" 2>/dev/null) && [ -n "$pid" ]; then
            kill "$pid" 2>/dev/null || true
        fi
        rm -f "$FIFO" "$PID_FILE"
        ;;
    *) echo "usage: scripts/jobserver.sh [ensure|env|status|stop]" >&2; exit 2 ;;
esac
