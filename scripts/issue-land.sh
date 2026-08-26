#!/usr/bin/env bash
# Verify, commit, push and open a PR for the issue this worktree is for.
#
# Replaces `/land` for GitHub-tracked work. The differences from the shared-tree
# ritual are all simplifications: this worktree is private, so staging
# everything is correct rather than dangerous, and there is no bead to close --
# `Closes #N` in the PR body closes the issue when it merges.
#
# Usage:
#   scripts/issue-land.sh -m "feat(gtk): teach the list to do the thing"
#   scripts/issue-land.sh                       # tree already committed; -m not needed
#   scripts/issue-land.sh -m "..." --wip        # push without opening a PR
#   scripts/issue-land.sh -m "..." --no-merge   # open the PR, do not wait
#   scripts/issue-land.sh --gates-only          # run the checks, commit nothing
#
# -m is only for uncommitted work: CLAUDE.md says commit as you go, so the
# ordinary case is a clean tree with the branch's commits already on it, and
# this must not demand a message it would only throw away. #120.
#
# By default this waits for CI and merges. A PR nobody merges is work that
# looks finished and is not: it goes stale, it conflicts with whatever lands
# next, and the issue it closes stays open. The session that wrote it is the
# one that knows what to do if the checks fail, so it is the one that waits.
#
# This script rebases the tree it lives in, so a rebase that brings in new
# landing machinery would otherwise be invisible to the very run that pulled
# it in. When that happens it hands over -- execs the copy the rebase brought
# in, from the top -- so the gates and the merge decision are both the new
# machinery's. See the rebase step below and #160.
set -euo pipefail

TREE=$(git rev-parse --show-toplevel)
BRANCH=$(git rev-parse --abbrev-ref HEAD)
# How many times one landing may hand over to a rebased copy of itself before
# it gives up rather than merging. Two is enough for the case this exists for
# -- machinery landing while a branch is being landed -- and a run that needs
# a third is one where main is moving faster than a landing takes, which a
# session should look at rather than a script should push through.
REEXEC_LIMIT="${POSTIO_LAND_REEXEC_LIMIT:-2}"

# Kept whole, because the handover below re-runs this script with exactly what
# this run was asked for; the loop underneath shifts them away.
ORIGINAL_ARGS=("$@")

MSG=""; WIP=0; GATES_ONLY=0; MERGE=1
while [ $# -gt 0 ]; do
    case "$1" in
        -m|--message) MSG="$2"; shift 2 ;;
        --wip)        WIP=1;    shift ;;
        --gates-only) GATES_ONLY=1; shift ;;
        --no-merge)   MERGE=0;      shift ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done

if [ "$BRANCH" = "main" ]; then
    echo "Refusing to run on main. This lands a branch from a worktree;" >&2
    echo "claim an issue first with scripts/issue-claim.sh." >&2
    exit 2
fi
ISSUE=$(printf '%s' "$BRANCH" | sed -n 's/^issue-\([0-9]\+\)-.*/\1/p')
if [ -z "$ISSUE" ]; then
    echo "Branch '$BRANCH' is not an issue branch (expected issue-<n>-<slug>)." >&2
    exit 2
fi

# Which branch this work belongs to, recorded by `issue-claim.sh` when the
# worktree was cut. Everything below -- the crate list, the rebase, the PR's
# base -- follows it. Defaulting to `main` is what keeps every ordinary
# landing exactly as it was; the alternative, a flag repeated on every
# command, is a flag somebody eventually forgets, and forgetting this one
# merges initiative work straight into `main`. #290.
BASE=$(cat "$(git rev-parse --git-dir)/postio-base" 2>/dev/null || echo main)
if ! git ls-remote --exit-code --heads origin "$BASE" >/dev/null 2>&1; then
    echo "This worktree records base '$BASE', which is not on origin any more." >&2
    echo "Nothing was pushed. If the initiative branch was merged and deleted," >&2
    echo "rebase this branch onto main and record that instead:" >&2
    echo "    printf 'main\n' > \"\$(git rev-parse --git-dir)/postio-base\"" >&2
    exit 2
