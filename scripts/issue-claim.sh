#!/usr/bin/env bash
# Claim a GitHub issue and get a private worktree to do it in. Inside it `git add -A`, `git commit
# -a`, `git stash` and
# `cargo fmt --all` are safe, because nothing else edits those files.
#
# Usage:
#   scripts/issue-claim.sh                    # take the next ready issue
#   scripts/issue-claim.sh 42                 # take issue 42 specifically
#   scripts/issue-claim.sh --milestone MVP    # only from a milestone
#   scripts/issue-claim.sh --label area:compose
#   scripts/issue-claim.sh --dry-run          # show what it would take
#   scripts/issue-claim.sh --base feature/x   # cut from an initiative branch
#   scripts/issue-claim.sh --reuse            # in this worktree, target warm (strict)
#   scripts/issue-claim.sh --resume 42        # back to issue 42's branch: its PR went red
#   scripts/issue-claim.sh --fresh            # a new worktree even from inside one
#   scripts/issue-claim.sh --cold             # ...and do not seed its target/
#   scripts/issue-claim.sh --ready-label ready-mac   # a different queue
#
# Run from inside a landed worktree, a plain claim reuses it and falls back to a fresh tree, saying
# why, when reuse would
# strand something. A fresh tree's target/debug is seeded by copying the newest sibling's (#1102).
set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib/require-gh.sh"
source "$(dirname "${BASH_SOURCE[0]}")/lib/ready-labels.sh"
source "$(dirname "${BASH_SOURCE[0]}")/lib/drop-workspace-artifacts.sh"

REPO_ROOT=$(git -C "$(dirname "${BASH_SOURCE[0]}")/.." rev-parse --show-toplevel)
WORKTREES="${POSTIO_WORKTREES:-$HOME/src/postio-worktrees}"
# Resolved, because `git rev-parse --show-toplevel` resolves and the two are compared: on macOS
# `/var` is a symlink to `/private/var` (#1208).
[ -d "$WORKTREES" ] && WORKTREES="$(cd "$WORKTREES" && pwd -P)"
CLAIMS="${POSTIO_CLAIMS:-$HOME/.cache/postio/claims}"

# The label that marks an issue claimable. The macOS initiative uses `ready-mac`, so a Linux session
# skips its issues; claim locks
# are per-machine, so the label is what keeps two hosts off the same work (#552). The default comes
# from `lib/ready-labels.sh`,
# shared with `issue-release.sh` (#621); `--ready-label`/`$POSTIO_READY_LABEL` can name any queue.
READY_LABEL="${POSTIO_READY_LABEL:-${READY_LABELS[0]}}"

# A claim lock is a directory, so `mkdir` is the atomic take. Beside it, `issue-<n>.owner` names the
# worktree the claim created,
# so a refusal can say which tree holds an issue and `--resume` can tell a live claim from a
# finished session's lock (#1218).
# Beside the lock, not inside it (#1230): every worktree carries its own copy of these scripts, so a
# lock's format must be one every
# version can drop; an older release's `rmdir` failed on a file inside it and orphaned a lock.
drop_lock() {
    rmdir "$1" 2>/dev/null || true
    rm -f "$1.owner" 2>/dev/null || true
}

# The lock this run holds but has not finished earning. `mkdir` takes it before the worktree is cut,
# and under `set -e` any step between
# can end the script; nothing released it, so a failed claim left an issue permanently unclaimable
# (#1255).
HELD_LOCK=""

release_held_lock() {
    [ -n "$HELD_LOCK" ] || return 0
    drop_lock "$HELD_LOCK"
    HELD_LOCK=""
}
trap release_held_lock EXIT

# Whether the session behind a held lock is still there: a live worktree is a live claim (as
# `issue-release.sh --stale` judges).
# A lock with no `owner` predates #1218; the conventional path keeps it readable.
lock_is_live() {
    local lock="$1" num="$2" owner
    owner="$(cat "$lock.owner" 2>/dev/null || true)"
    if [ -n "$owner" ]; then
        [ -d "$owner" ]
    else
        [ -d "$WORKTREES/issue-$num" ]
    fi
}

