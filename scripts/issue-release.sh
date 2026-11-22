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

# A claim lock is an empty directory with `issue-<n>.owner` beside it (#1218, #1230), so `rmdir`
# alone drops it, as every older copy of these scripts does.
drop_lock() {
    rmdir "$1" 2>/dev/null || true
    rm -f "$1.owner" 2>/dev/null || true
}

if [ "${1:-}" = "--stale" ]; then
    # A claim is not stale because it is quiet: a session can spend hours on one issue, and
    # releasing live work is worse than a label a day too long.
    DAYS="${2:-1}"
    found=0
    for claim in "$CLAIMS"/issue-*; do
        [ -d "$claim" ] || continue
        num=$(basename "$claim" | sed 's/^issue-//')

        # A live worktree is a live claim whatever the labels say: the label can go missing while a
        # session works, which once let a second session into its tree (#328).
        [ -d "$WORKTREES/issue-$num" ] && continue

        # A lock whose issue is no longer claimed is left-over machinery that makes `issue-claim.sh`
        # refuse the issue forever; safe to drop at any age.
        state=$(gh issue view "$num" --json state,labels \
            --jq '"\(.state) \([.labels[].name] | join(","))"' 2>/dev/null || echo "")
        if [ -n "$state" ] && ! printf '%s' "$state" | grep -q "in-progress"; then
            drop_lock "$claim"
            echo "cleared orphaned lock on #$num (no longer claimed)"
            found=1
            continue
        fi

        git -C "$REPO_ROOT" ls-remote --exit-code --heads origin "issue-$num-*" \
            >/dev/null 2>&1 && continue

        if [ "$DAYS" != 0 ]; then
            # When the claim was made, not when the issue last changed: someone else's comment is
            # not progress.
            since=$(gh api "repos/{owner}/{repo}/issues/$num/timeline" \
                --jq '[.[] | select(.event=="labeled" and .label.name=="in-progress")]
                      | last | .created_at' 2>/dev/null)
            if [ -n "$since" ] && [ "$since" != "null" ]; then
                # python3 rather than `date -d`: `-d` is GNU and BSD date rejects it (#559).
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

        drop_lock "$claim"
        gh issue edit "$num" --remove-assignee @me --remove-label in-progress \
            >/dev/null 2>&1 || true
        echo "released abandoned claim on #$num"
        found=1
    done
    [ "$found" = 0 ] && echo "nothing to release."
    exit 0
fi

# Delete the issue's branch from `origin` if its landing did not get to it: `issue-land.sh` deletes
# it as its last step, a killed run leaves it, and `issue-claim.sh` refuses an issue with a remote
# branch, so a reopened issue would be unclaimable. `git cherry`, not "the issue is closed": only a
# patch-id comparison says whether this branch's commits are upstream; a `+` line is somebody's
# unlanded work and stays.
sweep_remote_branch() {
    local num=$1 base=$2 branch unlanded
    # `|| true`: there may be no `origin` (a sandbox), and pipefail would abort a cleanup that already worked.
    branch=$(git -C "$REPO_ROOT" ls-remote --heads origin "issue-$num-*" 2>/dev/null \
             | sed 's|.*refs/heads/||' | head -1 || true)
    [ -n "$branch" ] || return 0

    # Fetched to a ref of its own, not read off `FETCH_HEAD`: with two refs it resolves to the first
    # (the base), so every branch looked landed and unlanded work was deleted.
    local scratch="refs/postio-sweep/$branch"
    if ! git -C "$REPO_ROOT" fetch --quiet --force origin \
        "+refs/heads/$base:refs/remotes/origin/$base" \
        "+refs/heads/$branch:$scratch" 2>/dev/null; then
        echo "left $branch on origin: could not fetch it to check whether it landed." >&2
        return 0
    fi
    unlanded=$(git -C "$REPO_ROOT" cherry "origin/$base" "$scratch" 2>/dev/null \
               | grep -c '^+' || true)
    git -C "$REPO_ROOT" update-ref -d "$scratch" 2>/dev/null || true
    if [ "${unlanded:-0}" -ne 0 ]; then
        echo "left $branch on origin: $unlanded commit(s) on it are not on $base." >&2
        echo "That is unlanded work, and this branch may be the only copy." >&2
        return 0
    fi

    if git -C "$REPO_ROOT" push origin --delete "$branch" >/dev/null 2>&1; then
        echo "deleted the merged remote branch $branch"
    else
        echo "warning: could not delete $branch on origin -- it may already be gone." >&2
    fi
}

NUM="${1:?usage: issue-release.sh <issue-number> [--abandon]}"
ABANDON=0
[ "${2:-}" = "--abandon" ] && ABANDON=1
TREE="$WORKTREES/issue-$NUM"

if [ -d "$TREE" ]; then
    # Never discard work silently: an abandoned branch is recoverable, a removed worktree with
    # uncommitted changes is not.
    if [ -n "$(git -C "$TREE" status --porcelain 2>/dev/null)" ]; then
        echo "$TREE has uncommitted changes. Commit or push them first:" >&2
        git -C "$TREE" status --short >&2
        exit 1
    fi
    BRANCH=$(git -C "$TREE" rev-parse --abbrev-ref HEAD)
    # Read before the tree goes: the base is recorded in its git dir, and asking after the removal
    # would answer `main` for every initiative.
    BASE=$(cat "$(git -C "$TREE" rev-parse --git-dir)/postio-base" 2>/dev/null || echo main)
    git -C "$REPO_ROOT" worktree remove "$TREE"
    echo "removed $TREE"
    # Only after the worktree is gone: git refuses to delete a branch a worktree has checked out.
    git -C "$REPO_ROOT" branch -D "$BRANCH" >/dev/null 2>&1 \
        && echo "deleted local branch $BRANCH"
fi

drop_lock "$CLAIMS/issue-$NUM"

if [ "$ABANDON" = 1 ]; then
    gh issue edit "$NUM" --remove-assignee @me --remove-label in-progress >/dev/null
    gh issue comment "$NUM" --body "Unclaimed by an agent session. Any pushed branch is still on origin."
    echo "#$NUM released and available again."
else
    # Every queue label, not just `ready`: a closed issue wearing one reads as claimable work (#328,
    # #621). Removing a label the issue never had is a no-op.
    remove_args=(--remove-label in-progress)
    for label in "${READY_LABELS[@]}"; do
        remove_args+=(--remove-label "$label")
    done
    gh issue edit "$NUM" "${remove_args[@]}" >/dev/null 2>&1 || true
    sweep_remote_branch "$NUM" "${BASE:-main}"
    echo "#$NUM cleaned up."
    echo "(Work not actually landed? Release with --abandon instead -- this path assumes it did.)"
fi
