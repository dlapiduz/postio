#!/usr/bin/env bash
# Runs the full workspace test suite under nextest, and treats a failing
# test as real only if it *still* fails on its own. See #886.
#
# Cutting v0.2.0 by hand hit this twice: two full-suite runs each threw a
# couple of failures, never the same targets twice, none touching the
# release commit's own diff. Rerunning each failing target alone -- away
# from whatever else the suite was running concurrently -- passed clean
# every time. That triage is what this script automates, instead of a
# session reasoning it out from scratch under time pressure (or, worse, a
# release blocked on noise).
#
# Cutting v0.3.0 hit a different failure this script did not guard against:
# it used to run `cargo test --workspace --no-fail-fast`, written before this
# workspace adopted nextest. That has two costs, and the second one is what
# hung the release for 30+ minutes with no output. `cargo test` has no
# concept of `.config/nextest.toml`'s `default-filter`, so `idle_store_cpu`
# -- a measurement test deliberately excluded from the merge path, documented
# in its own file as costing 44.1s there specifically because it is too slow
# for it -- ran anyway. And plain `cargo test` has no timeout backstop the
# way nextest's `slow-timeout` does, so when that test's `Engine::spawn`
# never answered in this environment, nothing terminated it. nextest's
# `[profile.default] slow-timeout = { period = "60s", terminate-after = 4 }`
# is the fix for exactly this shape, is already configured, and this script
# was the one place still bypassing both protections.
#
# A test that fails in isolation too is not a flake, and fails the run.
#
# Usage: scripts/test-with-flake-retry.sh
# Exit status: 0 if the suite passed, or every failure was confirmed a
# flake by an isolated rerun. 1 if any test failed twice.
set -uo pipefail

LOG="$(mktemp)"
trap 'rm -f "$LOG"' EXIT

cargo nextest run --workspace --profile ci --no-fail-fast 2>&1 | tee "$LOG"
STATUS="${PIPESTATUS[0]}"

if [ "$STATUS" -eq 0 ]; then
    exit 0
fi

# nextest's own final summary, one FAIL line per failing test, after the
# `Summary [...]` marker -- the same shape appears once per test *during*
# the run too, so reading only what comes after that marker is what keeps
# a test counted once rather than twice:
#         FAIL [   0.004s] (2/2) postio-storage::storage_suite tests::a_thing
#
# Portable BRE, not `\?`/`\+` (GNU extensions BSD sed matches literally) and
# no `mapfile` (a bash-4 builtin absent from macOS's bash 3.2) -- both cost a
# real release once. See git history on this file.
FAILURES=()
while IFS= read -r line; do
    [ -n "$line" ] && FAILURES+=("$line")
done < <(
    sed -n '/^ *Summary /,$ {
        s/^ *FAIL \[[^]]*\] ([0-9]*\/[0-9]*) \(.*\)$/\1/p
    }' "$LOG"
)

if [ "${#FAILURES[@]}" -eq 0 ]; then
    # Something failed and it did not take this shape -- a compile error
    # before any test ran, for instance. Nothing to retry in isolation;
    # the original failure stands.
    echo "release gate: suite failed with no per-test summary to retry" >&2
    exit "$STATUS"
fi

echo
echo "release gate: ${#FAILURES[@]} test(s) failed together; retrying each alone" >&2

REAL_FAILURES=()
for spec in "${FAILURES[@]}"; do
    binary_id="${spec%% *}"
    test_name="${spec#* }"
    echo "release gate: retrying isolated: $spec" >&2
    if cargo nextest run --profile ci -E "binary_id($binary_id) & test(=$test_name)"; then
        echo "release gate: confirmed a flake: $spec" >&2
    else
        echo "release gate: failed again in isolation, not a flake: $spec" >&2
        REAL_FAILURES+=("$spec")
    fi
done

if [ "${#REAL_FAILURES[@]}" -gt 0 ]; then
    echo >&2
    echo "release gate: ${#REAL_FAILURES[@]} test(s) failed twice, blocking the release:" >&2
    printf '  %s\n' "${REAL_FAILURES[@]}" >&2
    exit 1
fi

echo "release gate: every failure was a flake; suite passes" >&2
exit 0
