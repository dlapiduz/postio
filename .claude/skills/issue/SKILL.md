---
name: issue
description: Take a GitHub issue and work it end to end in a private worktree — claim it, branch, build, verify, commit, push, open a PR that closes it. This is how all work in this repository starts. Run it whenever you need work, and again the moment a PR is open.
---

# Work a GitHub issue

The whole loop is three commands. Everything else in this file explains why
they are shaped that way.

```bash
scripts/issue-claim.sh                 # take the next ready issue, get a worktree
cd ~/src/postio-worktrees/issue-<n>    # work there from now on
scripts/issue-land.sh -m "feat(gtk): ..."
```

## 1. Claim

```bash
scripts/issue-claim.sh                      # next ready issue
scripts/issue-claim.sh --milestone MVP      # scoped to a milestone
scripts/issue-claim.sh 42                   # a specific issue
scripts/issue-claim.sh --dry-run            # look before taking
```

It takes an issue that is **open, labelled `ready`, unassigned, and not blocked
by anything still open** — `blockedBy` is a real GitHub field, so this is not a
convention that can drift. It never takes `epic` (a container), `icebox`
(deferred), or `needs-architecture` (a human has to decide something first).

Claiming is atomic: a `mkdir` under `~/.cache/postio/claims`, which either
succeeds or fails with no window in between. **Assignee cannot be the lock** —
every session authenticates as the same GitHub user, so `--add-assignee @me`
tells two sessions apart not at all. Assignee and the `in-progress` label are
set anyway, for humans looking at the board.

If it prints *no ready issues*, **stop and say so.** Do not go hunting for
work in the backlog; an unlabelled issue has not been triaged as agent-ready.

## 2. Work

`cd` into the worktree and stay there. It is a real checkout on its own branch,
cut from `origin/main`.

Point cargo at the shared target directory so the 400-odd third-party crates
(GTK, WebKit — the expensive ones) stay warm instead of rebuilding:

```bash
export CARGO_TARGET_DIR=~/src/postio/target
```

**The parallel-work hazard table in CLAUDE.md does not apply in here.** That
table exists because sessions shared one tree and one index. This tree is
yours. So all of these are now correct rather than dangerous:

| In the shared checkout | In your worktree |
|---|---|
| `git add -A` destroys others' work | fine — nothing else writes here |
| `git commit --only <paths>` was mandatory | plain `git commit` is fine |
| `git stash` stashed everyone's changes | fine |
| `cargo fmt --all` churned others' diffs | fine |
| stay inside your own crate | **touch whatever the issue needs** |

That last row is the real gain. Work is isolated by *branch* now, not by crate,
so an issue that spans `postio-storage` and `postio-gtk` is one piece of work
rather than something you have to hand off.

Tests are headless by default — `.cargo/config.toml` puts every test binary on
a compositor of its own, so plain `cargo test` does not throw windows at
whoever is at the keyboard. `POSTIO_HEADLESS=0` if you want to watch one.

Be aware it is faster than a real session and will expose tests that race an
async load — that is the test's bug, not the harness's.

`main` moves while you work. `issue-land.sh` fetches and rebases onto it
before pushing, so you do not have to — but **fetch before you reason about
the tree**: `git log`, `git diff` and anything comparing against `origin/main`
read a snapshot that may be hours old. Rebase a long branch as you go rather
than only at the end, and re-read the issue before you finish it in case
someone decided something while you worked.

Still true, and not negotiable: **TDD** — write the failing test first;
**no network in the default suite**; **no personal data in fixtures**; the
[architectural invariants](../../../CLAUDE.md); and the perf and motion budgets.

## 3. Land

```bash
scripts/issue-land.sh -m "feat(gtk): teach the list to do the thing"
scripts/issue-land.sh --gates-only    # check without committing
scripts/issue-land.sh -m "..." --wip  # push a branch, no PR yet
```

