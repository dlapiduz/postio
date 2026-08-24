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
#   scripts/issue-land.sh -m "..." --wip     # push without opening a PR
#   scripts/issue-land.sh --gates-only       # run the checks, commit nothing
set -euo pipefail

TREE=$(git rev-parse --show-toplevel)
BRANCH=$(git rev-parse --abbrev-ref HEAD)
MAIN_CHECKOUT="${POSTIO_MAIN_CHECKOUT:-$HOME/src/postio}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$MAIN_CHECKOUT/target}"

MSG=""; WIP=0; GATES_ONLY=0
while [ $# -gt 0 ]; do
    case "$1" in
        -m|--message) MSG="$2"; shift 2 ;;
        --wip)        WIP=1;    shift ;;
        --gates-only) GATES_ONLY=1; shift ;;
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
    # On its own compositor: postio-gtk's tests present real windows, and on a
    # live session they land on the maintainer's desktop and steal focus.
    scripts/test-headless.sh cargo test -p "$crate"
done

echo "--- repository invariants ---"
python3 scripts/check-crate-boundaries.py
python3 scripts/check-no-personal-data.py
python3 scripts/check-no-silent-tracking.py

# CI installs `rustup default stable`. When that is newer than this toolchain,
# lints exist there that cannot fire here -- which has already turned main red
# on an unused import nobody could reproduce locally.
echo "--- toolchain ---"
echo "local: $(rustc --version)"
echo "CI floats on stable; if these diverge, expect lints you cannot reproduce."

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
gh pr view --json url -q .url
