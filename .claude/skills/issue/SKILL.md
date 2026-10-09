---
name: issue
description: Take a GitHub issue and work it end to end in a private worktree — claim it, branch, build, verify, commit, push, open a PR that closes it. This is how all work in this repository starts. Run it whenever you need work, and again the moment a PR is open.
---

# Work a GitHub issue

```bash
scripts/issue-claim.sh                    # next ready issue -> its own worktree
cd ~/src/postio-worktrees/issue-<n>       # work there from now on
scripts/issue-land.sh --detach            # gates, commit, push, PR, auto-merge
scripts/issue-land.sh --status            # what the landing did
scripts/issue-claim.sh                    # from inside the worktree: the next one
```

## 1. Claim

```bash
scripts/issue-claim.sh 42                   # a specific issue
scripts/issue-claim.sh --milestone MVP      # scoped to a milestone
scripts/issue-claim.sh --label opus         # only issues sized for this model
scripts/issue-claim.sh --dry-run            # look before taking
scripts/issue-claim.sh --base feature/x 42  # cut from a feature branch
scripts/issue-claim.sh --resume 42          # back to a branch whose PR went red
scripts/issue-claim.sh --fresh | --cold | --reuse
```

A claimable issue is open, `ready`, unassigned and blocked by nothing open.
`epic`, `icebox`, `needs-architecture` and `needs-maintainer` are never taken.
The lock is a `mkdir` under `~/.cache/postio/claims`; the assignee and
`in-progress` label are for people reading the board.

Run the claim from inside the worktree you just landed: it moves that tree to
the new issue and keeps the build warm. If that would strand something (a
dirty tree, unlanded commits) it says so and seeds a fresh tree instead.

If it prints *no ready issues*, stop and say so. Do not hunt the backlog.

**Small issues can share a branch.** Claim each extra one by hand —
`gh issue edit <n> --add-assignee @me --add-label in-progress` and
`mkdir ~/.cache/postio/claims/issue-<n>` — give each its own commits with its
own `Refs:`, land once, then `gh issue close <n> -c "Landed with #<anchor>"`
and `rmdir` its lock.

**Interdependent issues get a feature branch** when landing them one at a
time would leave `main` half-migrated. Claim each with `--base feature/<x>`;
the base is recorded in the worktree and the landing follows it. Resolve
conflicts in append-only registries (`CommandId`, `focus_suite`'s `CASES`,
generated docs) one item at a time, never by concatenating hunks, and run
`cargo nextest run -p postio-core -p postio-config` before continuing.
Rebase the feature branch onto `main` as you go. Children do not close on a
merge into the feature branch; one PR to `main` naming every child closes
them.

## 2. Work

Stay in the worktree. Everything is allowed in it — `git add -A`, `cargo fmt
--all`, editing any crate the issue needs.

- **Test first.** Write the test, see it red, make it green. Never re-break
  code to prove a test.
- **Iterate cheaply.** `scripts/test-fast.sh` between edits; the integration
  suite your change touches (`cargo nextest run -p <crate> --test <suite>`,
  name the case) to confirm.
- **Wait for conditions, not durations.** Use `settle_until`; a slow machine
  raises `POSTIO_TEST_PATIENCE`. A new test file must be named by a `mod`
  line in its suite's `main.rs`.
- **Keyboard behaviour changes start with a storyboard** in `storyboards/`,
  red on the base (`scripts/storyboards.sh base`). Run `/ux-review` before
  landing.
- **Fetch before you reason about the tree.** `main` moves under you.
- **Do not edit while a build or test run is in flight**; its result would
  be for a tree that never existed.

## 3. Land

```bash
scripts/issue-land.sh --detach                # the ordinary landing
scripts/issue-land.sh --detach --full-suite   # a new feature or big change: CI runs the nightly on the PR
scripts/issue-land.sh --detach --refs-only    # this PR does not finish the issue
scripts/issue-land.sh --detach --wip          # push a branch, no PR yet
scripts/issue-land.sh --detach --no-merge     # PR for a human to look at first; say why
```

It formats, runs clippy and the unit tests of what you changed plus
`scripts/check.sh`, commits, rebases on the base, pushes, opens a PR that
closes the issue, and arms auto-merge. PRs land **squashed**. `--full` (the
integration suites locally) needs a specific reason; run the suites your diff
touches yourself instead.

**Opening a PR by hand** -- from a host that cannot build a crate the branch
touches, so `issue-land.sh` refuses -- skips its gates, so run the ones this
host can: `scripts/check.sh`, the suites you touched, and
`scripts/doc-check.sh` (rustdoc under CI's flags). Then
`gh pr merge <n> --auto --squash`.

A red PR is still yours: the next claim and `/steward` both report it, and
`scripts/issue-claim.sh --resume <n>` takes you back to fix it on the same
PR. A gate failure in code you did not touch is probably someone else's —
reproduce it alone and search the issues before re-running.

## 4. Is it reachable?

If the issue built a surface — a widget, a command, a view — check a person
can reach it in the running app. Every layer passing its tests is not the
same as the layers being joined. Either wire it or file the wiring issue
before you close, and say which in the PR.

## 5. Finish and keep going

When the landing returns, claim the next issue from the same worktree. Do not
ask whether to continue. Stop only when nothing is ready, a decision is the
maintainer's, or context is nearly gone — and land or commit first.

```bash
scripts/issue-release.sh <n>              # only when you stop
scripts/issue-release.sh <n> --abandon    # hand it back to the pool
```

## When something is in the way

- **A bug blocks you:** fix it on your branch and say so in the PR.
- **The issue is wrong or bigger than it says:** comment with what you found,
  file the rest with `scripts/issue-file.sh`, keep going on your part.
- **It needs a design or architecture call an agent can make:**
  `needs-architecture` plus a comment with the question and options, release
  with `--abandon`, take something else.
- **Only the maintainer can decide it:** the same, with `needs-maintainer`.
- **A lint fires only in CI:** a `RUSTUP_TOOLCHAIN` in your shell is
  overriding the pin; `rustc --version` says which compiler you have.
