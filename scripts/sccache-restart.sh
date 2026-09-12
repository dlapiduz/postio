#!/usr/bin/env bash
# Notice a wedged sccache daemon, and restart it without breaking the cache.
#
# The compile cache has stalled every build on this box twice
# (docs/notes/2026-09-03-the-compile-cache-was-full-and-had-been-for-a-long-time.md,
# then #1184). Both times it cost hours, for two reasons that this script is
# the two answers to.
#
# # 1. Nothing noticed
#
# A wedge is invisible from inside a session: the build simply never finishes
# and prints nothing. The last one held four build directories -- three
# worktrees and the shared checkout -- with eleven sccache-wrapped `rustc`
# processes asleep, ages from one hour to 2h13m, on a box at load 0.17.
#
# The tell is two readings of the daemon's own counter:
#
#     t0 14:02:26 executed=5351
#     t1 14:03:36 executed=5351
#
# `Compile requests executed` frozen while `rustc` processes sit for minutes
# using no CPU. **Both halves are required.** A frozen counter on its own is
# just an idle machine, which is most of the time; stalled compiles on their
# own could be a genuinely slow build. Only together do they mean the daemon
# has stopped answering.
#
# # 2. Restarting it by hand re-created the other failure
#
# `sccache --stop-server` fixes the wedge. The trap is what starts the next
# daemon: sccache's settings are read **when the server starts**, from
# whatever environment that particular command happened to have. So a bare
# `sccache --show-stats` -- the obvious thing to run next -- starts a server
# with the **default 10 GiB** cap. Against a 24 GiB cache directory that is
# instant permanent eviction, which is precisely the September 3 failure. It
# happened during #1184's own diagnosis and had to be undone.
#
# The restart therefore goes through `rustc-wrapper.sh`, which is the one
# place that knows the size, the idle timeout and the logging, and the size is
# read back afterwards rather than assumed.
#
# Usage:
#   scripts/sccache-restart.sh --check     # report only; 0 healthy, 3 wedged
#   scripts/sccache-restart.sh             # restart through the wrapper
#   scripts/sccache-restart.sh --if-wedged # check, and restart only if it is
#   scripts/sccache-restart.sh --reap      # restart, then kill what stayed parked
#
# # 3. The restart does not reach the casualties
#
# Replacing the daemon fixes the daemon. It does nothing for the clients
# already parked on the one it replaced: they are futex-parked waiting for a
# reply from a process that no longer exists, so they wait for ever, and the
# cargo above each one holds `target/`'s lock and `~/.cargo/.package-cache`
# the whole time. On 2026-09-11 that was one abandoned 29-hour `cargo nextest`
# tree serialising every other session on the box, still there after a clean
# restart and only fixed by killing nine pids by hand.
#
# `--reap` is that, as a step: restart, then kill the compiles still parked
# past `STALLED_AFTER`. It is a flag rather than the default because killing
# somebody's build is destructive, and it names every pid as it goes. A plain
# restart says what it left behind and stops there.
#
# Exit status: 0 fine (or restarted), 2 sccache is not installed or would not
# answer, 3 wedged (with --check).
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# How long to leave between the two counter readings, and how old a `rustc`
# has to be to count as stalled. Overridable so the self-test does not sleep.
WINDOW="${POSTIO_SCCACHE_WINDOW:-60}"
STALLED_AFTER="${POSTIO_SCCACHE_STALLED_AFTER:-300}"

# How to kill a parked compile. Overridable because `kill` is a shell builtin
# and a PATH stub cannot shadow one, so it is the only way the self-test can
# record what would die instead of killing it.
KILL="${POSTIO_SCCACHE_KILL:-kill}"

if ! command -v sccache >/dev/null 2>&1; then
    echo "sccache-restart: sccache is not installed; nothing to do" >&2
    exit 2
fi

executed() {
    sccache --show-stats 2>/dev/null |
        awk '/Compile requests executed/ { print $NF; exit }'
}

# How many sccache-wrapped compiles have been sitting for too long.
#
# Matched on `rustc` rather than on sccache: the daemon itself is always there
# and always idle-looking, and what a wedge produces is *clients* waiting on
# it.
stalled() {
    ps -eo etimes,pid,args 2>/dev/null |
        awk -v old="$STALLED_AFTER" '$1 > old && /rustc/ && !/awk/ { count++ } END { print count + 0 }'
}

# The same processes, by pid, for `--reap`.
#
# Deliberately the same match as `stalled` above: the number the check reports
# and the number reaped must not be able to disagree. Counting never needed
# the pid column; killing does.
stalled_pids() {
    ps -eo etimes,pid,args 2>/dev/null |
        awk -v old="$STALLED_AFTER" '$1 > old && /rustc/ && !/awk/ { print $2 }'
}

max_cache_size() {
    # Through the wrapper as well: if the start above somehow did not take,
    # this must not be the command that starts one, because a bare `sccache`
    # starts it at the default size.
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
    # Through the wrapper, and this is the whole point of the script: it is
    # the one caller that knows the size, the idle timeout and the logging
    # this workspace needs, and those are read when the server *starts*.
    #
    # `--start-server` and not a compile. The first version of this ran
    # `rustc --version` through the wrapper on the theory that any compile
    # spawns a daemon -- it does not, because `--version` is not a cacheable
    # compile and sccache just runs it. The server was then started by the
    # `--show-stats` below, which does not go through the wrapper, **at the
    # default 10 GiB** -- this script walking into the exact trap it exists to
    # prevent. The size check caught it, on an isolated daemon.
    #
    # Worth knowing for anything else that has to restart it: the self-test
    # cannot see this. It stubs `sccache`, so it can prove the logic and not
    # what the real binary does with an argument.
    "$HERE/rustc-wrapper.sh" --start-server >/dev/null 2>&1
    local size
    size="$(max_cache_size)"
    if [ -z "$size" ]; then
        echo "sccache-restart: restarted, but the daemon will not say its size" >&2
        return 2
    fi
    # Said out loud rather than assumed, because the failure this guards
    # against is silent: a 10 GiB daemon works perfectly and evicts for ever.
    echo "sccache: restarted, max cache size $size"
    # The daemon is new; anything still parked is parked on the old one and
    # will not recover. Said out loud because the failure is otherwise
    # invisible: the daemon looks fixed while the box stays serialised behind
    # a lock held by a build that can never finish.
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

# Kill the compiles left behind by a daemon that is gone.
#
# Destructive on purpose and never reached without `--reap`: these pids are
# somebody's build, and the only thing that makes killing them right is that
# the process each is waiting for no longer exists.
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
            # Gone between the listing and the kill, or another user's. Not a
            # failure: the point was for it not to be waiting any more.
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
    # Bound before it is tested: `[ $? -eq 3 ]` replaces `$?` with its own
    # result, so the `exit $?` after it reports whether the comparison
    # succeeded rather than what the check found. The self-test caught this
    # reporting a healthy daemon as a failure.
    status=$?
    [ "$status" -eq 3 ] || exit "$status"
    restart
    exit $?
    ;;
--reap)
    # Restart first: reaping while the wedged daemon is still up would kill
    # the clients and leave the thing that wedged them running to collect
    # more.
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
