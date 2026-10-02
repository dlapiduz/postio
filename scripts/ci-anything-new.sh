#!/usr/bin/env bash
# Does a scheduled workflow have anything new to test?
#
# A timer does not care whether anyone committed, so a scheduled run over a
# tree nobody touched re-proves the last answer at full price. This compares
# the head being tested against the head the workflow's last completed run
# tested, and says `run=false` when they match.
#
# It lived inline in `nightly.yml`. Then the benches (a 33-minute release
# compile, daily) and mutation testing (2,064 runner-minutes a week, red every
# night over the same tree) needed the same question asked, and three inline
# copies of it is three chances for the part that matters -- which way it
# fails -- to drift (#1710). An inline step also cannot be tested; this one
# is, by `scripts/tests/test-ci-anything-new.py`.
#
# # Two things it must not do
#
# **It only ever skips a schedule.** `workflow_dispatch` is somebody asking
# for the answer now, and `workflow_call` is a pull request proving it fixed
# the nightly; skipping either answers a question nobody asked.
#
# **It fails towards running.** No answer from the API, or no completed run
# yet, is not evidence that nothing changed.
#
# # `--rerun-failed`
#
# The nightly passes it, and must. `ci.yml`'s `Nightly is green` reads the
# last *completed* run's conclusion, and a run whose jobs all skip concludes
# as `skipped` or `success` -- both of which it counts as green. Skipping a
# red nightly for want of a commit would silently unblock merges with the
# tree still broken. So with the flag, anything that is not success, skipped
# or cancelled runs again even over an unchanged tree: it is the one state
# where the answer has to keep being said out loud. (Written as "not one of
# those three" rather than "is a failure" so it does not depend on which
# spelling of red GitHub uses.)
#
# Nothing reads the benches' or mutants' conclusion, so they do not pass it:
# a red run over an unchanged tree would only report the same survivors again.
#
# Usage: scripts/ci-anything-new.sh <workflow-file> [--rerun-failed]
#   Reads GITHUB_REPOSITORY, GITHUB_EVENT_NAME and GITHUB_SHA, which every
#   Actions step has. Prints exactly `run=true` or `run=false` on stdout --
#   append it to $GITHUB_OUTPUT -- and the reasoning on stderr.
set -euo pipefail

WORKFLOW=""
RERUN_FAILED=0
for arg in "$@"; do
    case "$arg" in
        --rerun-failed) RERUN_FAILED=1 ;;
        -*) echo "unknown option: $arg" >&2; exit 2 ;;
        *) WORKFLOW="$arg" ;;
    esac
done

if [ -z "$WORKFLOW" ]; then
    echo "usage: scripts/ci-anything-new.sh <workflow-file> [--rerun-failed]" >&2
    exit 2
fi

answer() {
    echo "$2" >&2
    echo "run=$1"
    exit 0
}

EVENT="${GITHUB_EVENT_NAME:-}"
HEAD="${GITHUB_SHA:-}"

if [ "$EVENT" != "schedule" ]; then
    answer true "Triggered by ${EVENT:-nothing named}, not the timer — running."
fi

# `status=completed`, so a run still going is not mistaken for an answer --
# and this run is not completed, so the first entry is the previous one.
# `branch=`, so only runs on the branch being tested count: an agent may run
# the full suite on a feature branch before landing, and that run must not
# become the one a scheduled run on `main` compares against.
BRANCH="${GITHUB_REF_NAME:-main}"
last=$(gh api \
    "repos/${GITHUB_REPOSITORY}/actions/workflows/${WORKFLOW}/runs?status=completed&branch=${BRANCH}&per_page=1" \
    --jq '.workflow_runs[0] | "\(.conclusion)\t\(.head_sha)"' 2>/dev/null) || last=""

if [ -z "$last" ] || [ "$last" = "null" ]; then
    answer true "No completed ${WORKFLOW} run to compare against — running."
fi

conclusion=$(printf '%s' "$last" | cut -f1)
sha=$(printf '%s' "$last" | cut -f2)
echo "previous run: ${conclusion} at ${sha}" >&2
echo "this run:     ${HEAD}" >&2

if [ "$RERUN_FAILED" = 1 ]; then
    case "$conclusion" in
        success|skipped|cancelled) ;;
        *) answer true "The last run was ${conclusion} — running, so the answer keeps being said." ;;
    esac
fi

if [ -n "$sha" ] && [ "$sha" = "$HEAD" ]; then
    answer false "Nothing has landed since — skipping."
fi
answer true "New commits since the last run — running."
