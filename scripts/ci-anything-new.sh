#!/usr/bin/env bash
# Does a scheduled workflow have anything new to test? Compares the head under test with the head
# the workflow's last completed run tested and says `run=false` when they match. Only a schedule is
# ever skipped (dispatch and workflow_call are someone asking), and it fails towards running when
# the API has no answer. `--rerun-failed`: `ci.yml`'s `Nightly is green` reads the last completed
# run, and an all-skipped run concludes green, so a red nightly must run again over an unchanged
# tree. Shared by nightly, bench and mutants (#1710); tested by
# `scripts/tests/test-ci-anything-new.py`.
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

# `status=completed`, so a running run is not an answer; `branch=`, so a feature-branch full-suite
# run is not what a scheduled `main` run compares against.
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
