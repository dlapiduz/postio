#!/usr/bin/env bash
# Reclaims worktrees nothing will miss (#1428, #1460).
#
#   scripts/worktree-reap.sh            # report: what would go, what stays, and why
#   scripts/worktree-reap.sh --reap     # remove what the report says can go
#   scripts/worktree-reap.sh --days 3   # a tree is stale after 3 quiet days (default 1)
#   scripts/worktree-reap.sh --merged-hours 6   # rule 2's quiet (default 2)
#
# Fifty-two worktrees once held a 475 GB disk at 100%, each keeping its own
# 11 GB target/ -- deliberately, since #76 forbids sharing one -- and more
# than thirty of them held branches whose every commit had landed hours or
# days before. Nothing removed them: `issue-claim.sh` creates trees and
# `issue-release.sh` removes one only when a session explicitly stops, and
# sessions mostly end some other way. A landing on the full disk then died
# reporting a compile error, or SIGBUS (#1428, #1460).
#
# Four rules, checked in this order; the first that applies wins:
#
#   1. A tree with uncommitted changes is reported and never touched.
#   2. A clean tree whose branch's pull request has merged, quiet for
#      --merged-hours (default 2), loses only target/. Squash merges put the
#      work on main as a commit whose patch matches none of the branch's, so
#      rule 3 below calls it unlanded for ever -- and the disk filled twice
#      on 2026-09-24 with exactly these trees, each keeping 30 GB of build
#      for work that had already merged. Hours rather than a day, because a
#      session that has landed and moved on stops touching its tree at once;
#      not minutes, because one that has just landed often reuses the tree
#      for its next issue, build and all.
#   3. A tree with commits not on its base keeps them and loses only
#      target/ -- the expensive part, and the regenerable one.
#   4. A clean tree whose every commit is upstream is build output and goes
#      whole: the worktree, its local branch, and its claim lock.
#
# "Upstream" is by patch id (`git cherry`) against the base the tree was cut
# from -- `postio-base` in its git dir, `main` otherwise -- because
# `issue-land.sh` merges by rebase and a sha comparison calls every landed
# branch unlanded for ever. A base origin no longer has cannot be reasoned
# about, and such a tree is kept.
#
# Quiet means nothing has touched the tree's git index, HEAD or landing log
# for --days. A session between commits touches those every few minutes, so
# a day of silence is a session that is gone, not one that is thinking. The
# tree this runs from is never a candidate, and the main checkout is not a
# worktree and is never listed.
#
# This runs when asked. `issue-claim.sh` prints the report before seeding a
# fresh tree onto a disk below its floor, which is the moment the space is
# wanted; putting `--reap` on a timer is the maintainer's call.
set -euo pipefail

REPO_ROOT=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
WORKTREES="${POSTIO_WORKTREES:-$HOME/src/postio-worktrees}"
CLAIMS="${POSTIO_CLAIMS:-$HOME/.cache/postio/claims}"

REAP=0
DAYS=1
MERGED_HOURS=2
while [ $# -gt 0 ]; do
    case "$1" in
        --reap) REAP=1; shift ;;
        --days) DAYS="${2:?--days needs a number}"; shift 2 ;;
        --merged-hours) MERGED_HOURS="${2:?--merged-hours needs a number}"; shift 2 ;;
        -h|--help) sed -n '2,48p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

if [ ! -d "$WORKTREES" ]; then
    echo "no worktrees under $WORKTREES"
    exit 0
fi
WORKTREES="$(cd "$WORKTREES" && pwd -P)"
HERE="$(pwd -P)"
NOW=$(date +%s)

# Seconds since the epoch of the newest thing a working session touches.
last_touched() {
    local gitdir="$1" tree="$2" newest=0 file stamp
    for file in "$gitdir/index" "$gitdir/HEAD" "$gitdir/ORIG_HEAD" \
                "$gitdir/postio-land.log" "$tree/.git"; do
        [ -e "$file" ] || continue
        stamp=$(stat -c %Y "$file" 2>/dev/null || stat -f %m "$file")
        [ "$stamp" -gt "$newest" ] && newest=$stamp
    done
    echo "$newest"
}

# One fetch per base, however many trees were cut from it. A base that is
# gone from origin is remembered as such rather than retried.
FETCHED=""
UNREACHABLE=""
base_is_on_origin() {
    local base="$1"
    case " $FETCHED " in *" $base "*) return 0 ;; esac
    case " $UNREACHABLE " in *" $base "*) return 1 ;; esac
    if git -C "$REPO_ROOT" fetch --quiet origin "$base" 2>/dev/null; then
        FETCHED="$FETCHED $base"
        return 0
    fi
    UNREACHABLE="$UNREACHABLE $base"
    return 1
}

# Branches whose pull request has merged, one per line. Asked of GitHub
# once per run; POSTIO_MERGED_BRANCHES stands in for the answer, which is
# how the self-test runs with no network. No answer -- no `gh`, no network --
# is an empty list, and rule 2 then never applies: the others still do.
if [ -n "${POSTIO_MERGED_BRANCHES+set}" ]; then
    MERGED=$(printf '%b' "$POSTIO_MERGED_BRANCHES")