WANT=""; MILESTONE=""; LABEL=""; DRY=0; BASE="main"; REUSE=0; FRESH=0; COLD=0
REUSE_IMPLICIT=0; RESUME=""
SKIPPED_CLAIMED=""; SKIPPED_BRANCH=""; SKIPPED_WORKTREE=""
while [ $# -gt 0 ]; do
    case "$1" in
        --milestone) MILESTONE="$2"; shift 2 ;;
        --label)     LABEL="$2";     shift 2 ;;
        --ready-label) READY_LABEL="$2"; shift 2 ;;
        --base)      BASE="$2";      shift 2 ;;
        --dry-run)   DRY=1;          shift ;;
        --reuse)     REUSE=1;        shift ;;
        --fresh)     FRESH=1;        shift ;;
        --cold)      COLD=1;         shift ;;
        --resume)    RESUME="$2";    shift 2 ;;
        [0-9]*)      WANT="$1";      shift ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

# A base that does not exist would silently become a branch cut from nothing, so it is checked
# against the remote first.
if ! git -C "$REPO_ROOT" ls-remote --exit-code --heads origin "$BASE" >/dev/null 2>&1; then
    echo "No branch '$BASE' on origin, so there is nothing to cut from." >&2
    echo "Existing branches:" >&2
    git -C "$REPO_ROOT" ls-remote --heads origin \
        | sed 's|.*refs/heads/|    |' | head -20 >&2
    exit 2
fi

# --resume <n>: a worktree for an issue whose branch is already on origin, because its PR went red
# after the session moved on (#1107).
# It tracks the existing branch so a fix lands on the same PR.
if [ -n "$RESUME" ]; then
    RESUME_BRANCH="$(git -C "$REPO_ROOT" ls-remote --heads origin "issue-$RESUME-*" 2>/dev/null \
                     | sed 's|.*refs/heads/||' | head -1 || true)"
    if [ -z "$RESUME_BRANCH" ]; then
        echo "No branch issue-$RESUME-* on origin; there is nothing to resume." >&2
        echo "Claim it normally instead: scripts/issue-claim.sh $RESUME" >&2
        exit 2
    fi
    RESUME_TREE="$WORKTREES/issue-$RESUME"
    # The tree is usually still there; whether it is somebody's is the lock's question, and a tree
    # nobody holds is reused (#1422).
    REUSE_TREE=0
    if [ -d "$RESUME_TREE" ]; then
        REUSE_TREE=1
    fi
    mkdir -p "$WORKTREES" "$CLAIMS"
    # No `|| true`: it walked through another session's live claim (#1218). But a lock whose worktree is gone is not live: the loop
    # lands, claims the next issue into the same tree and leaves this lock behind, so refusing on
    # the lock alone would break `--resume`.
    if ! mkdir "$CLAIMS/issue-$RESUME" 2>/dev/null; then
        if lock_is_live "$CLAIMS/issue-$RESUME" "$RESUME"; then
            HOLDER="$(cat "$CLAIMS/issue-$RESUME.owner" 2>/dev/null || true)"
            echo "#$RESUME is claimed by another session${HOLDER:+, working in $HOLDER}." >&2
            echo "Two sessions on one issue produce one branch name and two" >&2
            echo "implementations of it (#1177). Not resuming." >&2
            echo "If that session is gone: scripts/issue-release.sh --stale" >&2
            exit 2
        fi
    fi
    printf '%s\n' "$RESUME_TREE" > "$CLAIMS/issue-$RESUME.owner"
    git -C "$REPO_ROOT" fetch --quiet origin "$RESUME_BRANCH" "$BASE"
    if [ "$REUSE_TREE" = 1 ]; then
        # Fast-forward only: a tree that diverged from origin has work somebody should look at.
        HOLDING="$(git -C "$RESUME_TREE" rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
        if [ "$HOLDING" != "$RESUME_BRANCH" ]; then
            echo "$RESUME_TREE holds '${HOLDING:-nothing}', not $RESUME_BRANCH; not resuming into it." >&2
            echo "Move it aside, or: scripts/issue-release.sh $RESUME" >&2
            rm -f "$CLAIMS/issue-$RESUME.owner"; rmdir "$CLAIMS/issue-$RESUME" 2>/dev/null || true
            exit 2
        fi
        if ! git -C "$RESUME_TREE" merge --quiet --ff-only "origin/$RESUME_BRANCH" 2>/dev/null; then
            echo "$RESUME_TREE has diverged from origin/$RESUME_BRANCH; reconcile it by hand first." >&2
            rm -f "$CLAIMS/issue-$RESUME.owner"; rmdir "$CLAIMS/issue-$RESUME" 2>/dev/null || true
            exit 2
        fi
    else
        git -C "$REPO_ROOT" branch --quiet -D "$RESUME_BRANCH" 2>/dev/null || true
        git -C "$REPO_ROOT" worktree add --quiet --track -b "$RESUME_BRANCH" "$RESUME_TREE" "origin/$RESUME_BRANCH"
    fi
    # The base comes from the open PR, not `$BASE`: nobody passes `--base` to `--resume`, and
    # defaulting to `main` made `issue-land.sh`
    # refuse a branch claimed from `feature/x` after every gate had run (#1401). Without a PR,
    # `$BASE` is all there is.
    RESUME_BASE="$(gh pr list --head "$RESUME_BRANCH" --state open \
                   --json baseRefName --jq '.[0].baseRefName // empty' 2>/dev/null || true)"
    printf '%s\n' "${RESUME_BASE:-$BASE}" > "$(git -C "$RESUME_TREE" rev-parse --git-dir)/postio-base"
    mkdir -p "$RESUME_TREE/target/tmp"
    [ -x "$REPO_ROOT/scripts/install-shims.sh" ] && "$REPO_ROOT/scripts/install-shims.sh"
    gh issue edit "$RESUME" --add-assignee @me --add-label in-progress >/dev/null 2>&1 || true
    RESUME_PR="$(gh pr list --head "$RESUME_BRANCH" --state open --json number,url \
                 --jq '.[0] | "#\(.number) \(.url)"' 2>/dev/null || true)"
    echo "resumed #$RESUME on $RESUME_BRANCH${RESUME_PR:+ (PR $RESUME_PR)}"
    echo
    echo "  tree:   $RESUME_TREE"
    echo "  branch: $RESUME_BRANCH, tracking origin -- a push updates the PR"
    echo "  base:   ${RESUME_BASE:-$BASE}${RESUME_BASE:+ (from the PR)}"
    echo
    echo "Fix it there, then scripts/issue-land.sh --detach lands onto the same PR."
    exit 0
