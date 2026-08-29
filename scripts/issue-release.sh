#!/usr/bin/env bash
# Give an issue back, or clean up after its PR merged.
#
# Usage:
#   scripts/issue-release.sh 42            # done: PR merged, tidy up
#   scripts/issue-release.sh 42 --abandon  # not done: unclaim so someone else can take it
#   scripts/issue-release.sh --stale       # release claims abandoned for a day+
#   scripts/issue-release.sh --stale 3     # ... or a different number of days
#   scripts/issue-release.sh --stale 0     # no age check (you are sure)
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib/require-gh.sh"
source "$(dirname "${BASH_SOURCE[0]}")/lib/ready-labels.sh"

REPO_ROOT="${POSTIO_MAIN_CHECKOUT:-$HOME/src/postio}"
WORKTREES="${POSTIO_WORKTREES:-$HOME/src/postio-worktrees}"
CLAIMS="${POSTIO_CLAIMS:-$HOME/.cache/postio/claims}"

if [ "${1:-}" = "--stale" ]; then
    # A claim is not stale because it is quiet. A session can spend hours on
    # one issue -- reading, waiting on CI, running a suite -- and leave no
    # visible trace for most of it. Releasing live work is far worse than
    # leaving a label up a day too long, so age is checked as well as
    # artefacts, and the default is deliberately generous.
    DAYS="${2:-1}"
    found=0
    for claim in "$CLAIMS"/issue-*; do
        [ -d "$claim" ] || continue
        num=$(basename "$claim" | sed 's/^issue-//')

        # A live worktree is a live claim, whatever the labels say. The
        # in-progress label can go missing while a session works (a failed
        # `gh issue edit` at claim time, a relabel, another sweep), and
        # clearing this lock on that evidence is what once let a second
        # session claim its way into the first one's tree. #328.
        [ -d "$WORKTREES/issue-$num" ] && continue

        # A lock whose issue is no longer claimed is left-over machinery from
        # work that finished: harmless-looking, and it makes issue-claim.sh
        # refuse that issue forever. Always safe to drop, regardless of age.
        state=$(gh issue view "$num" --json state,labels \
            --jq '"\(.state) \([.labels[].name] | join(","))"' 2>/dev/null || echo "")
        if [ -n "$state" ] && ! printf '%s' "$state" | grep -q "in-progress"; then
            rmdir "$claim" 2>/dev/null || true
            echo "cleared orphaned lock on #$num (no longer claimed)"
            found=1
            continue
        fi

        git -C "$REPO_ROOT" ls-remote --exit-code --heads origin "issue-$num-*" \
            >/dev/null 2>&1 && continue

        if [ "$DAYS" != 0 ]; then
            # When the claim was made, not when the issue last changed: a
            # comment from someone else must not look like progress.
            since=$(gh api "repos/{owner}/{repo}/issues/$num/timeline" \
                --jq '[.[] | select(.event=="labeled" and .label.name=="in-progress")]
                      | last | .created_at' 2>/dev/null)
            if [ -n "$since" ] && [ "$since" != "null" ]; then
                # python3 rather than `date -d`: `-d` is GNU, and BSD date
                # (macOS) rejects it outright. Every check in scripts/checks/
                # already needs python3, so this adds no dependency. #559.
                age=$(python3 -c 'import datetime, sys
since = datetime.datetime.fromisoformat(sys.argv[1].replace("Z", "+00:00"))
now = datetime.datetime.now(datetime.timezone.utc)
print(int((now - since).total_seconds()) // 86400)' "$since")
                if [ "$age" -lt "$DAYS" ]; then
                    echo "#$num claimed ${age}d ago — leaving it (needs ${DAYS}d)"
                    continue
                fi
            fi
        fi

        rmdir "$claim" 2>/dev/null || true
        gh issue edit "$num" --remove-assignee @me --remove-label in-progress \
            >/dev/null 2>&1 || true
        echo "released abandoned claim on #$num"
        found=1
    done
    [ "$found" = 0 ] && echo "nothing to release."
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
    # Every queue label, not just `ready`: a closed issue wearing one reads
    # as claimable work on every board and label query that does not also
    # check the state (#328) -- `ready-mac` fell through this exact gap
    # until #621 gave it one shared list with `issue-claim.sh` to read from.
    # `gh issue edit --remove-label` on a label the issue never had is a
    # harmless no-op, so trying every known queue needs no lookup first.
    remove_args=(--remove-label in-progress)
    for label in "${READY_LABELS[@]}"; do
        remove_args+=(--remove-label "$label")
    done
    gh issue edit "$NUM" "${remove_args[@]}" >/dev/null 2>&1 || true
    echo "#$NUM cleaned up."
    echo "(Work not actually landed? Release with --abandon instead -- this path assumes it did.)"
fi
