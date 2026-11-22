#!/usr/bin/env bash
# Wait for this branch's CI checks, and say whether it is safe to merge. "No checks reported" means
# either that the branch schedules no workflow
# or that GitHub has not registered one yet (#92, #135, #139), so the branch's own diff decides: the
# workflows' `on.pull_request` path filters
# (read by ci-expected-workflows.py) say what it schedules, and `gh` is only asked whether the
# predicted checks have appeared.
#
# Usage:  scripts/wait-for-checks.sh <pr-url>
#
# Exit status:
#   0  safe to merge -- checks passed, or none were ever due
#   1  do not merge -- a check failed, or one was due and never appeared
#
# Environment (the tests set these; nobody else should need to):
#   POSTIO_CHECKS_REGISTER_TIMEOUT  seconds to wait for a due check   (180)
#   POSTIO_CHECKS_GRACE             seconds to look anyway when none is due (30)
#   POSTIO_CHECKS_POLL              seconds between polls              (5)
# POSTIO_CI_WORKFLOWS_DIR         workflow directory to predict from, in place of this repo's real
# .github/workflows
# No `-e`: the predicate below signals "nothing scheduled" with exit 1, and
# that is an answer, not a failure.
set -uo pipefail

URL="${1:-the PR}"
HERE=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

REGISTER_TIMEOUT="${POSTIO_CHECKS_REGISTER_TIMEOUT:-180}"
GRACE="${POSTIO_CHECKS_GRACE:-30}"
POLL="${POSTIO_CHECKS_POLL:-5}"

CI_EXPECTED_ARGS=(--base main)
if [ -n "${POSTIO_CI_WORKFLOWS_DIR:-}" ]; then
    CI_EXPECTED_ARGS+=(--workflows "$POSTIO_CI_WORKFLOWS_DIR")
fi
EXPECTED=$(git diff --name-only origin/main...HEAD \
    | python3 "$HERE/checks/ci-expected-workflows.py" "${CI_EXPECTED_ARGS[@]}")
SCHEDULED=$?

case "$SCHEDULED" in
    0) echo "expecting: $(printf '%s' "$EXPECTED" | tr '\n' ' ')"
       DEADLINE=$REGISTER_TIMEOUT ;;
    1) # Look anyway. Being wrong here merges unchecked code; being wrong the
       echo "the diff schedules no workflow; watching briefly anyway."
       DEADLINE=$GRACE ;;
    *) echo "could not read the workflow filters; assuming a check is due." >&2
       DEADLINE=$REGISTER_TIMEOUT ;;
esac

# Asked positively: `gh pr checks` exits non-zero both while nothing has registered and when a check
# failed; `--json` reports the checks themselves.
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

# `--watch --fail-fast` exiting 0 is not proof every conclusion has landed: #161 saw it succeed two
# seconds before CI's FAILURE was recorded.
# So wait one beat, then read the buckets with `gh --json` without watching.
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