fi

# --reuse: work the next issue in the worktree this session is already in, instead of a fresh one
# with a cold `target/`
# (twelve minutes on #860). It is safe because it is one workspace: sharing a `CARGO_TARGET_DIR`
# between worktrees hands each other stale libraries (#76).
# Reuse is the default for a plain claim from inside a worktree under $WORKTREES (#1102); when reuse
# refuses (dirty tree, unlanded commits,
# base gone from origin) it says so and claims a fresh tree. `--reuse` stays strict, `--fresh` opts
# out, a dry run never moves anything.
if [ "$REUSE" = 0 ] && [ "$FRESH" = 0 ] && [ "$DRY" = 0 ]; then
    case "$(git rev-parse --show-toplevel 2>/dev/null || true)" in
        "$WORKTREES"/*) REUSE=1; REUSE_IMPLICIT=1 ;;
    esac
fi

# Vetted before any claim is taken, so a refusal cannot strand a lock. Every refusal prints its
# reason and returns 1.
REUSE_TREE=""
vet_reuse() {
    REUSE_TREE="$(git rev-parse --show-toplevel 2>/dev/null || true)"
    if [ -z "$REUSE_TREE" ]; then
        echo "--reuse works in a git worktree, and this is not one." >&2
        return 1
    fi
    case "$REUSE_TREE" in
        "$WORKTREES"/*) : ;;
        *)
            # Never the shared checkout: it holds other sessions' uncommitted work, and its guard
            # refuses the destructive commands this runs.
            echo "--reuse only reuses a worktree under $WORKTREES." >&2
            echo "This is $REUSE_TREE, which is not one -- claim without --reuse." >&2
            return 1
            ;;
    esac
    if [ -n "$(git -C "$REUSE_TREE" status --porcelain)" ]; then
        echo "$REUSE_TREE has uncommitted changes, so there is nothing safe to do here." >&2
        echo "Uncommitted work is unprotected work: commit or land it first." >&2
        return 1
    fi
    # Nothing unlanded: a reuse that carried away unmerged commits would lose them behind a branch
    # switch (#1054).
    # Against the base this tree was cut from (recorded by `--base`), not `main`, or initiative
    # worktrees always read as unlanded.
    # By patch id, not sha: `issue-land.sh` merges by rebase, so shas differ. `git cherry` prefixes
    # `-` for a patch already upstream
    # and `+` for one that is not; only `+` is work a branch switch would strand.
    REUSE_BASE="$(cat "$(git -C "$REUSE_TREE" rev-parse --git-dir)/postio-base" 2>/dev/null \
                  || echo main)"
    REUSE_BASE="${REUSE_BASE%%[[:space:]]*}"
    # A base gone from origin leaves nothing to compare against, and "cannot tell" must not read as
    # "nothing to strand".
    if ! git -C "$REUSE_TREE" fetch --quiet origin "$REUSE_BASE" 2>/dev/null; then
        echo "$REUSE_TREE was cut from '$REUSE_BASE', which origin no longer has." >&2
        echo "Nothing can be proven landed against a base that is gone, so this" >&2
        echo "refuses rather than guessing. Claim without --reuse, or correct" >&2
        echo "the record:" >&2
        echo "    printf 'main\n' > \"\$(git rev-parse --git-dir)/postio-base\"" >&2
        return 1
    fi
    UNLANDED="$(git -C "$REUSE_TREE" cherry FETCH_HEAD HEAD 2>/dev/null \
                | grep -c '^+' || true)"
    if [ "${UNLANDED:-0}" -ne 0 ]; then
        # Not stranded if every commit is on origin with an open PR: with auto-merge (#1107) the
        # landing returns before the merge.
        REUSE_BRANCH="$(git -C "$REUSE_TREE" rev-parse --abbrev-ref HEAD 2>/dev/null || true)"
        REUSE_REMOTE="$(git -C "$REUSE_TREE" rev-parse --verify --quiet "origin/$REUSE_BRANCH" 2>/dev/null || true)"
        REUSE_PR_STATE=""
        if [ -n "$REUSE_REMOTE" ] && [ "$REUSE_REMOTE" = "$(git -C "$REUSE_TREE" rev-parse HEAD)" ]; then
            REUSE_PR_STATE="$(gh pr view "$REUSE_BRANCH" --json state --jq .state 2>/dev/null || true)"
        fi
        if [ "$REUSE_PR_STATE" = "OPEN" ]; then
            echo "note: $REUSE_BRANCH is pushed with its PR open; auto-merge decides, and"
            echo "      scripts/issue-claim.sh --resume <n> comes back to it if CI says no."
        else
            echo "$REUSE_TREE holds $UNLANDED commit(s) that are not on $REUSE_BASE." >&2
            echo "Land them first -- reusing the tree now would leave them behind" >&2
            echo "a branch switch with nothing pointing at them." >&2
            return 1
        fi
    fi

    return 0
}
if [ "$REUSE" = 1 ] && ! vet_reuse; then
    if [ "$REUSE_IMPLICIT" = 1 ]; then
        echo "note: not reusing $REUSE_TREE for the reason above; claiming a fresh tree instead." >&2
        echo >&2
        REUSE=0; REUSE_TREE=""
    else
        exit 2
    fi
fi

mkdir -p "$WORKTREES" "$CLAIMS"

source "$(dirname "${BASH_SOURCE[0]}")/lib/seed-target.sh"

# What makes the bare `postio-linker` and `postio-cc` resolve (#1101). Guarded: the self-tests copy
# this script into a sandbox without its neighbours.
[ -x "$REPO_ROOT/scripts/install-shims.sh" ] && "$REPO_ROOT/scripts/install-shims.sh"

# Shared config, so one line covers every worktree; a tree whose base predates .githooks has no hook
# to run.
git -C "$REPO_ROOT" config core.hooksPath .githooks

# Candidates: open, labelled `$READY_LABEL`, unclaimed, not blocked by anything open. `epic`,
# `icebox`, `needs-architecture` and
# `needs-maintainer` are never agent work (a container, deferred, the architect's call, the
# maintainer's call).
args=(issue list --state open --limit 200
      --json number,title,labels,assignees,blockedBy,milestone)
[ -n "$MILESTONE" ] && args+=(--milestone "$MILESTONE")
[ -n "$LABEL" ]     && args+=(--label "$LABEL")

# A function because the self-heal below runs this exact query twice, before and after the stale
# sweep.
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
        # Naming an issue bypasses the label filters (the architect claims an `epic` by name), but
        # not a claim somebody else holds (#1218):
        # a rider claim (`gh issue edit --add-assignee`) takes no lock and was invisible here.
        if str(i["number"]) == want:
            marks = [m for m in (
                "assigned" if i["assignees"] else "",
                "labelled in-progress" if "in-progress" in names else "",
            ) if m]
            if marks:
                print("#%s is %s -- another session has it." % (
                    i["number"], " and ".join(marks)), file=sys.stderr)
                print("Working it anyway means two branches of one name and "
                      "the same fix twice.", file=sys.stderr)
                print("If that session is gone: scripts/issue-release.sh "
                      "--stale", file=sys.stderr)
            else:
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

# Highest priority first, then oldest; the API returns newest-first, so a P3 filed today would
# outrank a P1 filed last week.
def rank(i):
    p = next((l["name"] for l in i["labels"] if re.fullmatch(r"p[0-9]", l["name"])), "p9")
    return (p, i["number"])

ranked = sorted(rows, key=rank)

# Explain only the skips that outrank what we are about to take, or the queue reads as chosen at
# random. Skips at or below the chosen priority are noise.
if not want and ranked:
    top = rank(ranked[0])[0]
    louder = [line for pri, line in skipped if pri < top]
    for line in sorted(louder):
        print("note: " + line, file=sys.stderr)

for i in ranked:
    print(i["number"], i["title"], sep="\t")
'
}

# The caller's open PRs with a failing check, before new work is taken: with auto-merge nobody waits
# in front of a PR (#1107).
RED_PRS="$(gh pr list --author @me --state open --limit 50 \
            --json number,headRefName,url,statusCheckRollup 2>/dev/null \
    | python3 -c '
import json, re, sys
try:
    prs = json.load(sys.stdin)
except Exception:
    prs = []
for pr in prs:
    bad = [c.get("name") for c in pr.get("statusCheckRollup") or []
           if str(c.get("conclusion") or c.get("state") or "").upper() in ("FAILURE", "ERROR", "TIMED_OUT", "CANCELLED")]
    if not bad:
        continue
    m = re.match(r"issue-(\d+)-", pr.get("headRefName") or "")
    issue = m.group(1) if m else "?"
    print("note: PR #%s (issue-%s) has failing checks: %s -- scripts/issue-claim.sh --resume %s" % (pr["number"], issue, ", ".join(str(b) for b in bad), issue))
' 2>/dev/null || true)"
[ -n "$RED_PRS" ] && printf '%s\n\n' "$RED_PRS" >&2

CANDIDATES=$(fetch_candidates)

# The ready queue only shrinks unless something reclaims a dead session's lock, assignee and
# `in-progress` label (#924). This is the one
# moment "nothing is ready" is about to be the answer, so sweep here and only here.
# `issue-release.sh --stale`'s worktree-existence
# check keeps a live claim untouched.
# Skipped for `--dry-run` (the sweep mutates) and for a named `WANT`: that failure means something
# else.
if [ -z "$CANDIDATES" ] && [ -z "$WANT" ] && [ "$DRY" = 0 ]; then
    STALE_OUTPUT=$("$(dirname "${BASH_SOURCE[0]}")/issue-release.sh" --stale 2>&1) || true
    # The literal sentinel `issue-release.sh --stale` prints when it swept nothing; anything else
    # cleared a lock and is worth a retry.
    if ! printf '%s\n' "$STALE_OUTPUT" | grep -q '^nothing to release\.$'; then
        echo "$STALE_OUTPUT" >&2
        CANDIDATES=$(fetch_candidates)
    fi
fi

if [ -z "$CANDIDATES" ]; then
    if [ -n "$WANT" ]; then
        echo "issue #$WANT is not open, does not exist, or is already claimed" >&2
        echo "by another session -- see above if it said which." >&2
    else
        echo "No \`$READY_LABEL\`, unblocked, unclaimed issues${MILESTONE:+ in milestone $MILESTONE}."
        echo "Stop here and say so -- do not go looking for work elsewhere."
    fi
    exit 1
fi

# `[^a-z0-9][^a-z0-9]*` rather than `\+`, a GNU extension that BSD sed matches literally; git then
# refused the ref after the lock was taken (#559).
slug() {
    printf '%s' "$1" | tr '[:upper:]' '[:lower:]' \
        | sed -e 's/[^a-z0-9][^a-z0-9]*/-/g' -e 's/^-//' -e 's/-$//' | cut -c1-40
}

