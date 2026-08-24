#!/usr/bin/env bash
# Claim a GitHub issue and get a private worktree to do it in.
#
# Replaces `bd ready` + `bd update --claim` + working in the shared tree.
#
# The point of the worktree is that it makes the whole "Working in parallel"
# hazard table in CLAUDE.md moot. Inside it, `git add -A`, `git commit -a`,
# `git stash` and `cargo fmt --all` are all safe, because nothing else is
# editing those files. Agents stop needing to stay inside one crate: the work
# lands on a branch and merges through a PR.
#
# Usage:
#   scripts/issue-claim.sh                    # take the next ready issue
#   scripts/issue-claim.sh 42                 # take issue 42 specifically
#   scripts/issue-claim.sh --milestone MVP    # only from a milestone
#   scripts/issue-claim.sh --label area:compose
#   scripts/issue-claim.sh --dry-run          # show what it would take
set -euo pipefail

REPO_ROOT=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
WORKTREES="${POSTIO_WORKTREES:-$HOME/src/postio-worktrees}"
CLAIMS="${POSTIO_CLAIMS:-$HOME/.cache/postio/claims}"

WANT=""; MILESTONE=""; LABEL=""; DRY=0
while [ $# -gt 0 ]; do
    case "$1" in
        --milestone) MILESTONE="$2"; shift 2 ;;
        --label)     LABEL="$2";     shift 2 ;;
        --dry-run)   DRY=1;          shift ;;
        [0-9]*)      WANT="$1";      shift ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

mkdir -p "$WORKTREES" "$CLAIMS"

# Candidates: open, labelled `ready`, unclaimed, and not blocked by anything
# still open. `epic`, `icebox` and `needs-architecture` are never agent work --
# an epic is a container, an icebox item is deferred, and needs-architecture
# means a human has to decide something first.
args=(issue list --state open --limit 200
      --json number,title,labels,assignees,blockedBy,milestone)
[ -n "$MILESTONE" ] && args+=(--milestone "$MILESTONE")
[ -n "$LABEL" ]     && args+=(--label "$LABEL")

CANDIDATES=$(gh "${args[@]}" | WANT="$WANT" python3 -c '
import json, os, sys

want = os.environ.get("WANT") or ""
SKIP = {"epic", "icebox", "needs-architecture", "in-progress", "blocked"}

for i in json.load(sys.stdin):
    names = {l["name"] for l in i["labels"]}
    if want:
        if str(i["number"]) == want:
            print(i["number"], i["title"], sep="\t")
        continue
    if "ready" not in names or names & SKIP:
        continue
    if i["assignees"]:
        continue
    if any(not b.get("closed", False) for b in i["blockedBy"].get("nodes", [])):
        continue
    print(i["number"], i["title"], sep="\t")
')

if [ -z "$CANDIDATES" ]; then
    if [ -n "$WANT" ]; then
        echo "issue #$WANT is not open, or does not exist." >&2
    else
        echo "No ready, unblocked, unclaimed issues${MILESTONE:+ in milestone $MILESTONE}."
        echo "Stop here and say so -- do not go looking for work elsewhere."
    fi
    exit 1
fi

slug() {
    printf '%s' "$1" | tr '[:upper:]' '[:lower:]' \
        | sed -e 's/[^a-z0-9]\+/-/g' -e 's/^-//' -e 's/-$//' | cut -c1-40
}

while IFS=$'\t' read -r NUM TITLE; do
    [ -z "$NUM" ] && continue
    BRANCH="issue-$NUM-$(slug "$TITLE")"
    TREE="$WORKTREES/issue-$NUM"

    if [ "$DRY" = 1 ]; then
        echo "would claim #$NUM  $TITLE"
        echo "  branch: $BRANCH"
        echo "  tree:   $TREE"
        exit 0
    fi

    # Atomic claim. mkdir either creates the directory or fails; there is no
    # window between the check and the create. Every agent runs on this one
    # machine, so a local lock is a real lock -- assignee cannot be one,
    # because every session authenticates as the same GitHub user.
    if ! mkdir "$CLAIMS/issue-$NUM" 2>/dev/null; then
        echo "#$NUM is claimed by another session, trying the next one." >&2
        continue
    fi
    # Cross-machine backstop: someone already pushed a branch for it.
    if git -C "$REPO_ROOT" ls-remote --exit-code --heads origin "issue-$NUM-*" >/dev/null 2>&1; then
        rmdir "$CLAIMS/issue-$NUM" 2>/dev/null || true
        echo "#$NUM already has a remote branch, trying the next one." >&2
        continue
    fi

    git -C "$REPO_ROOT" fetch --quiet origin main
    if [ -d "$TREE" ]; then
        echo "reusing existing worktree $TREE"
    else
        git -C "$REPO_ROOT" worktree add --quiet -b "$BRANCH" "$TREE" origin/main
    fi

    gh issue edit "$NUM" --add-assignee @me --add-label in-progress >/dev/null

    echo "claimed #$NUM  $TITLE"
    echo
    echo "  tree:   $TREE"
    echo "  branch: $BRANCH"
    echo "  target: shared with the main checkout (deps stay warm)"
    echo
    echo "Work in that directory from here on:  cd $TREE"
    echo "Land it with:                         scripts/issue-land.sh"
    echo
    echo "--------------------------------------------------------------"
    gh issue view "$NUM"
    exit 0
done <<< "$CANDIDATES"

echo "Every candidate was already claimed. Nothing to do." >&2
exit 1
