#!/usr/bin/env bash
# Claim a GitHub issue and get a private worktree to do it in.
#
# Finds work, claims it, and gives you a tree of your own to do it in.
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
#   scripts/issue-claim.sh --base feature/x   # cut from an initiative branch
#   scripts/issue-claim.sh --ready-label ready-mac   # a different queue
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib/require-gh.sh"

REPO_ROOT=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
WORKTREES="${POSTIO_WORKTREES:-$HOME/src/postio-worktrees}"
CLAIMS="${POSTIO_CLAIMS:-$HOME/.cache/postio/claims}"

# Which label marks an issue as claimable. `ready` for ordinary work; the
# macOS frontend initiative (#15) uses `ready-mac`, so that an ordinary Linux
# session skips its issues for free rather than by remembering to. Sessions run
# on several machines and the claim locks below are per-machine, so the label is
# the only thing keeping two hosts off the same work. #552.
READY_LABEL="${POSTIO_READY_LABEL:-ready}"

WANT=""; MILESTONE=""; LABEL=""; DRY=0; BASE="main"
while [ $# -gt 0 ]; do
    case "$1" in
        --milestone) MILESTONE="$2"; shift 2 ;;
        --label)     LABEL="$2";     shift 2 ;;
        --ready-label) READY_LABEL="$2"; shift 2 ;;
        --base)      BASE="$2";      shift 2 ;;
        --dry-run)   DRY=1;          shift ;;
        [0-9]*)      WANT="$1";      shift ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

# A base that does not exist would silently become a branch cut from nothing,
# so it is checked against the remote before anything is claimed. A typo here
# is an error, never a new branch.
if ! git -C "$REPO_ROOT" ls-remote --exit-code --heads origin "$BASE" >/dev/null 2>&1; then
    echo "No branch '$BASE' on origin, so there is nothing to cut from." >&2
    echo "Existing branches:" >&2
    git -C "$REPO_ROOT" ls-remote --heads origin \
        | sed 's|.*refs/heads/|    |' | head -20 >&2
    exit 2
fi

mkdir -p "$WORKTREES" "$CLAIMS"

# Candidates: open, labelled `$READY_LABEL`, unclaimed, and not blocked by anything
# still open. `epic`, `icebox`, `needs-architecture` and `needs-maintainer`
# are never agent work -- an epic is a container, an icebox item is deferred,
# needs-architecture means the architect (an agent, via /ux-architect) has to
# decide something first, and needs-maintainer means only the maintainer can.
args=(issue list --state open --limit 200
      --json number,title,labels,assignees,blockedBy,milestone)
[ -n "$MILESTONE" ] && args+=(--milestone "$MILESTONE")
[ -n "$LABEL" ]     && args+=(--label "$LABEL")