fi

# Everything below compares against the base, and nothing here had been
# fetching it -- so the crate list, the PR body and the rebase were all reading
# whatever the last fetch happened to leave behind.
git fetch --quiet origin "$BASE"

# Which crates actually changed, so the gates run over those rather than the
# whole workspace. CI proves the workspace; this proves your own work fast.
CRATES=$(git diff --name-only "origin/$BASE...HEAD"; git status --porcelain \
         | sed 's/^...//') 
CRATES=$(printf '%s\n' $CRATES | sed -n 's|^crates/\([^/]*\)/.*|\1|p' | sort -u)

echo "issue:  #$ISSUE"
echo "branch: $BRANCH"
echo "base:   $BASE"
echo "crates: ${CRATES:-none}"
# #178 gave every worktree its own target/ because sharing one compiled a
# worktree's crate against a sibling's. Nothing defaults this any more: these
# gates are the run a merge is staked on, so they are the last place that
# should share artifacts with whatever else is landing right now. A caller who
# genuinely wants a directory of their own still gets it -- see #253 and
# docs/engineering-notes.md.
echo "target: ${CARGO_TARGET_DIR:-$TREE/target (this worktree)}"
echo

# rust-toolchain.toml pins the compiler, and RUSTUP_TOOLCHAIN in the
# environment beats it -- this workstation's mise config sets it, so a
# session builds, lints and tests on the wrong compiler while every gate here
# looks green. A warning in the log is weaker than the pin was supposed to
# give, so the value is captured for the diagnostic below and then cleared:
# every cargo invocation from here on runs on whatever rust-toolchain.toml
# names, whatever this shell exports. See docs/engineering-notes.md and #112.
HOST_RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-}"
unset RUSTUP_TOOLCHAIN

# This worktree is private, so formatting the whole thing is safe here. In the
# shared checkout it would reach into files another session has open, which is
# why CLAUDE.md forbids it there and permits it here.
echo "--- rustfmt ---"
cargo fmt --all

# Stage now, before the invariants below rather than after them: this tree
# is private, so staging is safe this early too. check-no-personal-data.py
# and three of its neighbours read `git ls-files`, which only sees tracked
# files -- so a file this session is adding was invisible to them until
# some later, unrelated branch happened to run the same check once it was
# already on `main`. That is exactly how #269's PNG got through unscanned,
# and would have done the same for a real leak. #270.
git add -A

for crate in $CRATES; do
    [ -d "$TREE/crates/$crate" ] || continue
    echo "--- clippy: $crate ---"
    cargo clippy -p "$crate" --all-targets -- -D warnings
    echo "--- test: $crate ---"
    # Headless without asking: .cargo/config.toml's runner puts every test
    # binary on a compositor of its own.
    cargo test -p "$crate"
done

echo "--- repository invariants ---"
"$TREE/scripts/check.sh"

# rust-toolchain.toml pins the compiler, so CI and this shell agree by
# construction -- the gates above ran with RUSTUP_TOOLCHAIN cleared, so
# `rustc --version` here reports the pinned compiler regardless of what this
# shell exports.
echo "--- toolchain ---"
echo "local: $(rustc --version)"
echo "pinned: $(sed -n 's/^channel *= *"\(.*\)"/\1/p' "$TREE/rust-toolchain.toml")"
if [ -n "$HOST_RUSTUP_TOOLCHAIN" ]; then
    echo "note: this shell exports RUSTUP_TOOLCHAIN=$HOST_RUSTUP_TOOLCHAIN, which" \
         "overrides rust-toolchain.toml -- cleared above, so the gates ran" \
         "pinned regardless."
fi

[ "$GATES_ONLY" = 1 ] && { echo; echo "gates passed; nothing committed."; exit 0; }

if [ -n "$(git status --porcelain)" ]; then
    if [ -z "$MSG" ]; then
        echo "Uncommitted changes: pass -m \"<type>(<scope>): <summary>\"," >&2
        echo "or commit them yourself first." >&2
        exit 2
    fi
    # Already staged, above -- before the invariants ran rather than here.
    git commit -m "$MSG

