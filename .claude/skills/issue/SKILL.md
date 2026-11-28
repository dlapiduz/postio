---
name: issue
description: Take a GitHub issue and work it end to end in a private worktree: claim, branch, build, verify, commit, push, PR that closes it. All work here starts with it. Run it when you need work, and again when a PR is open.
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

- Claimable: open, `ready`, unassigned, blocked by nothing open. `epic`, `icebox`, `needs-architecture` and `needs-maintainer` are never taken.
- The lock is a `mkdir` under `~/.cache/postio/claims`; assignee and `in-progress` are for people.
- Claim from inside the worktree you just landed: it moves that tree to the new issue and keeps the build warm. If that would strand a dirty tree or unlanded commits, it says so and seeds a fresh tree.
- *No ready issues*: stop and say so. Do not hunt the backlog.

**Small issues can share a branch.** Claim each extra by hand: `gh issue edit <n> --add-assignee @me --add-label in-progress` and `mkdir ~/.cache/postio/claims/issue-<n>`. Give each its own commits and `Refs:`, land once with `--closes <n>,<m>` so the PR closes them all, then `rmdir` their locks.

**Interdependent issues get a feature branch** when one-at-a-time landing would leave `main` half-migrated.
- Claim each with `--base feature/<x>`; the base is recorded in the worktree and the landing follows it.
- Resolve conflicts in append-only registries (`CommandId`, `focus_suite`'s `CASES`, generated docs) one item at a time, never by concatenating hunks; run `cargo nextest run -p postio-core -p postio-config` before continuing.
- Rebase the feature branch onto `main` as you go.
- Children do not close on a merge into the feature branch; one PR to `main` naming every child closes them.

**A branch that needs an open PR's branch** is stacked: claim with `--base <that-branch>` (or write the name to `postio-base`) and land as usual; the PR opens against that branch. After it squash-merges, the next `issue-land.sh` rebases `--onto origin/main` from the base's old tip (a plain rebase would replay commits `main` has under other hashes), records `main` and retargets the PR. While the base is open, it says the PR retargets when the base merges.

## 2. Work

Stay in the worktree; everything is allowed in it (`git add -A`, `cargo fmt --all`, any crate the issue needs).

- **Test first.** Red, then green. Never re-break code to prove a test.
- **Iterate cheaply.** `scripts/test-fast.sh` between edits; `cargo nextest run -p <crate> --test <suite>` (name the case) to confirm.
- **Wait for conditions, not durations.** Use `settle_until`; a slow machine raises `POSTIO_TEST_PATIENCE`. Name a new test file in a `mod` line in its suite's `main.rs`.
- **Keyboard behaviour changes start with a storyboard** in `storyboards/`, red on the base (`scripts/storyboards.sh base`); run `/ux-review` before landing.
- **Fetch before you reason about the tree**; `main` moves.
- **Do not edit while a build or test run is in flight**; its result would be for a tree that never existed.

## 3. Land

```bash
scripts/issue-land.sh --detach                # the ordinary landing
scripts/issue-land.sh --detach --closes 12,34 # a batch: the PR closes these too
scripts/issue-land.sh --detach --refs-only    # this PR does not finish the issue
scripts/issue-land.sh --detach --wip          # push a branch, no PR yet
scripts/issue-land.sh --detach --no-merge     # PR for a human first; say why
```

**A landing costs you at most five minutes.** It formats, runs clippy, your crates' units and `scripts/check.sh` (a 300 s budget), commits, rebases, pushes, opens a PR closing the issue and arms auto-merge. PRs land **squashed**.
- Then report the PR number; never wait on CI. PR checks run unattended; the full suite and the frontends run on `main` after each merge.
- While `main` is red only a `fixes-nightly` PR merges.
- A watchdog stops a landing stuck past 20 minutes and names the phase (`--status`).

**Opening a PR by hand** (from a host that cannot build a crate the branch touches) skips the gates, so run what the host can: `scripts/check.sh`, the suites you touched, `scripts/doc-check.sh` (rustdoc under CI's flags). Then `gh pr merge <n> --auto --squash`.

**A red PR is yours**: the next claim and `/steward` both report it; `scripts/issue-claim.sh --resume <n>` brings you back. A failure in code you did not touch is probably someone else's: reproduce it alone and search the issues before re-running. For a suspected flake, `scripts/flake-check.sh -p <crate> --test <suite> <test>` runs it idle and under load.

## 4. Is it reachable?

If the issue built a surface (widget, command, view), check a person can reach it in the running app; passing layers are not joined layers. Wire it, or file the wiring issue before you close, and say which in the PR.

## 5. Finish and keep going

Once the PR is open, claim the next issue from the same worktree; do not ask whether to continue. Stop only when nothing is ready, a decision is the maintainer's, or context is nearly gone, and land or commit first.

```bash
scripts/issue-release.sh <n>              # only when you stop
scripts/issue-release.sh <n> --abandon    # hand it back to the pool
```

## When something is in the way

- **A bug blocks you:** fix it on your branch; say so in the PR.
- **The issue is wrong or bigger than it says:** comment with what you found and fix the rest on your branch. `scripts/issue-file.sh` only for what is tangential or cannot be done now.
- **A design call an agent can make:** `needs-architecture` plus a comment with the question and options, `--abandon`, take something else.
- **Only the maintainer can decide:** the same, with `needs-maintainer`.
- **A lint fires only in CI:** a `RUSTUP_TOOLCHAIN` in your shell overrides the pin; `rustc --version` says which compiler you have.
