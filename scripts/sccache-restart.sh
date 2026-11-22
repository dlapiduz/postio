#!/usr/bin/env bash
# Notice a wedged sccache daemon, and restart it without breaking the cache. It has stalled every
# build on this box twice
# (docs/notes/2026-09-03-the-compile-cache-was-full-and-had-been-for-a-long-time.md, #1184). A wedge
# is invisible: builds never finish and print nothing. The tell is the daemon's `Compile requests
# executed` counter frozen across two readings while `rustc` processes sit for minutes using no CPU;
# both halves are required. Restarting by hand re-created the other failure: sccache reads its
# settings when the server STARTS, so a bare `sccache --show-stats` starts a daemon at the default
# 10 GiB cap. The restart goes through `rustc-wrapper.sh`, which knows the size, idle timeout and
# logging, and the size is read back. A restart does not reach clients parked on the old daemon,
# which hold `target/`'s lock and `~/.cargo/.package-cache` for ever; `--reap` kills those compiles
# still parked past `STALLED_AFTER`. A flag, not the default, since it kills somebody's build and
# names every pid.
#
# Usage:
#   scripts/sccache-restart.sh --check     # report only; 0 healthy, 3 wedged
#   scripts/sccache-restart.sh             # restart through the wrapper
#   scripts/sccache-restart.sh --if-wedged # check, and restart only if it is
#   scripts/sccache-restart.sh --reap      # restart, then kill what stayed parked
#
# Exit status: 0 fine (or restarted), 2 sccache is not installed or would not
# answer, 3 wedged (with --check).
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# How long between the two counter readings, and how old a `rustc` must be to count as stalled;
# overridable so the self-test does not sleep.
WINDOW="${POSTIO_SCCACHE_WINDOW:-60}"
STALLED_AFTER="${POSTIO_SCCACHE_STALLED_AFTER:-300}"

# Overridable because `kill` is a shell builtin a PATH stub cannot shadow; the self-test records
# what would die.
KILL="${POSTIO_SCCACHE_KILL:-kill}"

if ! command -v sccache >/dev/null 2>&1; then
    echo "sccache-restart: sccache is not installed; nothing to do" >&2
    exit 2
fi

executed() {
    sccache --show-stats 2>/dev/null |
        awk '/Compile requests executed/ { print $NF; exit }'
}

# How many sccache-wrapped compiles have sat too long: matched on `rustc`, since the daemon is
# always there and a wedge produces clients waiting on it.
stalled() {
    ps -eo etimes,pid,args 2>/dev/null |
        awk -v old="$STALLED_AFTER" '$1 > old && /rustc/ && !/awk/ { count++ } END { print count + 0 }'
}

# The same processes, by pid, for `--reap`; the same match as `stalled`, so the reported and reaped
# numbers cannot disagree.
stalled_pids() {
    ps -eo etimes,pid,args 2>/dev/null |
        awk -v old="$STALLED_AFTER" '$1 > old && /rustc/ && !/awk/ { print $2 }'
}

max_cache_size() {
    # Through the wrapper: a bare `sccache` would start a server at the default size.
    "$HERE/rustc-wrapper.sh" --show-stats 2>/dev/null |
        awk '/Max cache size/ { $1=""; $2=""; $3=""; sub(/^ +/, ""); print; exit }'
}

check() {
    local first second waiting
    first="$(executed)"
    if [ -z "$first" ]; then
        echo "sccache-restart: the daemon did not answer --show-stats" >&2
        return 2
    fi
    waiting="$(stalled)"
    if [ "$waiting" -eq 0 ]; then
        echo "sccache: $first compiles executed, nothing waiting -- fine"
        return 0
    fi
    sleep "$WINDOW"
    second="$(executed)"
    if [ "$first" != "$second" ]; then
        echo "sccache: $first -> $second over ${WINDOW}s with $waiting waiting -- moving"
        return 0
    fi
    echo "sccache: WEDGED -- $waiting compile(s) waiting over ${STALLED_AFTER}s" >&2
    echo "sccache: and the counter has not moved from $first in ${WINDOW}s" >&2
    echo "sccache: restart it with scripts/sccache-restart.sh" >&2
    return 3
}

restart() {
    sccache --stop-server >/dev/null 2>&1
    # `--start-server` through the wrapper, not a compile: `rustc --version` is not cacheable and
    # spawns no daemon, so the `--show-stats` below started one at the default 10 GiB.
    # The self-test stubs `sccache`, so it cannot see this.
    "$HERE/rustc-wrapper.sh" --start-server >/dev/null 2>&1
    local size
    size="$(max_cache_size)"
    if [ -z "$size" ]; then
        echo "sccache-restart: restarted, but the daemon will not say its size" >&2
        return 2
    fi
    echo "sccache: restarted, max cache size $size"
    # Anything still parked is parked on the old daemon and will not recover; said out loud, since
    # the box otherwise stays serialised behind a lock held by a build that cannot finish.
    local parked
    parked="$(stalled_pids | wc -l)"
    if [ "$parked" -gt 0 ]; then
        echo "sccache: $parked compile(s) are still parked on the daemon that was replaced." >&2
        echo "sccache: they cannot recover, and each one's cargo holds its locks meanwhile." >&2
        echo "sccache: clear them with scripts/sccache-restart.sh --reap" >&2
    fi
    case "$size" in
    *10\ GiB*)
        echo "sccache-restart: that is the DEFAULT size, not this workspace's." >&2
        echo "sccache-restart: something started a daemon outside the wrapper." >&2
        return 2
        ;;
    esac
    return 0
}

# Kill the compiles left behind by a gone daemon: destructive, so only with `--reap`, and right only
# because the process each waits for no longer exists.
reap() {
    local pids count=0
    pids="$(stalled_pids)"
    if [ -z "$pids" ]; then
        echo "sccache: nothing is parked; nothing to reap"
        return 0
    fi
    for pid in $pids; do
        if "$KILL" "$pid" 2>/dev/null; then
            echo "sccache: reaped compile $pid, parked over ${STALLED_AFTER}s"
            count=$((count + 1))
        else
            echo "sccache: $pid was already gone" >&2
        fi
    done
    echo "sccache: reaped $count parked compile(s)"
    return 0
}

case "${1:-}" in
--check)
    check
    exit $?
    ;;
--if-wedged)
    check
    # Bound before it is tested: `[ $? -eq 3 ]` replaces `$?`, so `exit $?` reported the comparison,
    # not the check.
    status=$?
    [ "$status" -eq 3 ] || exit "$status"
    restart
    exit $?
    ;;
--reap)
    # Restart first: reaping while the wedged daemon is up would kill the clients and leave what
    # wedged them running.
    restart
    status=$?
    reap
    exit "$status"
    ;;
"")
    restart
    exit $?
    ;;
*)
    echo "usage: sccache-restart.sh [--check|--if-wedged|--reap]" >&2
    exit 2
    ;;
esac
