#!/usr/bin/env bash
# Runs the full workspace test suite, and treats a failing target as real
# only if it *still* fails on its own. See #886.
#
# Cutting v0.2.0 by hand hit this twice: two full-suite runs each threw a
# couple of failures, never the same targets twice, none touching the
# release commit's own diff. Rerunning each failing target alone -- away
# from whatever else `cargo test --workspace` was running concurrently --
# passed clean every time. That triage is what this script automates,
# instead of a session reasoning it out from scratch under time pressure
# (or, worse, a release blocked on noise).
#
# A target that fails in isolation too is not a flake, and fails the run.
#
# Usage: scripts/test-with-flake-retry.sh
# Exit status: 0 if the suite passed, or every failure was confirmed a
# flake by an isolated rerun. 1 if any target failed twice.
set -uo pipefail

LOG="$(mktemp)"
trap 'rm -f "$LOG"' EXIT

cargo test --workspace --no-fail-fast 2>&1 | tee "$LOG"
STATUS="${PIPESTATUS[0]}"

if [ "$STATUS" -eq 0 ]; then
    exit 0
fi

# cargo's own summary of what didn't compile or run, one backtick-quoted
# invocation per line:
#   error: 2 targets failed:
#       `-p postio-account --lib`
#       `-p postio-sync --test sync_suite`
mapfile -t TARGETS < <(
    sed -n '/^error: [0-9]* targets\? failed:$/,$ {
        s/^ *`\(.*\)`$/\1/p
    }' "$LOG"
)

if [ "${#TARGETS[@]}" -eq 0 ]; then
    # Something failed and it did not take this shape -- a compile error
    # before any target list, for instance. Nothing to retry in isolation;
    # the original failure stands.
    echo "release gate: suite failed with no per-target summary to retry" >&2
    exit "$STATUS"
fi

echo
echo "release gate: ${#TARGETS[@]} target(s) failed together; retrying each alone" >&2

REAL_FAILURES=()
for spec in "${TARGETS[@]}"; do
    echo "release gate: retrying isolated: cargo test $spec" >&2
    # shellcheck disable=SC2086 -- $spec is cargo's own argv, meant to split
    if cargo test $spec; then
        echo "release gate: confirmed a flake: $spec" >&2
    else
        echo "release gate: failed again in isolation, not a flake: $spec" >&2
        REAL_FAILURES+=("$spec")
    fi
done

if [ "${#REAL_FAILURES[@]}" -gt 0 ]; then
    echo >&2
    echo "release gate: ${#REAL_FAILURES[@]} target(s) failed twice, blocking the release:" >&2
    printf '  %s\n' "${REAL_FAILURES[@]}" >&2
    exit 1
fi

echo "release gate: every failure was a flake; suite passes" >&2
exit 0
