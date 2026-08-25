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
MAIN_CHECKOUT="${POSTIO_MAIN_CHECKOUT:-$HOME/src/postio}"
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

# Everything below compares against origin/main, and nothing here had been
# fetching it -- so the crate list, the PR body and the rebase were all reading
# whatever the last fetch happened to leave behind.
git fetch --quiet origin main

# Which crates actually changed, so the gates run over those rather than the
# whole workspace. CI proves the workspace; this proves your own work fast.
CRATES=$(git diff --name-only origin/main...HEAD; git status --porcelain \
         | sed 's/^...//') 
CRATES=$(printf '%s\n' $CRATES | sed -n 's|^crates/\([^/]*\)/.*|\1|p' | sort -u)

echo "issue:  #$ISSUE"
echo "branch: $BRANCH"
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
python3 scripts/check-crate-boundaries.py
python3 scripts/check-no-personal-data.py
python3 scripts/check-no-silent-tracking.py
python3 scripts/check-toolchain-pinned.py
python3 scripts/check-no-gtk-init-in-unit-tests.py
python3 scripts/check-runtime-crossings.py

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
    # Safe here in a way it never is in the shared checkout: this tree belongs
    # to this agent and nothing else is writing to it.
    git add -A
    git commit -m "$MSG

Refs: #$ISSUE"
else
    echo "no local changes to commit"
fi

# A clean tree is not the same question as an empty branch: the guard above
# only ever asked whether *this run* had something to commit. A branch that
# never had any work on it -- claimed and landed without a line changed --
# would otherwise sail through the push and open a PR with nothing in it.
AHEAD=$(git rev-list --count origin/main..HEAD)
if [ "$AHEAD" = 0 ]; then
    echo "Nothing to land: this branch has no commits beyond origin/main." >&2
    exit 2
fi

# Rebase onto current main before pushing. Other sessions land while you
# work -- four commits arrived during one recent piece of work -- and a branch
# built on a stale base means CI tests a combination that will never exist,
# the merge is a surprise, and the push can be rejected outright.
BEHIND=$(git rev-list --count HEAD..origin/main)
if [ "$BEHIND" -gt 0 ]; then
    echo "main moved $BEHIND commit(s) while you worked; rebasing onto it"
    # Both sides of the comparison are read before the rebase runs, because
    # after it the question cannot be asked honestly any more.
    BEFORE_HEAD=$(git rev-parse HEAD)
    BEFORE_SCRIPTS=$(git rev-parse "HEAD:scripts" 2>/dev/null || echo none)
    if ! git rebase origin/main; then
        git rebase --abort 2>/dev/null || true
        echo >&2
        echo "Rebase onto origin/main hit a conflict. Nothing was pushed." >&2
        echo "Resolve it here, then run this script again:" >&2
        echo "    git rebase origin/main" >&2
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

if gh pr view --json number >/dev/null 2>&1; then
    echo "PR already open for $BRANCH; the push updated it."
else
    TITLE=$(git log -1 --format=%s)
    gh pr create --base main --head "$BRANCH" --title "$TITLE" --body "$(cat <<BODY
$(git log origin/main..HEAD --format='- %s')

Closes #$ISSUE

🤖 Generated with [Claude Code](https://claude.com/claude-code)
BODY
)"
fi
URL=$(gh pr view --json url -q .url)
echo "$URL"

[ "$MERGE" = 1 ] || { echo "left open at your request (--no-merge)."; exit 0; }

# Watch, do not fire and forget. GitHub's own --auto would merge immediately
# here: it waits for *required* checks, branch protection is what makes a check
# required, and this repository cannot set any (private repo, free plan). So
# auto-merge would land the PR before CI had started.
echo
echo "--- waiting for checks ---"
# Not inline, because this is the step that decides whether an unreviewed
# commit reaches main and it needs a test of its own -- see
# scripts/test-wait-for-checks.py's stubbed `gh`. It exits non-zero when a
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