CANDIDATES=$(gh "${args[@]}" | WANT="$WANT" READY_LABEL="$READY_LABEL" python3 -c '
import json, os, re, sys

want = os.environ.get("WANT") or ""
ready = os.environ.get("READY_LABEL") or "ready"
rows = []
skipped = []
SKIP = {"epic", "icebox", "needs-architecture", "needs-maintainer", "in-progress", "blocked"}

for i in json.load(sys.stdin):
    names = {l["name"] for l in i["labels"]}
    if want:
        if str(i["number"]) == want:
            print(i["number"], i["title"], sep="\t")
        continue
    pri = next((n for n in names if re.fullmatch(r"p[0-9]", n)), "p9")
    num = i["number"]
    if ready not in names or names & SKIP:
        why = ", ".join(sorted(names & SKIP)) or ("not labelled " + ready)
        skipped.append((pri, "#%s (%s) skipped: %s" % (num, pri, why)))
        continue
    if i["assignees"]:
        skipped.append((pri, "#%s (%s) skipped: already claimed" % (num, pri)))
        continue
    if any(b.get("state") != "CLOSED" for b in i["blockedBy"].get("nodes", [])):
        skipped.append((pri, "#%s (%s) skipped: blocked" % (num, pri)))
        continue
    rows.append(i)

# Highest priority first, then oldest -- an issue that has been waiting is
# more likely to be blocking something than one filed this morning. Without
# this the order is whatever the API returned, which is newest-first, so a
# P3 filed today outranks a P1 filed last week.
def rank(i):
    p = next((l["name"] for l in i["labels"] if re.fullmatch(r"p[0-9]", l["name"])), "p9")
    return (p, i["number"])

ranked = sorted(rows, key=rank)

# Explain only the skips that outrank what we are about to take. An `epic` or
# `needs-architecture` issue is deliberately not claimable, so the top of the
# queue can be a P2 while three P0s sit above it -- which reads as the script
# choosing at random unless it says why. Skips at or below the chosen
# priority are noise.
if not want and ranked:
    top = rank(ranked[0])[0]
    louder = [line for pri, line in skipped if pri < top]
    for line in sorted(louder):
        print("note: " + line, file=sys.stderr)

for i in ranked:
    print(i["number"], i["title"], sep="\t")
')

if [ -z "$CANDIDATES" ]; then
    if [ -n "$WANT" ]; then
        echo "issue #$WANT is not open, or does not exist." >&2
    else
        echo "No \`$READY_LABEL\`, unblocked, unclaimed issues${MILESTONE:+ in milestone $MILESTONE}."
        echo "Stop here and say so -- do not go looking for work elsewhere."
    fi
    exit 1
fi

# `[^a-z0-9][^a-z0-9]*` rather than the `\+` it used to say: `\+` is a GNU
# extension, and BSD sed (macOS) matches it as a literal plus. The title then
# passed through untouched and git refused the ref -- "is not a valid branch
# name" -- after the claim lock had already been taken. #559.
slug() {
    printf '%s' "$1" | tr '[:upper:]' '[:lower:]' \
        | sed -e 's/[^a-z0-9][^a-z0-9]*/-/g' -e 's/^-//' -e 's/-$//' | cut -c1-40
}

while IFS=$'\t' read -r NUM TITLE; do
    [ -z "$NUM" ] && continue
    BRANCH="issue-$NUM-$(slug "$TITLE")"
    TREE="$WORKTREES/issue-$NUM"

    if [ "$DRY" = 1 ]; then
        # A dry run previews the real decision, and the real run never
        # adopts an existing worktree (#328) -- so a preview that names this
        # issue anyway is previewing a claim that would immediately refuse.
        # No lock to release here: dry-run never takes one.
        if [ -d "$TREE" ]; then
            echo "#$NUM already has a worktree at $TREE; not adopting it -- a session may be in it." >&2
            echo "trying the next candidate." >&2
            continue
        fi
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

    # Never adopt an existing worktree. A directory already at this path may
    # be another session's live tree -- a claim lock can go missing while its
    # session works (#328) -- and two sessions in one worktree trample each
    # other with the very commands that are safe everywhere else.
    if [ -d "$TREE" ]; then
        rmdir "$CLAIMS/issue-$NUM" 2>/dev/null || true
        echo "#$NUM already has a worktree at $TREE; not adopting it -- a session may be in it." >&2
        if [ -n "$WANT" ]; then
            echo "If it is truly abandoned, release it first (refuses if dirty):" >&2
            echo "    scripts/issue-release.sh $NUM" >&2
            exit 2
        fi
        echo "trying the next candidate." >&2
        continue
    fi
    git -C "$REPO_ROOT" fetch --quiet origin "$BASE"
    git -C "$REPO_ROOT" worktree add --quiet -b "$BRANCH" "$TREE" "origin/$BASE"
    # Recorded rather than retyped. `issue-land.sh` reads this back, so a
    # session that claimed from an initiative branch lands onto it without
    # having to remember a flag -- and forgetting *this* flag is a merge to
    # main, which is the one thing an initiative branch exists to prevent.
    # The worktree's private git dir, so it is per worktree (a shared repo
    # config would collide across sessions), untracked, and removed with the
    # worktree it describes. #290.
    printf '%s\n' "$BASE" > "$(git -C "$TREE" rev-parse --git-dir)/postio-base"
    # .cargo/config.toml points TMPDIR at target/tmp (relative), and nothing
    # else creates it in a fresh worktree -- without this, the first
    # tempfile::tempdir() in a test fails with NotFound (#178, #219).
    mkdir -p "$TREE/target/tmp"

    gh issue edit "$NUM" --add-assignee @me --add-label in-progress >/dev/null

    echo "claimed #$NUM  $TITLE"
    echo
    echo "  tree:   $TREE"
    echo "  branch: $BRANCH"
    echo "  target: this worktree's own (deps come from the machine-wide sccache, wired in automatically)"
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
