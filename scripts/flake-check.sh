#!/usr/bin/env bash
# Is this failing test broken, or only load-sensitive? One command.
#
# A session spent several rounds proving that a red test
# (`sync_wave::a_queued_write_during_the_wave_is_drained_without_restarting_its_lanes`,
# #2105) passed on an idle box and timed out while other sessions compiled.
# That proof is always the same three steps, so this does them: build once,
# run the one test N times idle, run it N times again beside K busy loops,
# and say which of the three things is true.
#
# It goes through `cargo nextest run`, never the test binary directly: the
# GTK suites return early without the headless compositor that `.cargo/config.toml`'s
# runner and nextest's setup provide, and a test that quietly does nothing
# "passes" every time. Compile jobs come from the machine-wide jobserver; this
# never passes `-j`.
#
# Usage:
#   scripts/flake-check.sh -p <crate> --test <suite> <test-name> [--runs N] [--load-cores K]
#
#   -p, --package CRATE   the crate the test lives in
#   --test SUITE          its integration-test target (omit for a lib test)
#   <test-name>           matched exactly, as nextest names it
#                         (e.g. sync_wave::a_queued_write_during_the_wave...)
#   --runs N              runs per phase (default 5)
#   --load-cores K        busy-loop processes during the loaded phase
#                         (default: every core)
#
# Use it before re-running a red check you think is a flake. Under load a pass
# count below N is the finding: read the test's timing, not its luck.
#
# Exit status:
#   0  passes idle and loaded
#   1  fails only under load (load-sensitive)
#   2  fails idle too: every idle run failed (broken, not flaky)
#   3  usage error, or the build failed
#   4  flaky even idle: some, not all, idle runs failed
#
# Test hooks (scripts/tests/test-flake-check.py): FLAKE_CHECK_CARGO replaces
# `cargo`, FLAKE_CHECK_LOAD_CMD replaces one busy-loop process.
set -uo pipefail

usage() { sed -n '2,/^set -uo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'; }
die() { echo "flake-check: $*" >&2; exit 3; }

CARGO="${FLAKE_CHECK_CARGO:-cargo}"
PKG=""; SUITE=""; NAME=""; RUNS=5; CORES=""
while [ $# -gt 0 ]; do
    case "$1" in
        -h|--help) usage; exit 0 ;;
        -p|--package) [ $# -ge 2 ] || die "$1 needs a value"; PKG="$2"; shift 2 ;;
        --test) [ $# -ge 2 ] || die "$1 needs a value"; SUITE="$2"; shift 2 ;;
        --runs) [ $# -ge 2 ] || die "$1 needs a value"; RUNS="$2"; shift 2 ;;
        --load-cores) [ $# -ge 2 ] || die "$1 needs a value"; CORES="$2"; shift 2 ;;
        -j|-j*|--jobs) die "never pass -j; the machine-wide jobserver governs compile jobs" ;;
        -*) die "unknown option $1 (see --help)" ;;
        *) [ -z "$NAME" ] || die "one test name only, got '$NAME' and '$1'"; NAME="$1"; shift ;;
    esac
done
[ -n "$PKG" ] || die "-p <crate> is required (see --help)"
[ -n "$NAME" ] || die "a test name is required (see --help)"
case "$RUNS" in ''|*[!0-9]*|0) die "--runs must be a positive integer, got '$RUNS'" ;; esac
if [ -z "$CORES" ]; then
    CORES=$(getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)
fi
case "$CORES" in ''|*[!0-9]*|0) die "--load-cores must be a positive integer, got '$CORES'" ;; esac

if [ -z "${FLAKE_CHECK_CARGO:-}" ]; then
    cd "$(git rev-parse --show-toplevel)" || exit 3
    [ -x scripts/install-shims.sh ] && scripts/install-shims.sh
    [ -x scripts/jobserver.sh ] && eval "$(scripts/jobserver.sh env 2>/dev/null || true)"
fi

SCOPE=(-p "$PKG")
[ -z "$SUITE" ] || SCOPE+=(--test "$SUITE")