else
    MERGED=$(gh pr list --repo "$(git -C "$REPO_ROOT" remote get-url origin)" \
        --state merged --limit 300 --json headRefName --jq '.[].headRefName' 2>/dev/null || true)
fi
is_merged() {
    printf '%s\n' "$MERGED" | grep -qxF -- "$1"
}

# An upper bound, not a cost: on btrfs a seeded tree shares extents with the
# sibling it was reflinked from (#1102), and `du` counts a shared extent once
# per tree. Removing five trees `du` called 11 GB each freed about 5 GB each
# (docs/notes/2026-09-09-where-475-gigabytes-went.md).
size_of() {
    local size
    size=$(du -sh "$1" 2>/dev/null | cut -f1)
    echo "up to ${size:-0}"
}

kept=0
going=0
gone=0
while IFS= read -r tree; do
    [ -n "$tree" ] || continue
    # A tree whose directory is gone is git's to forget, not ours to judge.
    [ -d "$tree" ] || continue
    tree="$(cd "$tree" && pwd -P)"
    case "$tree" in "$WORKTREES"/*) ;; *) continue ;; esac
    name="${tree#"$WORKTREES"/}"
    case "$HERE" in "$tree"|"$tree"/*)
        echo "keep    $name: this is where you are"
        kept=$((kept + 1))
        continue
        ;;
    esac

    gitdir=$(git -C "$tree" rev-parse --absolute-git-dir)
    branch=$(git -C "$tree" rev-parse --abbrev-ref HEAD)

    if [ -n "$(git -C "$tree" --no-optional-locks status --porcelain)" ]; then
        echo "keep    $name: uncommitted changes -- never touched"
        kept=$((kept + 1))
        continue
    fi

    base=$(cat "$gitdir/postio-base" 2>/dev/null || echo main)
    base="${base%%[[:space:]]*}"
    if ! base_is_on_origin "$base"; then
        echo "keep    $name: cut from '$base', which origin no longer has, so nothing can be proven landed"
        kept=$((kept + 1))
        continue
    fi
    unlanded=$(git -C "$tree" cherry "origin/$base" HEAD 2>/dev/null | grep -c '^+' || true)

    quiet=$(( NOW - $(last_touched "$gitdir" "$tree") ))
    if is_merged "$branch"; then
        hours=$(( quiet / 3600 ))
        if [ "$hours" -lt "$MERGED_HOURS" ]; then
            echo "keep    $name: merged, touched ${hours}h ago (target/ goes after ${MERGED_HOURS}h)"
            kept=$((kept + 1))
            continue
        fi
        if [ -d "$tree/target" ]; then
            size=$(size_of "$tree/target")
            if [ "$REAP" = 1 ]; then
                rm -rf "$tree/target"
                echo "dropped $name/target ($size): its pull request merged"
                gone=$((gone + 1))
            else
                echo "target  $name: its pull request merged; would drop target/ ($size) and keep the rest"
                going=$((going + 1))
            fi
            continue
        fi
    fi

    age=$(( quiet / 86400 ))
    if [ "$age" -lt "$DAYS" ]; then
        echo "keep    $name: touched ${age}d ago (stale after ${DAYS}d)"
        kept=$((kept + 1))
        continue
    fi

    if [ "${unlanded:-0}" -ne 0 ]; then
        size=$(size_of "$tree/target")
        if [ "$REAP" = 1 ]; then
            rm -rf "$tree/target"
            echo "dropped $name/target ($size): $unlanded commit(s) not on origin/$base stay on $branch"
            gone=$((gone + 1))
        else
            echo "target  $name: $unlanded commit(s) not on origin/$base; would drop target/ ($size) and keep the rest"
            going=$((going + 1))
        fi
        continue
    fi

    size=$(size_of "$tree")
    if [ "$REAP" = 1 ]; then
        if git -C "$REPO_ROOT" worktree remove "$tree" 2>/dev/null; then
            git -C "$REPO_ROOT" branch -D "$branch" >/dev/null 2>&1 || true
            case "$name" in
                issue-[0-9]*) rm -rf "$CLAIMS/${name%%-[a-z]*}" 2>/dev/null || true ;;
            esac
            echo "removed $name ($size): every commit on origin/$base"
            gone=$((gone + 1))
        else
            echo "keep    $name: git would not remove it (see 'git worktree remove $tree')"
            kept=$((kept + 1))
        fi
    else
        echo "remove  $name: every commit on origin/$base and clean; would remove whole ($size)"
        going=$((going + 1))
    fi
done < <(git -C "$REPO_ROOT" worktree list --porcelain | awk '/^worktree /{ print substr($0, 10) }')

if [ "$REAP" = 1 ]; then
    echo "reaped $gone, kept $kept"
else
    echo "$going would go, $kept kept -- scripts/worktree-reap.sh --reap does it"
fi
