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
#   scripts/issue-land.sh -m "..." --wip        # push without opening a PR
#   scripts/issue-land.sh -m "..." --no-merge   # open the PR, do not wait
#   scripts/issue-land.sh --gates-only          # run the checks, commit nothing
#
# By default this waits for CI and merges. A PR nobody merges is work that
# looks finished and is not: it goes stale, it conflicts with whatever lands
# next, and the issue it closes stays open. The session that wrote it is the
# one that knows what to do if the checks fail, so it is the one that waits.
set -euo pipefail

TREE=$(git rev-parse --show-toplevel)
BRANCH=$(git rev-parse --abbrev-ref HEAD)
MAIN_CHECKOUT="${POSTIO_MAIN_CHECKOUT:-$HOME/src/postio}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$MAIN_CHECKOUT/target}"

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
echo "target: $CARGO_TARGET_DIR"
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

if [ -z "$MSG" ]; then
    echo "Nothing committed: pass -m \"<type>(<scope>): <summary>\"." >&2
    exit 2
fi

if [ -n "$(git status --porcelain)" ]; then
    # Safe here in a way it never is in the shared checkout: this tree belongs
    # to this agent and nothing else is writing to it.
    git add -A
    git commit -m "$MSG

Refs: #$ISSUE"
else
    echo "no local changes to commit"
fi

# Rebase onto current main before pushing. Other sessions land while you
# work -- four commits arrived during one recent piece of work -- and a branch
# built on a stale base means CI tests a combination that will never exist,
# the merge is a surprise, and the push can be rejected outright.
BEHIND=$(git rev-list --count HEAD..origin/main)
if [ "$BEHIND" -gt 0 ]; then
    echo "main moved $BEHIND commit(s) while you worked; rebasing onto it"
    if ! git rebase origin/main; then
        git rebase --abort 2>/dev/null || true
        echo >&2
        echo "Rebase onto origin/main hit a conflict. Nothing was pushed." >&2
        echo "Resolve it here, then run this script again:" >&2
        echo "    git rebase origin/main" >&2
        exit 1
    fi
    echo "rebased. The gates above ran on the previous base -- CI is what"
    echo "checks the combination, which is why this waits for it."
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
gh pr merge --rebase --delete-branch
echo
echo "merged and branch deleted."
echo "Now: scripts/issue-release.sh $ISSUE   (removes the worktree)"
echo "Then claim the next one -- finishing an issue is not finishing a session."
