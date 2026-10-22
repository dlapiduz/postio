#!/usr/bin/env bash
# How risky a diff is, from its paths alone: `high` or `normal`.
#
#     git diff --name-only origin/main...HEAD | scripts/risk-of-diff.sh
#     high
#     crates/postio-focus: the shared controller
#
# Line one is the answer. For `high`, line two names the rule that matched, so
# `issue-land.sh` can say why it asked for the whole nightly on the pull
# request (the `full-suite` label) without the author having asked.
#
# # Why these rules
#
# The sanity tier a pull request gets is fast on purpose, and the bugs here
# live between layers that each pass their own tests. Four GTK-only failures
# reached main in one session because nothing asked the integration suites
# about a change whose blast radius was wider than its diff. What widens it:
#
#   seeded mail    `seed.rs` and the `.eml` corpus are what every suite reads
#                  as "a mailbox"; a change moves every assertion at once
#   postio-focus   the one controller every frontend drives
#   migrations     the schema and its migrations: a wrong one corrupts a store
#                  that suites open fresh and never see migrated
#   postio-sync    the engine every account runs, with races no unit sees
#   3+ crates      a change that wide crosses seams nobody tested together
#
# Fails safe: an empty list is no evidence of a small diff, so it is `high`,
# the same direction `ci-changes.sh` fails.
#
# `scripts/tests/test-risk-of-diff.py` is what tests it.
set -euo pipefail

files=$(cat)
if [ -z "${files//[[:space:]]/}" ]; then
    printf 'high\nno diff to read: cannot show the change is small\n'
    exit 0
fi

high() { printf 'high\n%s\n' "$1"; exit 0; }

crates=""
while IFS= read -r file; do
    [ -n "$file" ] || continue
    case "$file" in
        crates/postio-storage/src/seed.rs) high "$file: the seeded demo mail" ;;
        *.eml)                             high "$file: fixture mail" ;;
        crates/postio-focus/*)             high "crates/postio-focus: the shared controller" ;;
        crates/postio-sync/*)              high "crates/postio-sync: the sync engine" ;;
        crates/postio-storage/src/schema.rs|crates/postio-storage/src/schema/*)
                                           high "$file: a storage migration" ;;
    esac
    crate=$(printf '%s' "$file" | sed -n 's|^crates/\([^/]*\)/.*|\1|p')
    [ -z "$crate" ] || crates=$(printf '%s\n%s' "$crates" "$crate")
done <<EOF_FILES
$files
EOF_FILES

count=$(printf '%s\n' "$crates" | sed '/^$/d' | sort -u | wc -l | tr -d ' ')
if [ "$count" -ge 3 ]; then
    high "$count crates touched (3 or more)"
fi
echo normal
