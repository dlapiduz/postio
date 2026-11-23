#!/usr/bin/env bash
# One cargo jobserver for the whole machine (#1104). Cargo joins one via
# `MAKEFLAGS=--jobserver-auth=fifo:<path>`, hands it to rustc and
# build scripts, and ignores `jobs` while it has one: a lone session gets the whole box and the
# ceiling is on the machine, which bounds memory.
# Without the fifo cargo warns and falls back to `jobs`. Three things make cannot provide here:
# * A holder: a fifo keeps its bytes only while something has it open, so `ensure` starts one
# long-lived `sleep` and records its pid.
# * A refill: a killed cargo never returns its tokens, so when no cargo or rustc is running `ensure`
# resets the pool to N.
# * Someone to run `ensure`: the repo scripts, the PreToolUse hook, and `eval
# "$(scripts/jobserver.sh env)"`; MAKEFLAGS comes from .claude/settings.json.
# N defaults to two below the core count: memory is what ran out, and each cargo holds one implicit
# token of its own.
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

# Nothing that could hold a token is running. Names, not `pgrep -f`, so a shell whose command line
# mentions cargo does not count.
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

# Read whatever is buffered (non-blocking) and put exactly N tokens back; Python because `dd
# iflag=nonblock` is GNU-only.
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
    # The holder opens the fifo read+write so the buffer outlives every client; nohup + disown,
    # since macOS has no setsid.
    # `sleep 2147483647`, not `sleep infinity`: BSD sleep rejects the word, so on macOS the holder
    # exited and cargo fell back to `jobs = 2` (#1144).
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
        # Repair only: every token is free when nothing is building, so resetting to N is exact then
        # and wrong at any other time.
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