Refs: #$ISSUE"
else
    echo "no local changes to commit"
fi

# A clean tree is not the same question as an empty branch: the guard above
# only ever asked whether *this run* had something to commit. A branch that
# never had any work on it -- claimed and landed without a line changed --
# would otherwise sail through the push and open a PR with nothing in it.
AHEAD=$(git rev-list --count "origin/$BASE..HEAD")
if [ "$AHEAD" = 0 ]; then
    echo "Nothing to land: this branch has no commits beyond origin/$BASE." >&2
    exit 2
fi

# Rebase onto current main before pushing. Other sessions land while you
# work -- four commits arrived during one recent piece of work -- and a branch
# built on a stale base means CI tests a combination that will never exist,
# the merge is a surprise, and the push can be rejected outright.
BEHIND=$(git rev-list --count "HEAD..origin/$BASE")
if [ "$BEHIND" -gt 0 ]; then
    echo "$BASE moved $BEHIND commit(s) while you worked; rebasing onto it"
    # Both sides of the comparison are read before the rebase runs, because
    # after it the question cannot be asked honestly any more.
    BEFORE_HEAD=$(git rev-parse HEAD)
    BEFORE_SCRIPTS=$(git rev-parse "HEAD:scripts" 2>/dev/null || echo none)
    if ! git rebase "origin/$BASE"; then
        git rebase --abort 2>/dev/null || true
        echo >&2
        echo "Rebase onto origin/$BASE hit a conflict. Nothing was pushed." >&2
        echo "Resolve it here, then run this script again:" >&2
        echo "    git rebase origin/$BASE" >&2
        exit 1
    fi
    echo "rebased."

    # This script has just rebased the tree it lives in, so every line below
    # is being decided by a copy that no longer matches what is on disk. That
    # is how #50 merged a three-crate change without waiting for CI: the fix
    # that would have stopped it arrived in that run's own rebase, and the run
    # kept executing the copy bash already had open. It is worse than stale --
    # bash reads a script by byte offset as it goes, so a file rewritten
    # underneath it does not even reliably behave like the old version.
    #
    # So the run hands over: exec the machinery the rebase brought in, from
    # the top, with the same arguments. The commit is already made and the
    # tree is now zero behind, so the new run re-runs the gates against the
    # combination CI will actually see and skips straight past both -- which
    # is also the only way the gates and the merge decision can be talking
    # about the same tree. Nothing has been pushed at this point, so a
    # handover cannot double anything.
    #
    # The whole decision lives inside this `if`, which bash parsed before the
    # rebase touched anything: that is what makes it reachable at all.
    AFTER_SCRIPTS=$(git rev-parse "HEAD:scripts" 2>/dev/null || echo none)
    if [ "$BEFORE_SCRIPTS" != "$AFTER_SCRIPTS" ]; then
        DEPTH="${POSTIO_LAND_REEXEC_DEPTH:-0}"
        echo "the rebase brought in the landing machinery itself:"
        git diff --name-only "$BEFORE_HEAD" HEAD -- scripts/ | sed 's/^/    /'
        if [ "$DEPTH" -ge "$REEXEC_LIMIT" ] || [ ! -f "$TREE/scripts/issue-land.sh" ]; then
            echo >&2
            echo "Refusing to merge: after $DEPTH handover(s) the machinery is" >&2
            echo "still moving underneath this run, so it cannot say which copy" >&2
            echo "of itself would be deciding. Nothing was pushed. Run this" >&2
            echo "script again on the rebased tree -- #160." >&2
            exit 1
        fi
        echo "handing over to the landing machinery this rebase brought in (#160)."
        export POSTIO_LAND_REEXEC_DEPTH=$((DEPTH + 1))
        exec bash "$TREE/scripts/issue-land.sh" \
            ${ORIGINAL_ARGS[@]+"${ORIGINAL_ARGS[@]}"}
    fi

    echo "the gates above ran on the previous base -- CI is what checks the"
    echo "combination, which is why this waits for it."
