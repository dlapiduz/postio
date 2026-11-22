#!/usr/bin/env bash
# File an issue, after checking whether it has already been filed. Searching first and stopping when
# something is found
# prevents duplicates filed straight off a fresh reproduction (#332, #392, #406).
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

source "$(dirname "${BASH_SOURCE[0]}")/lib/require-gh.sh"

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

# The words worth searching on: distinctive, three at most, because GitHub's issue search ANDs its
# terms and six would match nothing.
# `postio` and `issue` are dropped: in this repository they carry no information.
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

# `--state all` on purpose: a bug that was fixed once and came back matches a closed issue.
search() {
    gh issue list --search "$1" --state all --limit 10 \
        --json number,state,title \
        --jq '.[] | "  #\(.number) \(.state)\t\(.title)"' 2>/dev/null || true
}

# Three terms, then two, then one: three is precise but misses a prior issue worded differently
# (#332 vs #392). Widening only when
# the narrower search found nothing costs one extra call. A heuristic: prior art becomes likely to
# surface, never certain.
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
    # A title with nothing distinctive: searching on nothing returns everything or nothing, so say
    # so.
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
