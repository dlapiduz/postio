#!/usr/bin/env bash
# File an issue, after checking whether it has already been filed.
#
# The claim side of this workflow is scripted and the file side was not,
# which is plausibly the whole reason one bug ended up with three issue
# numbers in two days (#332, #392, #406). Two sessions independently filed a
# duplicate straight off a fresh reproduction -- the state where searching
# first feels redundant, because you are not wondering whether the bug exists,
# you just watched it happen.
#
# So this searches first and *stops* when it finds something, rather than
# asking a question nobody is at the keyboard to answer. The friction is one
# extra command, and only in the case where there is prior art to read.
#
# Usage:
#   scripts/issue-file.sh --title "..." --body-file notes.md [--label ready,p2]
#   scripts/issue-file.sh --title "..." --body "..."
#   scripts/issue-file.sh --title "..." --body-file notes.md --anyway
#   scripts/issue-file.sh --search-only --title "..."
#
# Exit status:
#   0  filed, or --search-only found nothing
#   1  a usage problem, or `gh` failed
#   2  possible duplicates found and nothing was filed
set -euo pipefail

TITLE=""
BODY=""
BODY_FILE=""
LABELS=""
ANYWAY=0
SEARCH_ONLY=0
EXTRA=()

while [ $# -gt 0 ]; do
    case "$1" in
        --title)       TITLE="${2:-}"; shift 2 ;;
        --body)        BODY="${2:-}"; shift 2 ;;
        --body-file)   BODY_FILE="${2:-}"; shift 2 ;;
        --label)       LABELS="${2:-}"; shift 2 ;;
        --anyway)      ANYWAY=1; shift ;;
        --search-only) SEARCH_ONLY=1; shift ;;
        *)             EXTRA+=("$1"); shift ;;
    esac
done

if [ -z "$TITLE" ]; then
    echo "usage: scripts/issue-file.sh --title \"...\" --body-file <path>" >&2
    exit 1
fi

# The words worth searching on: long enough to be distinctive, and not the
# ones every title in a mail client's tracker contains. Three of them, because
# GitHub's issue search ANDs its terms -- six would match nothing and report a
# clean sheet, which is the one failure this must not have.
#
# `postio` and `issue` are dropped for the same reason a search engine drops
# "the": in this repository they carry no information at all.
read_terms() {
    printf '%s\n' "$2" \
        | tr '[:upper:]' '[:lower:]' \
        | tr -c '[:alnum:]._-' '\n' \
        | grep -vE '^(the|and|for|that|with|from|this|when|what|does|not|but|its|has|are|was|were|will|into|then|than|only|also|postio|issue|issues)$' \
        | awk 'length($0) >= 4' \
        | head -n "$1" \
        | tr '\n' ' ' \
        | sed 's/ *$//'
}

# `--state all` on purpose. The duplicate that started this was filed against
# an issue that was already *closed*, which is the common case: a bug that was
# fixed once and came back does not announce itself as familiar, and a search
# that only looked at open issues would have found nothing both times.
search() {
    gh issue list --search "$1" --state all --limit 10 \
        --json number,state,title \
        --jq '.[] | "  #\(.number) \(.state)\t\(.title)"' 2>/dev/null || true
}

# Three terms, then two, then one. GitHub ANDs its search terms, so three is
# precise and misses a prior issue that said the same thing in different
# words -- which is not hypothetical: #332 called this bug "issue-land's merge
# verification can false-negative" and #392 called it "reports MERGE DID NOT
# LAND", and three terms from either finds neither of the other.
#
# Widening only when the narrower search found *nothing* costs one extra call
# in the case where there was nothing to find, and buys recall exactly when
# recall is the thing that failed. It is still a heuristic: this makes prior
# art likely to surface, never certain.
FOUND=""
TERMS=""
for width in 3 2 1; do
    TERMS=$(read_terms "$width" "$TITLE")
    [ -n "$TERMS" ] || continue
    echo "searching for: $TERMS"
    FOUND=$(search "$TERMS")
    [ -n "$FOUND" ] && break
done
if [ -z "$TERMS" ]; then
    # A title with nothing distinctive in it. Searching on nothing would
    # return everything or nothing, and either answer is a lie, so this says
    # so rather than pretending it checked.
    echo "note: no distinctive words in the title; searched for nothing." >&2
fi

if [ -n "$FOUND" ]; then
    echo
    echo "already filed, or close enough to read first:"
    printf '%s\n' "$FOUND"
    echo
fi

if [ "$SEARCH_ONLY" = 1 ]; then
    [ -z "$FOUND" ] && echo "nothing similar."
    exit 0
fi

if [ -n "$FOUND" ] && [ "$ANYWAY" != 1 ]; then
    cat >&2 <<'EOF_DUPLICATE'
Nothing was filed.

If one of those is the same thing, comment on it instead -- a new occurrence
on an existing issue is worth more than a second issue, because it is evidence
the bug survived a fix or has come back under new conditions:

    gh issue comment <n> --body "..."
    gh issue reopen <n>          # if it is closed and should not be

If yours is genuinely different, say so and file it:

    scripts/issue-file.sh --anyway ...
EOF_DUPLICATE
    exit 2
fi

# Nothing similar, or the caller has read what there was and said so.
ARGS=(--title "$TITLE")
if [ -n "$BODY_FILE" ]; then
    ARGS+=(--body-file "$BODY_FILE")
elif [ -n "$BODY" ]; then
    ARGS+=(--body "$BODY")
else
    echo "usage: one of --body or --body-file is required to file" >&2
    exit 1
fi
[ -n "$LABELS" ] && ARGS+=(--label "$LABELS")
[ ${#EXTRA[@]} -gt 0 ] && ARGS+=("${EXTRA[@]}")

gh issue create "${ARGS[@]}"