fi

git push -u origin "$BRANCH"

[ "$WIP" = 1 ] && { echo; echo "pushed $BRANCH without a PR (work in progress)."; exit 0; }

# The *state*, not merely the existence, of a PR for this head branch.
# `gh pr view` resolves the most recent PR for the branch whatever state it is
# in, so a branch name that has been used before -- which
# `issue-claim.sh` makes likely, since it generates the name from the issue
# title and two sessions on one issue is the normal state of this repository --
# resolves to somebody else's *merged* PR. Adopting that as "already open"
# then merges nothing and reports success. #312.
PR_STATE=$(gh pr view --json state -q .state 2>/dev/null || echo "")
if [ "$PR_STATE" = "OPEN" ]; then
    echo "PR already open for $BRANCH; the push updated it."
    # It may have been opened before this worktree recorded a base -- or
    # against a different one entirely. Merging on that mismatch puts the work
    # on a branch nobody chose, so it stops here instead.
    OPEN_BASE=$(gh pr view --json baseRefName -q .baseRefName 2>/dev/null || echo "")
    if [ -n "$OPEN_BASE" ] && [ "$OPEN_BASE" != "$BASE" ]; then
        echo >&2
        echo "The open PR targets '$OPEN_BASE' and this worktree records" >&2
        echo "'$BASE'. Not merging: one of the two is wrong, and guessing" >&2
        echo "which would land the work somewhere nobody chose. Retarget the" >&2
        echo "PR, or correct the recorded base:" >&2
        echo "    printf '%s\\n' <branch> > \"\$(git rev-parse --git-dir)/postio-base\"" >&2
        exit 2
    fi
else
    if [ -n "$PR_STATE" ]; then
        # A closed or merged PR on this head branch: the name was reused.
        # Opening a new one is correct -- the alternative is adopting a PR
        # that is not this work's.
        echo "the previous PR for $BRANCH is $PR_STATE; opening a new one."
    fi
    TITLE=$(git log -1 --format=%s)
    gh pr create --base "$BASE" --head "$BRANCH" --title "$TITLE" --body "$(cat <<BODY
$(git log "origin/$BASE..HEAD" --format='- %s')

Closes #$ISSUE

🤖 Generated with [Claude Code](https://claude.com/claude-code)
BODY
)"
fi
URL=$(gh pr view --json url -q .url)
echo "$URL"

# What this branch is actually landing, recorded before the merge: `--rebase`
# gives every commit a new hash, so "did it land" cannot be asked by ancestry
# afterwards. Subjects survive a rebase; that is what gets checked. See the
# verification below.
LANDING=$(git log "origin/$BASE..HEAD" --format=%s)

[ "$MERGE" = 1 ] || { echo "left open at your request (--no-merge)."; exit 0; }

# Watch, do not fire and forget. GitHub's own --auto would merge immediately
# here: it waits for *required* checks, branch protection is what makes a check
# required, and this repository cannot set any (private repo, free plan). So
# auto-merge would land the PR before CI had started.
echo
echo "--- waiting for checks ---"
# Not inline, because this is the step that decides whether an unreviewed
# commit reaches main and it needs a test of its own -- see
# scripts/tests/test-wait-for-checks.py's stubbed `gh`. It exits non-zero when a
# check failed *or* when one was due and never registered; both mean the same
# thing here, which is do not merge.
if ! "$TREE/scripts/wait-for-checks.sh" "$URL"; then
    exit 1
fi

# Rebase, not squash. This history is linear and the project's convention is
# small focused commits; squashing a multi-commit branch throws away exactly
# the structure the commit rules exist to produce.
#
# Not --delete-branch: it deletes the *local* branch too, which makes `gh`
# switch this worktree off it first (to `main`, the PR's base) -- and `main`
# is permanently checked out in the shared checkout, so git refuses the
# checkout and `gh pr merge` reports failure even though the merge already
# went through on GitHub. #167. issue-release.sh already deletes the local
# branch once the worktree it belongs to is removed, so only the remote copy
# is left for this script to clean up, and that half never needed the local
# checkout at all.
gh pr merge --rebase