while IFS=$'\t' read -r NUM TITLE; do
    [ -z "$NUM" ] && continue
    BRANCH="issue-$NUM-$(slug "$TITLE")"
    TREE="$WORKTREES/issue-$NUM"

    if [ "$DRY" = 1 ]; then
        # A dry run previews the real decision, and the real run never adopts an existing worktree
        # (#328). No lock to release: dry-run takes none.
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

    # Atomic claim: `mkdir` creates the directory or fails. All agents share this machine, so a
    # local lock is real; an assignee cannot be one,
    # because every session is the same GitHub user.
    if ! mkdir "$CLAIMS/issue-$NUM" 2>/dev/null; then
        HOLDER="$(cat "$CLAIMS/issue-$NUM.owner" 2>/dev/null || true)"
        echo "#$NUM is claimed by another session${HOLDER:+ (working in $HOLDER)}, trying the next one." >&2
        SKIPPED_CLAIMED="$SKIPPED_CLAIMED $NUM"
        continue
    fi
    HELD_LOCK="$CLAIMS/issue-$NUM"
    printf '%s\n' "$TREE" > "$CLAIMS/issue-$NUM.owner"
    # Cross-machine backstop: someone already pushed a branch for it. Locks are per-machine, and
    # `issue-land.sh` deletes the branch it merges,
    # so a leftover is usually a killed landing; refusing on existence alone made issues unclaimable
    # (#1063).
    # So ask whether the branch holds unlanded work by patch id (a landing rebases): `git cherry`
    # shows `+` for a commit not upstream.
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
        # Empty means the branch could not be fetched; "cannot tell" must read as "somebody may be
        # working on this".
        if [ -z "$BRANCH_UNLANDED" ] || [ "$BRANCH_UNLANDED" -ne 0 ]; then
            release_held_lock
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
        # Provably merged, so not a claim. Deleted, since the claim cuts a branch of the same name
        # and the eventual push would conflict.
        echo "#$NUM's branch $STALE_BRANCH is stale -- every commit on it is"
        echo "already on $BASE -- so it is not a claim on anything. Removing it."
        git -C "$REPO_ROOT" push origin --delete "$STALE_BRANCH" >/dev/null 2>&1 \
            || echo "  (could not delete it; it may already be gone)" >&2
    fi

    # Never adopt an existing worktree: it may be another session's live tree, since a lock can go
    # missing while its session works (#328).
    if [ -d "$TREE" ]; then
        release_held_lock
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
        # Moved, not left: `issue-release.sh` finds a tree by its `issue-<n>` name, and the move
        # takes `target/` along.
        if [ "$REUSE_TREE" != "$TREE" ]; then
            git -C "$REPO_ROOT" worktree move "$REUSE_TREE" "$TREE"
        fi
        git -C "$TREE" checkout --quiet -b "$BRANCH" "origin/$BASE"
        # Moved, so every artifact of ours carries the old path (`env!("CARGO_MANIFEST_DIR")`); drop
        # them and keep the dependencies warm.
        drop_workspace_artifacts "$TREE/target"
        drop_swift_build "$TREE"
    else
        # A local branch with no worktree, left by reusing a worktree for another issue; `git
        # worktree add -b` refuses it (#1255).
        # Judged by patch id as the remote-branch backstop is (#1063): `git cherry` shows `+` for a
        # commit not upstream.
        if git -C "$REPO_ROOT" show-ref --verify --quiet "refs/heads/$BRANCH"; then
            LOCAL_UNLANDED="$(git -C "$REPO_ROOT" cherry "origin/$BASE" "$BRANCH" 2>/dev/null \
                              | grep -c '^+' || true)"
            if [ -n "$LOCAL_UNLANDED" ] && [ "$LOCAL_UNLANDED" -eq 0 ] 2>/dev/null; then
                echo "#$NUM's local branch $BRANCH holds nothing that is not already"
                echo "on $BASE, so it is not a claim on anything. Removing it."
                git -C "$REPO_ROOT" branch -D "$BRANCH" >/dev/null 2>&1 || true
            else
                # Somebody's work, or unreadable: not the moment to delete it; leave the issue for a
                # session that can look.
                release_held_lock
                echo "#$NUM has a local branch $BRANCH with ${LOCAL_UNLANDED:-unreadable}" >&2
                echo "commit(s) not on $BASE. That is unlanded work, so this claim" >&2
                echo "stops rather than cutting a worktree over it." >&2
                SKIPPED_BRANCH="$SKIPPED_BRANCH $NUM"
                continue
            fi
        fi
        # A seeded tree costs ~5 GB of real disk; seeding onto a disk that cannot hold it killed a
        # landing with a misleading compile error (#1428).
        # Below the floor the reaper's report says what holds the space (#1460). A warning, not a
        # refusal: the tree may still fit.
        CLAIM_FREE_GB=$(( $(df -Pk "$WORKTREES" 2>/dev/null | awk 'NR==2 { print $4 }' || echo 0) / 1024 / 1024 ))
        if [ "${CLAIM_FREE_GB:-0}" -lt "${POSTIO_CLAIM_DISK_FLOOR_GB:-16}" ]; then
            echo "warning: ${CLAIM_FREE_GB} GB free under $WORKTREES, and a seeded tree is about 5 GB of it." >&2
            # Reclaimed rather than reported: the reaper frees build output of finished work.
            if [ -x "$REPO_ROOT/scripts/worktree-reap.sh" ]; then
                "$REPO_ROOT/scripts/worktree-reap.sh" --reap >&2 || true
            fi
        fi
        git -C "$REPO_ROOT" worktree add --quiet -b "$BRANCH" "$TREE" "origin/$BASE"
        seed_target "$TREE"
    fi
    # Recorded so `issue-land.sh` lands onto an initiative branch without a flag; forgetting it is a
    # merge to main (#290).
    # In the worktree's private git dir: per worktree, untracked, removed with it.
    printf '%s\n' "$BASE" > "$(git -C "$TREE" rev-parse --git-dir)/postio-base"
    # `.cargo/config.toml` points TMPDIR at target/tmp and nothing else creates it in a fresh
    # worktree; without this the first tempdir fails with NotFound (#178, #219).
    mkdir -p "$TREE/target/tmp"

    gh issue edit "$NUM" --add-assignee @me --add-label in-progress >/dev/null

    # Earned: from here the lock is a claim, so the exit trap must not take it back.
    HELD_LOCK=""

    echo "claimed #$NUM  $TITLE"
    echo
    echo "  tree:   $TREE"
    echo "  branch: $BRANCH"
    if [ "$REUSE" = 1 ]; then
        echo "  target: reused, already warm (moved from $REUSE_TREE; Postio's own crates rebuild, the deps do not)"
    elif [ -n "$SEEDED" ]; then
        echo "  target: seeded by copy from $SEEDED; only what differs rebuilds"
    elif [ -n "$SEED_CANDIDATE" ]; then
        echo "  target: cold -- every seed candidate's copy failed ($SEED_FAILURE); falling back to a cold build (deps come from the machine-wide sccache)"
    else
        echo "  target: cold -- nothing to seed from (deps come from the machine-wide sccache)"
    fi
    echo
    echo "Work in that directory from here on:  cd $TREE"
    echo "Land it with:                         scripts/issue-land.sh"
    echo
    echo "--------------------------------------------------------------"
    gh issue view "$NUM"
    exit 0
done <<< "$CANDIDATES"

# Why nothing was taken, in the words of what happened: only one of three reasons is "already
# claimed", and the other two are recoverable.
# The old single wording read like `/issue`'s genuine stop condition, so sessions stopped with work
# available (#1077).
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
