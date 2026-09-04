---
name: initiative
description: Run several interdependent issues on one feature branch — the developer loop with one decision made up front about branch shape, plus the registry-conflict and finishing rules that single issues never hit. Use when the children would leave main half-migrated if landed one at a time, or when an epic's children mostly depend on each other.
---

# Initiative

This is `/issue` with one extra decision, made once, before you cut a
worktree. Load `/issue` for the loop itself.

An initiative is several interdependent issues that would leave `main`
half-migrated if they landed one at a time. It gets a feature branch:

```bash
scripts/issue-claim.sh --base feature/<x> <n>
```

## Decide the shape first

Two questions, both answerable before you write any code:

**Will the children touch the same append-only registry?** If two of them
each add a `CommandId`, a `CommandSpec`, a row in `gtk_suite`'s `CASES`, or
regenerate `docs/keybindings.md`, they conflict — every time, in the same
files, over content that never actually disagrees.

**How many are genuinely parallel?** Count the ones that need an earlier one
merged before they can even build. If most do, you do not have a parallel
initiative. You have a sequence, and you are about to pay a land chain for
each step of it.

Either answer points one way: one branch, a commit per child, one landing.
#1000 was about 70% sequential and ran as six branches: ten CI runs for six
issues, four rebases, and the two of those rebases resolved in bulk both
broke.

File every child before you write code, with acceptance criteria you could
hand to somebody else. You will make scope calls later — a widget that turns
out to be another widget with the buttons removed, a criterion that needs
plumbing nobody has built — and the issue is where those go. Not the commit
body, not your head. An issue closing while overstating what is on screen is
the failure to avoid.

## While it runs

**Resolve registry conflicts by item, never by hunk.** "Both sides added a
thing, keep both" is correct at statement granularity and wrong at item
granularity: concatenating the hunks fused two `CommandSpec` bodies into one
struct with `id` twice, and two test functions into one with an unclosed
brace. Neither was visible in the diff. Take one item at a time, then
`cargo check -p postio-core` before `git rebase --continue`.

**Run the suite that can fail for what you changed.** `cargo check
--workspace --all-targets` proves the workspace compiles; it does not run
the tests that *enumerate* things. Touch one `CommandId` and five files no
compiler checks go stale — the golden binding table, two generated docs, the
config vocabulary. `cargo nextest run -p postio-core -p postio-config` is
seconds and catches all of it. Ten minutes into a CI run is the expensive
place to learn this.

**Never `git stash` in a worktree.** `refs/stash` is one ref per repository,
not per worktree, so it lands on the same stack every other session is
using. Commit the work in progress instead — a commit marked as such costs
nothing and cannot collide.

**Rebase the feature branch onto `main` regularly.** The tooling will not do
it for you, and a branch 359 ahead and 486 behind is effectively dead.

## Finishing is its own step

The children do not close when they merge into the feature branch: GitHub
honours `Closes:` only into the default branch. When the branch is whole,
merge `origin/main` *into* it — merge, not rebase, because the branch is
shared — resolve, verify on the merged tree, push, and open one PR to main
naming every child. They close with it. The epic is yours to close by hand.
