#!/usr/bin/env bash
# Runs the full workspace test suite under nextest, and treats a failing test as real only if it
# still fails on its own (#886). Full-suite runs threw failures never the same twice and none
# touching the diff; rerunning each target alone passed. It uses nextest, not `cargo test`, which
# ignores `.config/nextest.toml`'s `default-filter` (so a slow measurement ran) and has no
# `slow-timeout` backstop. A test that fails in isolation too is not a flake, and fails the run.
#
# Usage: scripts/test-with-flake-retry.sh [profile] [package]
#
# The profile defaults to `ci`; the release gate asks for `ci-full`. An isolated retry uses the
# run's profile, or "it passed alone" answers a different question. A `package` narrows the first
# run, and each retry builds the same scope: a bare `cargo nextest run` builds only default members,
# and a retry of a postio-ffi test ran nothing and was called a second failure (#2154). The retry
# exists because the GTK editor cases flake on a shared runner (a WebKit message that never arrives
# with no GPU). It weakens nothing: a test that fails twice still fails. Further arguments are cargo
# target selection used by the first run and each retry; `POSTIO_NEXTEST_FILTER`, when set, is a
# filterset narrowing the first run. Together they let a PR re-run only what a red nightly failed
# (`scripts/ci-nightly-rerun.py`). `POSTIO_FAILED_TESTS_FILE`, when set, receives the tests that
# failed twice, one `<binary_id> <test name>` per line.
#
# Exit status: 0 if the suite passed, or every failure was confirmed a
# flake by an isolated rerun. 1 if any test failed twice.
set -uo pipefail

PROFILE="${1:-ci}"
PACKAGE="${2:-}"
if [ "$#" -ge 2 ]; then shift 2; else shift "$#"; fi
# `${a[@]+"${a[@]}"}` throughout: bash 3.2 calls an empty array unbound
# under `set -u`.
SELECT=("$@")
if [ -n "$PACKAGE" ]; then
    SCOPE=(-p "$PACKAGE")
elif [ "${#SELECT[@]}" -gt 0 ]; then
    SCOPE=()
else
    SCOPE=(--workspace)
fi
FILTER=()
if [ -n "${POSTIO_NEXTEST_FILTER:-}" ]; then
    FILTER=(--no-tests=warn -E "$POSTIO_NEXTEST_FILTER")
fi

LOG="$(mktemp)"
trap 'rm -f "$LOG"' EXIT

cargo nextest run ${SCOPE[@]+"${SCOPE[@]}"} ${SELECT[@]+"${SELECT[@]}"} \
    --profile "$PROFILE" --no-fail-fast ${FILTER[@]+"${FILTER[@]}"} 2>&1 | tee "$LOG"
STATUS="${PIPESTATUS[0]}"

# Each retry below rewrites the profile's report; the budget step reads the whole run's.
REPORT="${CARGO_TARGET_DIR:-target}/nextest/$PROFILE/junit.xml"
[ -f "$REPORT" ] && cp "$REPORT" "${REPORT%.xml}-run.xml"

if [ "$STATUS" -eq 0 ]; then
    exit 0
fi

# nextest's own final summary, one FAIL line per failing test after the `Summary [...]` marker (the
# same shape appears during the run, so only what follows counts each test once):
#         FAIL [   0.004s] (2/2) postio-storage::storage_suite tests::a_thing
# Portable BRE and no `mapfile` (GNU `\?`/`\+` and bash 4 break on macOS). On a runner the log is in
# colour, so the escapes go first or the summary matched nothing.
ESC="$(printf '\033')"
FAILURES=()
while IFS= read -r line; do
    [ -n "$line" ] && FAILURES+=("$line")
done < <(
    sed "s/${ESC}\[[0-9;]*m//g" "$LOG" | sed -n '/^ *Summary /,$ {
        s/^ *FAIL \[[^]]*\] ([0-9]*\/[0-9]*) \(.*\)$/\1/p
    }'
)

if [ "${#FAILURES[@]}" -eq 0 ]; then
    # Something failed in another shape (a compile error before any test ran): nothing to retry in
    # isolation.
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
    if cargo nextest run ${SCOPE[@]+"${SCOPE[@]}"} ${SELECT[@]+"${SELECT[@]}"} --profile "$PROFILE" -E "binary_id($binary_id) & test(=$test_name)"; then
        echo "release gate: confirmed a flake: $spec" >&2
        if [ -n "${GITHUB_ACTIONS:-}" ]; then
            echo "::warning title=Flaky test::failed with the suite, passed alone: $spec"
        fi
    else
        echo "release gate: failed again in isolation, not a flake: $spec" >&2
        REAL_FAILURES+=("$spec")
    fi
done

if [ -n "${POSTIO_FAILED_TESTS_FILE:-}" ]; then
    : > "$POSTIO_FAILED_TESTS_FILE"
    for spec in ${REAL_FAILURES[@]+"${REAL_FAILURES[@]}"}; do
        printf '%s\n' "$spec" >> "$POSTIO_FAILED_TESTS_FILE"
    done
fi

if [ "${#REAL_FAILURES[@]}" -gt 0 ]; then
    echo >&2
    echo "release gate: ${#REAL_FAILURES[@]} test(s) failed twice, blocking the release:" >&2
    printf '  %s\n' "${REAL_FAILURES[@]}" >&2
    exit 1
fi

echo "release gate: every failure was a flake; suite passes" >&2
exit 0
