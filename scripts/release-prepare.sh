#!/usr/bin/env bash
# Make the release pull request for version X.Y.Z.
#
# A release is now a version bump that lands like any other change (#1714):
# this makes a branch off `origin/main` with the notes, the bump and the
# refreshed lockfile in one commit; `issue-land.sh` opens the pull request;
# merging it is the release, because `release.yml` publishes whenever `main`
# names a version that has no tag yet.
#
# It used to happen inside the workflow, on a copy of the tree nobody
# committed -- an automated push to `main` cannot pass the ruleset, and a pull
# request opened by Actions' own token does not trigger CI -- so `main` said
# 0.3.0 while v0.4.2 was out, and the AppStream changelog stopped at 0.3.0.
# Run here, the PR is opened with your token and CI runs on it like anything
# else.
#
# The notes are docs/releases/X.Y.Z.md: the release page, and the AppStream
# changelog entry the bump renders from them. Write that file first and land it
# on main (the branch is cut from origin/main) if you want words of your own;
# otherwise this drafts it from the commit subjects since the last tag. To
# reword a draft, delete the branch, write the file, and run this again --
# the changelog entry is rendered once, from whatever the file says then.
#
# Usage: scripts/release-prepare.sh X.Y.Z
#   From a clean linked worktree -- never the shared checkout, where switching
#   branches would move every other session's ground. Refuses a version that
#   is not newer than the newest tag, before changing anything.
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib/release.sh"

VERSION="${1:-}"
if ! printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "usage: scripts/release-prepare.sh X.Y.Z   (got '${VERSION}')" >&2
    exit 2
fi

if [ "$(git rev-parse --git-dir)" = "$(git rev-parse --git-common-dir)" ]; then
    echo "This is the shared checkout. Run this from a worktree of your own:" >&2
    echo "    git worktree add ~/src/postio-worktrees/release-${VERSION} origin/main" >&2
    exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
    echo "The tree has uncommitted changes; commit or move them first:" >&2
    git status --short >&2
    exit 1
fi

git fetch -q origin main --tags

NEWEST=$(release_newest_tag)
if [ -n "$NEWEST" ] && [ "$(release_as_number "$VERSION")" -le "$(release_as_number "$NEWEST")" ]; then
    echo "${VERSION} is not newer than v${NEWEST}, the newest release." >&2
    exit 1
fi

BRANCH="chore/release-v${VERSION}"
git switch -q -c "$BRANCH" origin/main

NOTES="docs/releases/${VERSION}.md"
if [ -s "$NOTES" ]; then
    echo "notes: ${NOTES}, as written"
else
    mkdir -p docs/releases
    release_draft_notes "$NEWEST" > "$NOTES"
    [ -s "$NOTES" ] || echo "- Maintenance release" > "$NOTES"
    echo "notes: drafted ${NOTES} from $(wc -l < "$NOTES" | tr -d ' ') commit subject(s) since v${NEWEST:-<none>}"
fi

python3 scripts/release-bump.py "$VERSION" --notes-file "$NOTES"

# The internal crates' versions are in Cargo.lock; `metadata` rewrites it
# without compiling anything.
cargo metadata --format-version 1 > /dev/null

git add -A
git commit -q -F - <<COMMIT
chore(release): v${VERSION}

The version, the AppStream changelog entry, the macOS bundle's version
and the notes in ${NOTES}, moved together by scripts/release-prepare.sh.
Merging this publishes v${VERSION}: release.yml releases whenever main
names a version that has no tag yet.
COMMIT

cat <<NEXT

Prepared ${BRANCH}: $(git log -1 --format=%s).

  1. Read ${NOTES} -- it is the release page and the changelog entry.
  2. scripts/issue-land.sh --detach      # opens the pull request
  3. When it merges, release.yml builds every package in
     .github/release-variants.json and publishes v${VERSION}.
NEXT