It formats, runs clippy and tests **for the crates you actually changed**,
runs the three repository invariant checks, commits, pushes the branch, opens
a PR whose body says `Closes #<n>`, **waits for CI, and merges it**.

That last part is not optional and not someone else's job. A PR nobody merges
is work that looks finished and is not: the branch goes stale, it conflicts
with whatever lands next, and the issue it claims to close stays open. You are
the session that knows what the change was for, so you are the session that
waits for the checks and deals with them.

- **Checks pass** → it rebases onto `main` and deletes the branch. Then
  `scripts/issue-release.sh <n>` and claim the next issue.
- **Checks fail** → yours to fix, on the same branch, then run the script
  again. Do not open a second PR, and do not leave it sitting.
- `--no-merge` opens the PR and stops, for a change that genuinely needs a
  human to look first. Say in the PR why.

Merging is a **rebase**, not a squash: this history is linear and the commit
convention asks for small focused commits, so squashing a branch discards the
structure those rules exist to produce.

GitHub's own `--auto` merge is deliberately **not** used. It waits for
*required* checks, branch protection is what makes a check required, and this
repository cannot set any — so `--auto` would merge before CI had started.

The commit message rules are unchanged — conventional subject, a body that
explains **why**, wrapped at 72 columns. The footer becomes `Refs: #<n>`
and the script adds it.

**Push is now standing-authorised for issue branches only.** Pushing a branch
that exists to be reviewed cannot damage anything. Pushing `main` still
requires the user to ask, and force-push, remote changes and history rewrites
are still refused outright.

## 4. Is it actually reachable?

Before you call an issue done, if it built a **surface** — a widget, a pane, a
command, a view — answer this: can a person reach it in the running app?

This has gone wrong four times here, and the last one was the worst. Commands
resolved through the registry, the keymap, the palette and the selection model
and then hit a no-op handler. The entire search UI was built, tested, and fed
by nothing. The parts panel existed with no command to open it. And the
**Reader was never mounted** — `postio_gtk::reader::Reader` was constructed in
exactly one place in the workspace, for the search preview, while the pane the
layout gives the reader had one caller: the composer, taking it over. You
could not read mail in a mail client. Every test passed. The epic said
Reading was done.

That one was found by rendering the app and looking at it, not by a test:

```bash
cargo run -p postio-app --example shot -- /tmp/check.png demo selected
```

So: **either wire it, or open the wiring issue before you close** — and say
which in the PR. A green suite proves the widget works. It does not prove the
application does.

## 5. Finish

When the PR merges:

```bash
scripts/issue-release.sh <n>              # remove the worktree, release the claim
scripts/issue-release.sh <n> --abandon    # gave up: hand it back to the pool
scripts/issue-release.sh --stale          # sweep claims whose worktree is gone
```

It refuses to remove a worktree with uncommitted changes. A pushed branch is
recoverable; a deleted worktree is not.

Then **claim the next issue, without asking.** Finishing an issue is not
finishing a session, and "shall I pick up another?" is not a question — the
answer is written down and it is yes. A session that stops to ask has thrown
away the rest of its context waiting for a reply that may be hours away.

Stop only when `issue-claim.sh` reports nothing ready, when a decision is
genuinely the maintainer's, or when context is nearly gone. Land your work
first in every case.

## When something is in the way

- **A bug blocks your issue.** Fix it on your branch and say so in the PR
  body. You are not in anyone else's crate any more — that restriction was a
  shared-tree artefact.
- **The issue is wrong, or bigger than it says.** Comment on it with what you
  found and open a new issue for the part that does not belong. Then keep going
  on the part that does.
- **It needs a decision only the maintainer can make.** Add
  `needs-architecture`, comment with the specific question and the options,
  release with `--abandon`, and take something else.
- **CI fails on your PR.** That is yours to fix, on the same branch. Check
  `rustc --version` against CI's first: CI runs `rustup default stable`, and a
  newer compiler there produces lints that cannot fire here.
