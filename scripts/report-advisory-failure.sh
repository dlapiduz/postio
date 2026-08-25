#!/usr/bin/env bash
# File or update a GitHub issue naming the advisory a scheduled cargo-deny
# run failed on, so a RUSTSEC publication has a ceiling on how long it goes
# unnoticed instead of waiting for the next push to touch ci.yml's own
# supply-chain job. See audit.yml and #146.
#
# Split out into its own script, rather than living inline in the workflow,
# so the "new issue or a comment on the existing one" decision can be driven
# by a stubbed `gh` -- see scripts/tests/test-report-advisory-failure.py -- instead
# of only ever being exercised by a real advisory landing in Cargo.lock.
#
# Usage: scripts/report-advisory-failure.sh <cargo-deny-output-file> <run-url>
set -euo pipefail

OUTPUT="${1:?usage: report-advisory-failure.sh <output-file> <run-url>}"
RUN_URL="${2:?usage: report-advisory-failure.sh <output-file> <run-url>}"

# cargo-deny's advisory failures print a line shaped like:
#   ID:       RUSTSEC-2024-0421
# A licence, ban or source failure has no such id -- those do not change
# without a Cargo.lock change and are the push-time job's to catch, but this
# script may as well say something useful if it is ever pointed at one.
ADVISORY=$(grep -oE 'RUSTSEC-[0-9]{4}-[0-9]+' "$OUTPUT" | sort -u | head -n 1 || true)
if [ -n "$ADVISORY" ]; then
    TITLE="cargo-deny: $ADVISORY failed the scheduled audit"
else
    TITLE="cargo-deny: the scheduled advisories audit failed"
fi

BODY=$(printf 'The scheduled supply-chain audit found a problem.\n\n```\n%s\n```\n\nRun: %s\n' \
    "$(cat "$OUTPUT")" "$RUN_URL")

# Exact match on the title, not GitHub's own fuzzy `search`: a second
# advisory landing today must not read as the same issue as one from last
# month just because both titles share the words "cargo-deny" and "failed".
EXISTING_JSON=$(gh issue list --state open --search "$TITLE" --json number,title)
EXISTING=$(printf '%s' "$EXISTING_JSON" \
    | jq -r --arg title "$TITLE" '.[] | select(.title == $title) | .number' \
    | head -n 1)

if [ -n "$EXISTING" ]; then
    gh issue comment "$EXISTING" --body "$BODY"
    echo "commented on existing issue #$EXISTING"
else
    gh issue create --title "$TITLE" --body "$BODY" --label bug
    echo "filed a new issue: $TITLE"
fi
