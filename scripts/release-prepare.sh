#!/usr/bin/env bash
# Make the release pull request for version X.Y.Z (#1714): a branch off `origin/main` with the
# notes, the bump and the refreshed lockfile in one commit; `issue-land.sh` opens the PR, and
# merging it is the release. It cannot run in the workflow: an automated push to `main` cannot pass
# the ruleset and a PR opened by Actions' token triggers no CI. The notes are docs/releases/X.Y.Z.md
# (also the AppStream changelog, rendered once from it): write it first and land it for words of
# your own, else this drafts it from commit subjects. To reword a draft, delete the branch and run
# again.
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

# `cargo metadata` rewrites Cargo.lock's internal crate versions without compiling.
cargo metadata --format-version 1 > /dev/null

git add -A
git commit -q -F - <<COMMIT
chore(release): v${VERSION}

The version, the AppStream changelog entry, the macOS bundle's
version and the release notes, moved together by
scripts/release-prepare.sh. The notes are
${NOTES}.
Merging this publishes v${VERSION}: release.yml releases whenever
main names a version that has no tag yet.
COMMIT

cat <<NEXT

Prepared ${BRANCH}: $(git log -1 --format=%s).

  1. Read ${NOTES} -- it is the release page and the changelog entry.
  2. scripts/issue-land.sh --detach      # opens the pull request
  3. When it merges, release.yml builds every package in
     .github/release-variants.json and publishes v${VERSION}.
NEXT
