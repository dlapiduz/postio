#!/usr/bin/env bash
# Has the full suite passed on this release commit? Wait for the run on `main` that is already
# answering: the push that merged the release started `nightly.yml` on the same commit with the same
# suite, and running it twice cost the slower copy's time. Only that run's `Full suite` and
# `Storyboard passes` jobs count. The answer is one of
#
#   green     every one of those jobs passed: the release may publish.
# red       one failed: nothing publishes. Re-run the nightly's failed jobs, then this job; it reads
# the latest attempt.
# run-here  there is no answer to wait for (cancelled by a newer push, never started, skipped, or
# the wait ran out): the release runs the suite itself.
#
# Usage: scripts/release-await-nightly.sh
#   Reads GITHUB_SHA and GITHUB_REPOSITORY. Prints `answer=<answer>` for
#   $GITHUB_OUTPUT on stdout and the reasoning on stderr. Exits 1 on red.
#   RELEASE_AWAIT_POLL (seconds between asks, default 30), RELEASE_AWAIT_GRACE
#   (how long a run may take to appear, default 300) and RELEASE_AWAIT_LIMIT
#   (the whole wait, default 7200) exist for the self-test,
#   scripts/tests/test-release-await-nightly.py.
set -euo pipefail

SHA="${GITHUB_SHA:?}"
REPO="${GITHUB_REPOSITORY:?}"
POLL="${RELEASE_AWAIT_POLL:-30}"
GRACE="${RELEASE_AWAIT_GRACE:-300}"
LIMIT="${RELEASE_AWAIT_LIMIT:-7200}"

answer() {
    echo "$2" >&2
    printf 'answer=%s\n' "$1"
    [ "$1" = red ] && exit 1
    exit 0
}

GATES='^(Full suite|Storyboard passes)'

SECONDS=0
while :; do
    if [ "$SECONDS" -gt "$LIMIT" ]; then
        answer run-here "No answer from the nightly on ${SHA} within ${LIMIT}s; running the suite here."
    fi

    if ! run=$(gh api "repos/${REPO}/actions/workflows/nightly.yml/runs?head_sha=${SHA}&event=push&per_page=1" \
        --jq '.workflow_runs[0] // empty | "\(.id) \(.status) \(.conclusion) \(.html_url)"'); then
        sleep "$POLL"; continue
    fi
    if [ -z "$run" ]; then
        if [ "$SECONDS" -ge "$GRACE" ]; then
            answer run-here "No nightly run on ${SHA} after ${GRACE}s; running the suite here."
        fi
        sleep "$POLL"; continue
    fi
    read -r id status conclusion url <<<"$run"
    if [ "$conclusion" = cancelled ]; then
        answer run-here "The nightly on ${SHA} was cancelled, most likely by a newer push ($url); running the suite here."
    fi

    if ! jobs=$(gh api "repos/${REPO}/actions/runs/${id}/jobs?filter=latest&per_page=100" \
        --jq ".jobs[] | select(.name | test(\"${GATES}\")) | \"\(.status)\t\(.conclusion)\t\(.name)\""); then
        sleep "$POLL"; continue
    fi

    failed=$(awk -F'\t' '$1 == "completed" && ($2 == "failure" || $2 == "timed_out")' <<<"$jobs")
    if [ -n "$failed" ]; then
        answer red "The nightly on ${SHA} failed, so nothing ships: $(cut -f3 <<<"$failed" | paste -sd, -) ($url). Re-run its failed jobs, then this one."
    fi
    if awk -F'\t' '$1 == "completed" && $2 != "success" { found = 1 } END { exit !found }' <<<"$jobs"; then
        answer run-here "The nightly on ${SHA} did not run every gate to the end ($url); running the suite here."
    fi
    if grep -q $'\tFull suite' <<<"$jobs" && grep -q $'\tStoryboard passes' <<<"$jobs" \
        && ! awk -F'\t' '$1 != "completed" { found = 1 } END { exit !found }' <<<"$jobs"; then
        answer green "The nightly passed the full suite and the storyboards on ${SHA} ($url)."
    fi
    if [ "$status" = completed ]; then
        answer run-here "The nightly on ${SHA} finished without running the gates ($url); running the suite here."
    fi
    echo "waiting on $url ($(grep -c . <<<"$jobs" || true) gates seen, $(awk -F'\t' '$1 == "completed"' <<<"$jobs" | grep -c . || true) done)" >&2
    sleep "$POLL"
done
