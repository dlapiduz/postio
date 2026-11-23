#!/usr/bin/env bash
# Catch a CPU burst in a running Postio and profile it.
#
# #1216 is a burn nobody could profile: the bursts are sub-second, and every
# obvious instrument answers the wrong question. Four failures went into this
# script, each of which produced a confident empty or corrupt result:
#
#   1. Trigger-then-record cannot see a sub-second burst. A watcher that
#      deliberated for 4 s caught 18 samples; tightened to 1 s it caught
#      *zero* across a 25 s window, because the burst was over before perf
#      attached. So perf runs continuously in --overwrite mode, keeping the
#      last few seconds in a ring buffer and writing nothing until signalled.
#      The profile has to come from *before* the trigger.
#
#   2. `pgrep -f "perf record..."` matches the watcher's own command line, so
#      SIGUSR2 went to a shell and perf never dumped. Match the process NAME.
#
#   3. `grep "Samples: N"` also matches "Total Lost Samples: 0". A perfectly
#      good 6,000-sample capture was reported as zero and nearly discarded.
#      Anchor it: `^# Samples:`.
#
#   4. A dump that is not checked is not a capture, and there are two ways to
#      get one that looks fine and says nothing:
#
#        - Empty. The ring held nothing when it was signalled — it had just
#          been armed, or the process was near-idle. `perf report` parses it
#          happily and prints no samples at all. Observed: a 132 KB dump
#          with zero samples, and a 203 KB one with 53.
#        - Corrupt. `perf report` reads the header and then fails on the
#          first sample: "processing failed for event of type: INVALID".
#
#      They need different answers — wait longer versus throw it away — so
#      they are reported differently below, and a capture is announced only
#      when it parses AND carries at least MIN_SAMPLES. A profile of 53
#      samples misleads in exactly the way the three failures above did.
#
#
# Frame pointers, not DWARF, measured: the burn lands in `sha512_block_data_order_avx2` (OpenSSL
# assembly with no frame pointer), which neither unwinder gets out of, so the profile is flat either
# way, and frame pointers give ~125x the samples per MB (7,000 in 0.9 MB vs 248 in 4.2 MB). Naming
# the query wants SQLite's trace hook (`postio_storage::test_support::counting`), not a third
# unwinder. ptrace samplers (`eu-stack`, `gdb`) stop the target at a syscall boundary and reported
# it parked in `epoll_wait` while it burned a core.
#
# Usage:
#   scripts/profile-burst.sh [--out DIR] [--threshold PERCENT] [--seconds N]
#
# Needs passwordless sudo for perf, and perf installed (`sudo dnf install perf`).
set -uo pipefail

out="${TMPDIR:-/tmp}/postio-burst"
threshold=55
seconds=0            # 0 = run until interrupted

# How long the ring must have been collecting before a dump parses (failure 4 above).
ARM_TIME=20

# Fewest samples worth calling a profile: below it the percentages are noise. Kept low, since a
# burst caught early can be worth reading at a few hundred.
MIN_SAMPLES=150

while [ $# -gt 0 ]; do
  case "$1" in
    --out)       out="$2"; shift 2 ;;
    --threshold) threshold="$2"; shift 2 ;;
    --seconds)   seconds="$2"; shift 2 ;;
    -h|--help)   sed -n '2,36p' "$0"; exit 0 ;;
    *)           echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

command -v perf >/dev/null || { echo "perf is not installed: sudo dnf install perf" >&2; exit 1; }
sudo -n true 2>/dev/null || { echo "this needs passwordless sudo for perf" >&2; exit 1; }

pid=$(pgrep -x postio | head -1)
[ -n "$pid" ] || { echo "postio is not running" >&2; exit 1; }

mkdir -p "$out"
log="$out/burst.log"

say() { echo "$(date +%H:%M:%S) $*" | tee -a "$log"; }

# --overwrite keeps only the most recent buffer; --switch-output=signal turns SIGUSR2 into a dump.
sudo -n perf record -F 999 -g -m 64M -p "$pid" \
     --overwrite --switch-output=signal \
     -o "$out/ring.data" -- sleep 100000 >/dev/null 2>&1 &