# Believe it only after checking. `gh pr merge` prints
# "! Pull request #N was already merged" and exits **0** when there is nothing
# to do, so the exit status alone once let this script announce "merged.",
# delete the remote branch, and leave the work existing nowhere but the local
# worktree -- which the line it prints next tells you to remove. #312.
#
# Ancestry cannot answer this: `--rebase` rewrites every commit, so the local
# tip is never an ancestor of the base even on complete success. Subjects
# survive, so they are what is compared, and `main` having moved on underneath
# is fine -- this asks whether the work arrived, not whether it is the tip.
#
# Asked repeatedly rather than once. `gh pr merge` returns as soon as GitHub
# *accepts* the merge, and the fetch below can still be answered before the
# new tip is visible -- which turned a few seconds of replication lag into
# "MERGE DID NOT LAND" twice in three landings (#406), telling the session to
# open a second PR for commits that were already on main. The state this
# guards against (#312) is permanent, not late, so waiting a little costs it
# nothing at all.
LANDED_TIMEOUT="${POSTIO_LANDED_TIMEOUT:-30}"
LANDED_POLL="${POSTIO_LANDED_POLL:-3}"
LANDED_DEADLINE=$(( $(date +%s) + LANDED_TIMEOUT ))

# The subjects of $LANDING that $1 does not carry, one per line.
missing_from() {
    while IFS= read -r subject; do
        [ -n "$subject" ] || continue
        printf '%s\n' "$1" | grep -Fqx -- "$subject" || printf '%s\n' "$subject"
    done <<EOF_LANDING
$LANDING
EOF_LANDING
}

while :; do
    git fetch -q origin "$BASE"
    MISSING=$(missing_from "$(git log "origin/$BASE" --format=%s)")
    [ -n "$MISSING" ] || break
    [ "$(date +%s)" -lt "$LANDED_DEADLINE" ] || break
    sleep "$LANDED_POLL"
done

if [ -n "$MISSING" ]; then
    printf '%s\n' "$MISSING" | while IFS= read -r subject; do
        echo "not on origin/$BASE: $subject" >&2
    done
    # What the PR itself says, because it is the one fact that tells "your
    # work is unlanded" from "the ref never arrived": a MERGED state here
    # means something went wrong with this check rather than with the merge,
    # and the instructions below would be the wrong thing to follow.
    STATE=$(gh pr view --json state --jq .state 2>/dev/null || echo "unknown")
    echo >&2
    echo "MERGE DID NOT LAND after ${LANDED_TIMEOUT}s. gh reported success and" >&2
    echo "origin/$BASE does not carry the commits above. The PR itself says:" >&2
    echo "    state: $STATE" >&2
    if [ "$STATE" = "MERGED" ]; then
        echo "which disagrees with $BASE. Fetch again before doing anything --" >&2
        echo "if the commits are there, the work landed and only this check" >&2
        echo "was wrong; release the worktree as usual." >&2
        exit 1
    fi
    echo "The remote branch $BRANCH is deliberately NOT deleted -- with the PR" >&2
    echo "merged elsewhere or closed, it may be the only copy of this work" >&2
    echo "besides this worktree." >&2
    echo >&2
    echo "Do not run issue-release.sh. Check the PR, then land onto a branch" >&2
    echo "name that is not already spoken for:" >&2
    echo "    git branch -m <a-new-name> && scripts/issue-land.sh" >&2
    exit 1
fi

echo
echo "merged."
if git push origin --delete "$BRANCH" 2>&1; then
    echo "remote branch deleted."
else
    echo "warning: could not delete the remote branch $BRANCH -- it may" >&2
    echo "already be gone. Not fatal: the merge above already succeeded." >&2
fi
echo "Now: scripts/issue-release.sh $ISSUE   (removes the worktree)"
echo "Then claim the next one -- finishing an issue is not finishing a session."
