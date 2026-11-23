#!/usr/bin/env bash
# File or update a GitHub issue naming the advisory a scheduled cargo-deny run failed on (audit.yml,
# #146). A script, so the
# new-issue-or-comment decision can be driven by a stubbed `gh`
# (scripts/tests/test-report-advisory-failure.py).
#
# Usage: scripts/report-advisory-failure.sh <cargo-deny-output-file> <run-url>
set -euo pipefail

OUTPUT="${1:?usage: report-advisory-failure.sh <output-file> <run-url>}"
RUN_URL="${2:?usage: report-advisory-failure.sh <output-file> <run-url>}"

# Advisory failures print `ID:       RUSTSEC-2024-0421`; a licence, ban or source failure has no
# such id (the push-time job's to catch).
ADVISORY=$(grep -oE 'RUSTSEC-[0-9]{4}-[0-9]+' "$OUTPUT" | sort -u | head -n 1 || true)
if [ -n "$ADVISORY" ]; then
    TITLE="cargo-deny: $ADVISORY failed the scheduled audit"
else
    TITLE="cargo-deny: the scheduled advisories audit failed"
fi

BODY=$(printf 'The scheduled supply-chain audit found a problem.\n\n```\n%s\n```\n\nRun: %s\n' \
    "$(cat "$OUTPUT")" "$RUN_URL")

# Exact match on the title, not GitHub's fuzzy `search`: a new advisory must not read as an old one
# sharing the words "cargo-deny" and "failed".
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