sleep 3
perfpid=$(pgrep -x perf | tail -1)
[ -n "$perfpid" ] || { echo "could not start the ring-buffer profiler" >&2; exit 1; }
trap 'sudo -n kill $perfpid 2>/dev/null' EXIT

armed=$SECONDS
say "ring buffer armed on postio pid $pid (perf $perfpid), threshold ${threshold}%"
say "dumps and this log: $out"

# Classify a dump: a valid header proves nothing, so parse and count samples. Echoes `ok <n>`, `thin
# <n>` or `corrupt`.
classify() {
  local file="$1" out n
  # Not under sudo: perf refuses a file owned by neither root nor the caller, quietly (failure 5).
  out=$(perf report --force -i "$file" --stdio --no-children 2>&1)
  if grep -qE 'processing failed|failed to process' <<<"$out"; then
    echo corrupt
    return
  fi
  # "^# Samples:", not "Samples:": "Total Lost Samples: 0" matches the loose form (failure 3).
  n=$(grep -E '^# Samples:' <<<"$out" | head -1 | sed -E 's/^# Samples: +//; s/ .*//')
  case "$n" in
    "")     echo "thin 0" ;;
    *K|*M)  echo "ok $n" ;;
    *)      if [ "$n" -ge "$MIN_SAMPLES" ] 2>/dev/null; then echo "ok $n"; else echo "thin $n"; fi ;;
  esac
}

started=$SECONDS
while true; do
  if [ "$seconds" -gt 0 ] && [ $((SECONDS - started)) -ge "$seconds" ]; then
    say "watched for ${seconds}s; stopping"
    exit 0
  fi
  now=$(pgrep -x postio | head -1)
  if [ "$now" != "$pid" ]; then
    say "postio restarted or gone (was $pid, now ${now:-none}); re-arm needed"
    exit 0
  fi
  a=$(awk '{print $14+$15}' "/proc/$pid/stat" 2>/dev/null) || continue
  sleep 1
  b=$(awk '{print $14+$15}' "/proc/$pid/stat" 2>/dev/null) || continue
  pct=$((b - a))
  [ "$pct" -ge "$threshold" ] || continue
  if [ $((SECONDS - armed)) -lt "$ARM_TIME" ]; then
    say "CPU ${pct}% -- ring only armed $((SECONDS - armed))s ago; letting it fill"
    continue
  fi

  sudo -n kill -USR2 "$perfpid" 2>/dev/null
  sleep 2
  newest=$(sudo -n ls -t "$out"/ring.data.* 2>/dev/null | head -1)
  if [ -z "$newest" ]; then
    say "CPU ${pct}% -- signalled, but no snapshot appeared"
    continue
  fi
  sudo -n chown "$(id -un):$(id -gn)" "$newest" 2>/dev/null
  verdict=$(classify "$newest")
  case "$verdict" in
    ok\ *)
      say "CPU ${pct}% -- dumped $(basename "$newest"), ${verdict#ok } samples"
      say "  by thread:"
      perf report --force -i "$newest" --stdio --no-children --sort comm 2>/dev/null \
        | grep -E '^ +[0-9]+\.[0-9]+%' | head -5 | sed 's/^/  /' | tee -a "$log"
      say "  top frames:"
      perf report --force -i "$newest" --stdio --no-children 2>/dev/null \
        | grep -E '^ +[0-9]+\.[0-9]+%' | head -10 | sed 's/^/  /' | tee -a "$log"
      ;;
    thin\ *)
      say "CPU ${pct}% -- $(basename "$newest") holds only ${verdict#thin } samples;"
      say "  the ring had nothing in it. Not a profile. Still watching."
      ;;
    corrupt)
      say "CPU ${pct}% -- $(basename "$newest") will not parse past its header."
      say "  Kept for forensics, but it is not a profile. Still watching."
      ;;
  esac
  sleep 15
done
