#!/usr/bin/env bash
# Seed a worktree's target/debug from the newest sibling's, so its first
# build compiles what changed rather than the world.
#
# `issue-claim.sh` has done this for claimed trees since #1102: on btrfs a
# reflinked copy of a warm sibling takes seconds and no disk, and the tree
# then rebuilds Postio's own crates in about a minute against 15 to 19 cold.
# A tree made any other way -- `git worktree add` for a spec branch, which
# is CLAUDE.md's own recipe, or for a parallel lane -- never got it: on
# 2026-09-30 three Focus lanes compiled from nothing at the exact commit of a
# warm 24 GB sibling (#1717). This is the same seeding, for any tree:
#
#     git worktree add ~/src/postio-worktrees/<name> -b <branch> origin/main
#     scripts/worktree-seed.sh ~/src/postio-worktrees/<name>
#
# The rules are the claim's, from the same code (lib/seed-target.sh): the
# newest sibling first, falling through to older ones if a copy fails; never
# the tree itself; never a sibling's live `target/tmp`; and the sibling's own
# crates dropped, since they carry its path. A tree that already has a
# `target/debug` is left alone -- seeding over it would discard its own
# build. `POSTIO_CLAIM_SEED=0` opts out, as it does for a claim.
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

# `.cargo/config.toml` points TMPDIR at target/tmp, and nothing else makes it.
mkdir -p "$TREE/target/tmp"

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
