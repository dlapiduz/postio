#!/usr/bin/env bash
# Wait for this branch's CI checks, and say whether it is safe to merge.
#
# `issue-land.sh` used to ask `gh pr checks` and merge if it said "no checks
# reported". That sentence means two different things -- this branch schedules
# no workflow, and GitHub has not registered one yet -- and nothing downstream
# could tell them apart. Lost one way it cost a re-run (#92, #106, #118); lost
# the other it merged a five-crate change before CI started (#135). See #139.
#
# So the branch's own diff decides. The workflow files' `on.pull_request` path
# filters are the authority on what a change schedules, and
# ci-expected-workflows.py reads them. `gh` is then only asked whether the
# checks it predicted have shown up yet.
#
# Usage:  scripts/wait-for-checks.sh <pr-url>
#
# Exit status:
#   0  safe to merge -- checks passed, or none were ever due
#   1  do not merge -- a check failed, or one was due and never appeared
#
# Environment (the tests set all three; nobody else should need to):
#   POSTIO_CHECKS_REGISTER_TIMEOUT  seconds to wait for a due check   (180)
#   POSTIO_CHECKS_GRACE             seconds to look anyway when none is due (30)
#   POSTIO_CHECKS_POLL              seconds between polls              (5)
# No `-e`: the predicate below signals "nothing scheduled" with exit 1, and
# that is an answer, not a failure.
set -uo pipefail

URL="${1:-the PR}"
HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

REGISTER_TIMEOUT="${POSTIO_CHECKS_REGISTER_TIMEOUT:-180}"
GRACE="${POSTIO_CHECKS_GRACE:-30}"
POLL="${POSTIO_CHECKS_POLL:-5}"

EXPECTED=$(git diff --name-only origin/main...HEAD \
    | python3 "$HERE/ci-expected-workflows.py" --base main)
SCHEDULED=$?

case "$SCHEDULED" in
    0) echo "expecting: $(printf '%s' "$EXPECTED" | tr '\n' ' ')"
       DEADLINE=$REGISTER_TIMEOUT ;;
    1) # Look anyway. Being wrong here merges unchecked code; being wrong the
       # other way costs half a minute, and the diff is not the only thing
       # that can schedule a workflow -- a `workflow_dispatch` or a rerun can.
       echo "the diff schedules no workflow; watching briefly anyway."
       DEADLINE=$GRACE ;;
    *) echo "could not read the workflow filters; assuming a check is due." >&2
       DEADLINE=$REGISTER_TIMEOUT ;;
esac

# Asked positively. `gh pr checks` exits non-zero both while nothing has
# registered and when a check has failed, so its status cannot answer this;
# `--json` can, because it reports the checks themselves.
checks_registered() {
    local json
    json=$(gh pr checks --json name 2>/dev/null || true)
    [ -n "$json" ] && [ "$json" != "[]" ]
}

WAITED=0
while ! checks_registered && [ "$WAITED" -lt "$DEADLINE" ]; do
    sleep "$POLL"
    WAITED=$((WAITED + POLL))
done

if ! checks_registered; then
    if [ "$SCHEDULED" = 1 ]; then
        echo "no checks after ${DEADLINE}s, as the diff predicted; nothing to wait for."
        exit 0
    fi
    echo >&2
    echo "Expected a check and none appeared in ${DEADLINE}s. Not merging." >&2
    echo "$URL is open and the branch is pushed. Look at its checks tab," >&2
    echo "then run this script again." >&2
    exit 1
fi

if ! gh pr checks --watch --fail-fast; then
    echo >&2
    echo "Checks failed. $URL is open and the branch is pushed." >&2
    echo "Fix it on this branch and run issue-land.sh again -- do not open a" >&2
    echo "second PR, and do not leave it sitting." >&2
    exit 1
fi

# `--watch --fail-fast` exiting 0 is not proof every check's conclusion has
# actually landed: #161 saw it return success two seconds before CI's own
# FAILURE was recorded, and the merge went ahead on the red commit. So this
# waits one more beat for the API to catch up, then asks again without
# watching and reads the buckets itself -- `gh --json` reports the checks
# as they stand right now, unlike `--watch`'s exit code, which only proves
# what it last polled.
sleep "$POLL"
BAD=$(gh pr checks --json name,bucket \
    | jq -c '[.[] | select(.bucket != "pass" and .bucket != "skipping")]')
if [ "$BAD" != "[]" ]; then
    echo >&2
    echo "A check is not green after watching finished. $URL is open and" >&2
    echo "the branch is pushed. Fix it on this branch and run issue-land.sh" >&2
    echo "again -- do not open a second PR, and do not leave it sitting." >&2
    echo "$BAD" >&2
    exit 1
fi
