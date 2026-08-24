#!/usr/bin/env bash
# Give an issue back, or clean up after its PR merged.
#
# Usage:
#   scripts/issue-release.sh 42            # done: PR merged, tidy up
#   scripts/issue-release.sh 42 --abandon  # not done: unclaim so someone else can take it
#   scripts/issue-release.sh --stale       # release claims whose worktree is gone
set -euo pipefail

REPO_ROOT="${POSTIO_MAIN_CHECKOUT:-$HOME/src/postio}"
WORKTREES="${POSTIO_WORKTREES:-$HOME/src/postio-worktrees}"
CLAIMS="${POSTIO_CLAIMS:-$HOME/.cache/postio/claims}"

if [ "${1:-}" = "--stale" ]; then
    found=0
    for claim in "$CLAIMS"/issue-*; do
        [ -d "$claim" ] || continue
        num=$(basename "$claim" | sed 's/^issue-//')
        if [ ! -d "$WORKTREES/issue-$num" ]; then
            rmdir "$claim" 2>/dev/null && echo "released stale claim on #$num" && found=1
        fi
    done
    [ "$found" = 0 ] && echo "no stale claims."
    exit 0
fi

NUM="${1:?usage: issue-release.sh <issue-number> [--abandon]}"
ABANDON=0
[ "${2:-}" = "--abandon" ] && ABANDON=1
TREE="$WORKTREES/issue-$NUM"

if [ -d "$TREE" ]; then
    # Never discard work silently. An abandoned branch with commits on it is
    # recoverable; a removed worktree with uncommitted changes is not.
    if [ -n "$(git -C "$TREE" status --porcelain 2>/dev/null)" ]; then
        echo "$TREE has uncommitted changes. Commit or push them first:" >&2
        git -C "$TREE" status --short >&2
        exit 1
    fi
    BRANCH=$(git -C "$TREE" rev-parse --abbrev-ref HEAD)
    git -C "$REPO_ROOT" worktree remove "$TREE"
    echo "removed $TREE"
    # Only after the worktree is gone: git refuses to delete a branch a
    # worktree still has checked out, which is why this order matters.
    git -C "$REPO_ROOT" branch -D "$BRANCH" >/dev/null 2>&1 \
        && echo "deleted local branch $BRANCH"
fi

rmdir "$CLAIMS/issue-$NUM" 2>/dev/null || true

if [ "$ABANDON" = 1 ]; then
    gh issue edit "$NUM" --remove-assignee @me --remove-label in-progress >/dev/null
    gh issue comment "$NUM" --body "Unclaimed by an agent session. Any pushed branch is still on origin."
    echo "#$NUM released and available again."
else
    gh issue edit "$NUM" --remove-label in-progress >/dev/null 2>&1 || true
    echo "#$NUM cleaned up."
fi