WORK=$(mktemp -d)
LOADERS=()
stop_load() {
    local p
    for p in ${LOADERS[@]+"${LOADERS[@]}"}; do kill "$p" 2>/dev/null; done
    for p in ${LOADERS[@]+"${LOADERS[@]}"}; do wait "$p" 2>/dev/null; done
    LOADERS=()
}
cleanup() { stop_load; rm -rf "$WORK"; }
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

now() { python3 -c 'import time; print(time.time())'; }
loadavg() { uptime | sed 's/.*load averages*: *//'; }

echo "building once: $CARGO nextest run ${SCOPE[*]} --no-run"
if ! "$CARGO" nextest run "${SCOPE[@]}" --no-run; then
    die "the build failed; nothing was run"
fi

# phase <name>: RUNS runs of the one test; records seconds in $WORK/<name>.times
# and the pass count in $WORK/<name>.passed.
phase() {
    local name="$1" i start end passed=0
    : > "$WORK/$name.times"
    for i in $(seq 1 "$RUNS"); do
        start=$(now)
        if "$CARGO" nextest run "${SCOPE[@]}" --no-fail-fast --no-tests=fail \
                -- --exact "$NAME" >"$WORK/$name.$i.log" 2>&1; then
            passed=$((passed + 1)); res=pass
        else
            res=FAIL
        fi
        end=$(now)
        python3 -c "print('%.2f' % ($end - $start))" >> "$WORK/$name.times"
        echo "  $name run $i/$RUNS: $res ($(tail -n 1 "$WORK/$name.times")s)"
    done
    echo "$passed" > "$WORK/$name.passed"
}

# stats <name>: "min X s, median Y s, max Z s"
stats() {
    python3 - "$WORK/$1.times" <<'PY'
import sys, statistics
t = [float(x) for x in open(sys.argv[1]).read().split()]
print("min %.2fs, median %.2fs, max %.2fs" % (min(t), statistics.median(t), max(t)))
PY
}

echo
echo "idle: $RUNS runs"
LOAD_IDLE_BEFORE=$(loadavg)
phase idle
LOAD_IDLE_AFTER=$(loadavg)

echo
echo "loaded: $RUNS runs beside $CORES busy-loop process(es)"
for _ in $(seq 1 "$CORES"); do
    if [ -n "${FLAKE_CHECK_LOAD_CMD:-}" ]; then
        "$FLAKE_CHECK_LOAD_CMD" >/dev/null 2>&1 &
    else
        sh -c 'while :; do :; done' &
    fi
    LOADERS+=("$!")
done
sleep 0.5
LOAD_LOADED_BEFORE=$(loadavg)
phase loaded
LOAD_LOADED_AFTER=$(loadavg)
stop_load

IDLE_PASSED=$(cat "$WORK/idle.passed")
LOADED_PASSED=$(cat "$WORK/loaded.passed")

echo
echo "test: $PKG ${SUITE:+--test $SUITE }$NAME"
echo "idle:   $IDLE_PASSED/$RUNS passed; $(stats idle)"
echo "        load average before: $LOAD_IDLE_BEFORE; after: $LOAD_IDLE_AFTER"
echo "loaded: $LOADED_PASSED/$RUNS passed; $(stats loaded)"
echo "        load average before: $LOAD_LOADED_BEFORE; after: $LOAD_LOADED_AFTER"
echo

if [ "$IDLE_PASSED" -lt "$RUNS" ] && [ "$IDLE_PASSED" -gt 0 ]; then
    echo "verdict: flaky even idle ($((RUNS - IDLE_PASSED))/$RUNS idle failures)"
    echo "  intermittent, not a plain regression: read the failing run's output for the race."
    exit 4
elif [ "$IDLE_PASSED" -lt "$RUNS" ]; then
    echo "verdict: fails idle too (broken, not flaky)"
    for f in "$WORK"/idle.*.log; do
        if ! grep -q "1 passed" "$f"; then echo "--- tail of $(basename "$f") ---"; tail -n 25 "$f"; break; fi
    done
    exit 2
elif [ "$LOADED_PASSED" -lt "$RUNS" ]; then
    echo "verdict: fails only under load (load-sensitive: see the timing)"
    echo "  it needs a deadline that scales (postio_test_support::scaled/patience) or an assertion on order, not on the clock (docs/gotchas.md)."
    exit 1
fi
echo "verdict: passes idle and loaded"
exit 0
