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
#   scripts/issue-claim.sh --reuse            # in this worktree, target warm
#   scripts/issue-claim.sh --ready-label ready-mac   # a different queue
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib/require-gh.sh"
source "$(dirname "${BASH_SOURCE[0]}")/lib/ready-labels.sh"

REPO_ROOT=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
WORKTREES="${POSTIO_WORKTREES:-$HOME/src/postio-worktrees}"
CLAIMS="${POSTIO_CLAIMS:-$HOME/.cache/postio/claims}"

# Which label marks an issue as claimable. `${READY_LABELS[0]}` (`ready`) for
# ordinary work; the macOS frontend initiative (#15) uses `ready-mac`, so
# that an ordinary Linux session skips its issues for free rather than by
# remembering to. Sessions run on several machines and the claim locks below
# are per-machine, so the label is the only thing keeping two hosts off the
# same work. #552. The default comes from `lib/ready-labels.sh`, the same
# list `issue-release.sh` strips on landing, so the two cannot disagree
# about which queues exist (#621) -- `--ready-label`/`$POSTIO_READY_LABEL`
# can still name anything, on purpose: a one-off queue nobody has
# bureaucratized yet is still claimable.
READY_LABEL="${POSTIO_READY_LABEL:-${READY_LABELS[0]}}"

WANT=""; MILESTONE=""; LABEL=""; DRY=0; BASE="main"; REUSE=0
# Why each candidate was passed over, so the message at the end can say which
# of the three it was rather than guessing (#1077).
SKIPPED_CLAIMED=""; SKIPPED_BRANCH=""; SKIPPED_WORKTREE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --milestone) MILESTONE="$2"; shift 2 ;;
        --label)     LABEL="$2";     shift 2 ;;
        --ready-label) READY_LABEL="$2"; shift 2 ;;
        --base)      BASE="$2";      shift 2 ;;
        --dry-run)   DRY=1;          shift ;;
        --reuse)     REUSE=1;        shift ;;
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

