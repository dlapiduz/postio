#!/usr/bin/env bash
# Seed a worktree's target/debug from the newest sibling's, so its first build compiles what changed
# (#1717). `issue-claim.sh` does this for claimed
# trees (#1102); this is the same seeding (lib/seed-target.sh) for a tree made another way, such as
# a spec branch or a parallel lane:
#
#     git worktree add ~/src/postio-worktrees/<name> -b <branch> origin/main
#     scripts/worktree-seed.sh ~/src/postio-worktrees/<name>
#
# A tree that already has a `target/debug` is left alone; `POSTIO_CLAIM_SEED=0` opts out.
#
# Usage: scripts/worktree-seed.sh <worktree>
# Exit status: 0 seeded, already built, or nothing to seed from (each said);
# 2 usage.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$HERE/lib/drop-workspace-artifacts.sh"
source "$HERE/lib/seed-target.sh"

TREE="${1:-}"
if [ -z "$TREE" ] || [ ! -d "$TREE" ]; then
    echo "usage: scripts/worktree-seed.sh <worktree>" >&2
    exit 2
fi
TREE="$(cd "$TREE" && pwd)"

REPO_ROOT=$(git -C "$TREE" rev-parse --path-format=absolute --git-common-dir)
REPO_ROOT="${REPO_ROOT%/.git}"
WORKTREES="${POSTIO_WORKTREES:-$HOME/src/postio-worktrees}"
COLD=0

mkdir -p "$TREE/target/tmp"

git -C "$TREE" config core.hooksPath .githooks

if [ -d "$TREE/target/debug" ]; then
    echo "target: $TREE already has target/debug; leaving its build alone"
    exit 0
fi
if [ "${POSTIO_CLAIM_SEED:-1}" = 0 ]; then
    echo "target: cold -- POSTIO_CLAIM_SEED=0"
    exit 0
fi

seed_target "$TREE"

if [ -n "$SEEDED" ]; then
    echo "target: seeded by copy from $SEEDED; only what differs rebuilds"
elif [ -n "$SEED_CANDIDATE" ]; then
    echo "target: cold -- every seed candidate's copy failed ($SEED_FAILURE)"
else
    echo "target: cold -- no sibling has a target/debug to seed from"
fi