# --reuse: work the next issue in the worktree this session is already in,
# instead of a fresh one with a cold `target/`.
#
# The saving is the whole point -- a new worktree rebuilds Postio's own ~20
# crates before it can report a single gate, twelve minutes on #860's landing
# -- and the *reason this is safe* is that it is one workspace, not two.
# Sharing a `CARGO_TARGET_DIR` between worktrees is the p1 in #76: two trees
# present cargo the same relative paths and package versions, land in the same
# build slot, and hand each other stale libraries, so a suite can pass against
# a library the change never reached. One tree with a different branch checked
# out cannot do that, and the build stays warm because it is the same tree.
#
# Vetted here, before any claim is taken, so a refusal costs nothing and
# cannot strand a lock.
REUSE_TREE=""
if [ "$REUSE" = 1 ]; then
    REUSE_TREE="$(git rev-parse --show-toplevel 2>/dev/null || true)"
    if [ -z "$REUSE_TREE" ]; then
        echo "--reuse works in a git worktree, and this is not one." >&2
        exit 2
    fi
    case "$REUSE_TREE" in
        "$WORKTREES"/*) : ;;
        *)
            # Never the shared checkout: it is for coordination, other
            # sessions' uncommitted work lives in it, and its guard hook
            # refuses the destructive commands this would run there.
            echo "--reuse only reuses a worktree under $WORKTREES." >&2
            echo "This is $REUSE_TREE, which is not one -- claim without --reuse." >&2
            exit 2
            ;;
    esac
    if [ -n "$(git -C "$REUSE_TREE" status --porcelain)" ]; then
        echo "$REUSE_TREE has uncommitted changes, so there is nothing safe to do here." >&2
        echo "Uncommitted work is unprotected work: commit or land it first." >&2
        exit 2
    fi
    # Nothing unlanded. A reuse that carried away commits nobody merged would
    # lose them behind a branch switch, which is worse than a cold build.
    #
    # Two references, and this used to get both wrong (#1054).
    #
    # **Against the base this tree was cut from**, which `--base` records and
    # `issue-land.sh` reads back. Comparing against `main` regardless made
    # every initiative worktree read as holding unlanded work -- for ever,
    # since its commits are on the initiative branch by construction -- so
    # the saving was unavailable in exactly the case that needs it most: an
    # initiative is the longest run of consecutive claims anybody makes.
    #
    # **By patch id, not by sha.** `issue-land.sh` merges by rebase, so the
    # commit that lands has a different sha from the local one even when the
    # patch is identical, and `rev-list --count` therefore called every tree
    # unlanded the moment its work landed -- which is precisely when `/issue`
    # says to reuse it. `git cherry` prefixes a commit with `-` when its
    # patch is already upstream and `+` when it is not; only the `+` lines
    # are work that a branch switch would strand.
    REUSE_BASE="$(cat "$(git -C "$REUSE_TREE" rev-parse --git-dir)/postio-base" 2>/dev/null \
                  || echo main)"
    REUSE_BASE="${REUSE_BASE%%[[:space:]]*}"
    # A base that is gone from origin -- an initiative branch that was merged
    # and deleted -- leaves nothing to compare against, and "cannot tell"
    # must not read as "nothing to strand".
    if ! git -C "$REUSE_TREE" fetch --quiet origin "$REUSE_BASE" 2>/dev/null; then
        echo "$REUSE_TREE was cut from '$REUSE_BASE', which origin no longer has." >&2
        echo "Nothing can be proven landed against a base that is gone, so this" >&2
        echo "refuses rather than guessing. Claim without --reuse, or correct" >&2
        echo "the record:" >&2
        echo "    printf 'main\n' > \"\$(git rev-parse --git-dir)/postio-base\"" >&2
        exit 2
    fi
    UNLANDED="$(git -C "$REUSE_TREE" cherry FETCH_HEAD HEAD 2>/dev/null \
                | grep -c '^+' || true)"
    if [ "${UNLANDED:-0}" -ne 0 ]; then
        echo "$REUSE_TREE holds $UNLANDED commit(s) that are not on $REUSE_BASE." >&2
        echo "Land them first -- reusing the tree now would leave them behind" >&2
        echo "a branch switch with nothing pointing at them." >&2
        exit 2
    fi
fi

mkdir -p "$WORKTREES" "$CLAIMS"

# The linker and C compiler `.cargo/config.toml` names are bare program
# names, and this is what makes them resolve (#1101). Guarded: the
# self-tests copy this script into a sandbox without its neighbours.
[ -x "$REPO_ROOT/scripts/install-shims.sh" ] && "$REPO_ROOT/scripts/install-shims.sh"

# Candidates: open, labelled `$READY_LABEL`, unclaimed, and not blocked by anything
# still open. `epic`, `icebox`, `needs-architecture` and `needs-maintainer`
# are never agent work -- an epic is a container, an icebox item is deferred,
# needs-architecture means the architect (an agent, via /ux-architect) has to
# decide something first, and needs-maintainer means only the maintainer can.
args=(issue list --state open --limit 200
      --json number,title,labels,assignees,blockedBy,milestone)
[ -n "$MILESTONE" ] && args+=(--milestone "$MILESTONE")
[ -n "$LABEL" ]     && args+=(--label "$LABEL")

# A function rather than a one-shot pipeline, because the self-heal below
# needs to run this exact query twice: once before the stale sweep, once
# after, with nothing free to drift between the two calls.
fetch_candidates() {
    gh "${args[@]}" | WANT="$WANT" READY_LABEL="$READY_LABEL" python3 -c '
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
'
}

CANDIDATES=$(fetch_candidates)

# The ready queue only ever shrinks unless something reclaims a dead
# session's claim: nothing else runs the stale sweep on its own, so a
# session that dies mid-work (crashes, runs out of context, gets
# interrupted) leaves its lock -- and its GitHub assignee and
# `in-progress` label -- exactly where it was, forever (#924). This is
# the one moment "nothing is ready" is about to become the answer, which
# makes it the right moment to check whether that is only true because of
# a lock nobody is behind any more, and the only moment: sweeping on every
# claim would mean every session pays for a check it almost never needs.
#
# `issue-release.sh --stale`'s own worktree-existence check is what keeps
# this safe -- a live claim is never touched, however quiet -- so this
# reuses that logic rather than duplicating its judgment about what counts
# as abandoned.
#
# Skipped for `--dry-run`: the sweep is a real mutation (it removes a
# GitHub assignee and a label), and a dry run's whole contract elsewhere in
# this script is that it previews and does not act. Skipped for a specific
# `WANT`ed issue too -- that failure means something else (not open, does
# not exist, or genuinely still claimed), not an empty ready queue.
if [ -z "$CANDIDATES" ] && [ -z "$WANT" ] && [ "$DRY" = 0 ]; then
    STALE_OUTPUT=$("$(dirname "${BASH_SOURCE[0]}")/issue-release.sh" --stale 2>&1) || true
    # The literal sentinel `issue-release.sh --stale` prints when it swept
    # nothing at all -- anything else means a lock was cleared or released,
    # which is worth a retry (and worth showing, since it is the reason the
    # answer below might now be different).
    if ! printf '%s\n' "$STALE_OUTPUT" | grep -q '^nothing to release\.$'; then
        echo "$STALE_OUTPUT" >&2
        CANDIDATES=$(fetch_candidates)
    fi
fi

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
        SKIPPED_CLAIMED="$SKIPPED_CLAIMED $NUM"
        continue
    fi
    # Cross-machine backstop: someone already pushed a branch for it.
    #
    # Claim locks are per-machine, so another host's live work is invisible
    # except as a branch. What the mere *existence* of one cannot tell apart
    # is that work from a branch whose commits already merged -- and
    # `issue-land.sh` deletes the branch it merges, so a leftover is usually
    # a landing killed in between, which this workstation does. Refusing on
    # existence alone made those issues permanently unclaimable (#1063).
    #
    # So the question is whether the branch holds unlanded work, by patch id:
    # a landing rebases, so the shas never match even when the content did
    # land, and `git cherry` prefixes `+` for a commit that is genuinely not
    # upstream and `-` for one that is. Only `+` is somebody's work.
    STALE_BRANCH="$(git -C "$REPO_ROOT" ls-remote --heads origin "issue-$NUM-*" 2>/dev/null \
                    | sed 's|.*refs/heads/||' | head -1 || true)"
    if [ -n "$STALE_BRANCH" ]; then
        BRANCH_UNLANDED=""
        if git -C "$REPO_ROOT" fetch --quiet --force origin \
            "+refs/heads/$BASE:refs/remotes/origin/$BASE" \
            "+refs/heads/$STALE_BRANCH:refs/postio-claim-check" 2>/dev/null; then
            BRANCH_UNLANDED="$(git -C "$REPO_ROOT" cherry \
                "origin/$BASE" refs/postio-claim-check 2>/dev/null | grep -c '^+' || true)"
            git -C "$REPO_ROOT" update-ref -d refs/postio-claim-check 2>/dev/null || true
        fi
        # Empty means the branch could not be fetched, so nothing about it is
        # known -- and "cannot tell" has to read as "somebody may be working
        # on this", which is the direction that costs time rather than work.
        if [ -z "$BRANCH_UNLANDED" ] || [ "$BRANCH_UNLANDED" -ne 0 ]; then
            rmdir "$CLAIMS/issue-$NUM" 2>/dev/null || true
            if [ -z "$BRANCH_UNLANDED" ]; then
                echo "#$NUM has the remote branch $STALE_BRANCH, which could not be" >&2
                echo "read -- assuming another session is on it." >&2
            else
                echo "#$NUM has $BRANCH_UNLANDED commit(s) on $STALE_BRANCH that are" >&2
                echo "not on $BASE. That is somebody's unlanded work, so this is the" >&2
                echo "case the backstop exists for." >&2
            fi
            SKIPPED_BRANCH="$SKIPPED_BRANCH $NUM"
            continue
        fi
        # Provably merged, so it is not a claim on anything. Deleted rather
        # than merely tolerated: the claim below cuts a branch of the same
        # name, and leaving the old one there would make the eventual push a
        # conflict nobody would connect to this moment.
        echo "#$NUM's branch $STALE_BRANCH is stale -- every commit on it is"
        echo "already on $BASE -- so it is not a claim on anything. Removing it."
        git -C "$REPO_ROOT" push origin --delete "$STALE_BRANCH" >/dev/null 2>&1 \
            || echo "  (could not delete it; it may already be gone)" >&2
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
        SKIPPED_WORKTREE="$SKIPPED_WORKTREE $NUM"
        continue
    fi
    git -C "$REPO_ROOT" fetch --quiet origin "$BASE"
    if [ "$REUSE" = 1 ]; then
        # Moved rather than left where it is, because the `issue-<n>` name is
        # load-bearing: `issue-release.sh` finds a tree by it. The move takes
        # `target/` along, which is the entire saving.
        if [ "$REUSE_TREE" != "$TREE" ]; then
            git -C "$REPO_ROOT" worktree move "$REUSE_TREE" "$TREE"
        fi
        git -C "$TREE" checkout --quiet -b "$BRANCH" "origin/$BASE"
    else
        git -C "$REPO_ROOT" worktree add --quiet -b "$BRANCH" "$TREE" "origin/$BASE"
    fi
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
    if [ "$REUSE" = 1 ]; then
        echo "  target: reused, already warm (moved from $REUSE_TREE)"
    else
        echo "  target: this worktree's own (deps come from the machine-wide sccache, wired in automatically)"
    fi
    echo
    echo "Work in that directory from here on:  cd $TREE"
    echo "Land it with:                         scripts/issue-land.sh"
    echo
    echo "--------------------------------------------------------------"
    gh issue view "$NUM"
    exit 0
done <<< "$CANDIDATES"

# Why nothing was taken, in the words of what actually happened.
#
# This used to say "Every candidate was already claimed. Nothing to do." for
# all three reasons, and only one of them is that. The other two are
# recoverable, and the wording matters more than it looks: it is nearly the
# sentence `/issue` uses for the genuine stop condition, so a session that
# reads it stops with work still available (#1077, seen after a leftover
# branch blocked the top candidate while two dozen issues were free).
echo "Nothing was claimed. Why, per candidate:" >&2
[ -n "$SKIPPED_CLAIMED" ] && \
    echo "  claimed by another session:${SKIPPED_CLAIMED}" >&2
[ -n "$SKIPPED_WORKTREE" ] && \
    echo "  a worktree already exists:${SKIPPED_WORKTREE}" >&2
if [ -n "$SKIPPED_BRANCH" ]; then
    echo "  a branch on origin holds unlanded work:${SKIPPED_BRANCH}" >&2
    echo >&2
    echo "Those are the backstop doing its job: claim locks are per-machine," >&2
    echo "so another host's live work shows up only as a branch, and the" >&2
    echo "commits on these are not on $BASE. A branch whose work already" >&2
    echo "merged is no longer counted here -- it is removed and the issue" >&2
    echo "claimed (#1063), so anything left is worth looking at before" >&2
    echo "taking it:" >&2
    for blocked in $SKIPPED_BRANCH; do
        echo "    git log --oneline origin/$BASE..origin/issue-$blocked-*" >&2
    done
fi
echo >&2
if [ -z "$SKIPPED_BRANCH$SKIPPED_WORKTREE" ]; then
    echo "Every candidate is genuinely taken. Stop here and say so." >&2
else
    echo "This is NOT the \"nothing is ready\" case: some of the above are" >&2
    echo "recoverable, and other sessions free theirs as they finish." >&2
fi
exit 1
