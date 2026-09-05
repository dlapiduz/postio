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
scripts/issue-claim.sh --base feature/x 42  # cut from an initiative branch
scripts/issue-claim.sh --fresh              # a new worktree even from inside one
scripts/issue-claim.sh --cold               # ...and do not seed its target/
scripts/issue-claim.sh --reuse              # strict: refuse rather than fall back
```

**Run it from inside the worktree you just landed.** A plain claim there
moves that tree to the new issue's name and checks out its branch, so the
build stays warm — and when reuse would strand something (a dirty tree,
commits not on the base) it says why and claims a fresh tree instead,
leaving this one exactly as it was (#1102). A fresh tree is not cold either:
its `target/debug` is reflink-copied from the newest sibling, one second for
11 GB. In both cases Postio's own crates are dropped and rebuilt — about a
minute — because they carry the tree's absolute path and cargo does not
notice a move; the dependencies stay. Neither is the sharing #76 forbids:
that is two worktrees writing one target.

`--fresh` when you want the old tree kept on purpose; `--reuse` when you
want to be told rather than helped.

## Several small issues on one branch

Landing has a fixed cost — the gate chain, the rebase, the PR — so three
ten-minute issues as three landings is mostly overhead. When the issues are
small and touch compatible ground, batch them:

1. Claim the first one normally; it names the branch and the worktree.
2. Claim each rider with `gh issue edit <n> --add-assignee @me --add-label
   in-progress` and `mkdir ~/.cache/postio/claims/issue-<n>` — the lock
   without a second worktree.
3. Work each as its own commit (or commits), `Refs:` its own issue.
4. Land once. The PR's `Closes #<anchor>` handles the first; close each
   rider yourself: `gh issue close <n> -c "Landed with #<anchor> in <pr-url>"`,
   and `rmdir` its claim lock.

Batch small and compatible, not merely convenient: an issue that grows past
"small" mid-work deserves its own branch, and a batch should still read as
one reviewable PR. When in doubt, land what is done and start a fresh batch.

## Landing somewhere other than `main`

Most work is one issue onto `main` and nothing below applies. An
**initiative** — several interdependent issues that would leave `main`
half-migrated if they landed one at a time — gets a branch of its own, and
`--base` is how a worktree is cut from it.

**You pass `--base` once, to the claim.** It is recorded in the worktree, and
`issue-land.sh` reads it back: the rebase, the changed-crate list and the PR's
base all follow it, with no flag repeated. That is deliberate — a flag you
have to remember on every landing is one you eventually forget, and forgetting
*this* one merges initiative work straight into `main`, which is the single
thing the initiative branch exists to prevent.

Two refusals to expect, both of which mean stop rather than retry:

- **A base that is not on `origin`.** Refused by the claim, before anything is
  taken, so a typo cannot leave a claim behind that locks the issue for every
  other session.
- **An open PR whose base disagrees with the recorded one.** `issue-land.sh`
  will not merge on that mismatch: one of the two is wrong and guessing which
  lands the work somewhere nobody chose. Retarget the PR, or correct the
  record.

`main` is still the default, so `scripts/issue-claim.sh` with no `--base`
behaves exactly as it always has.

**Rebase the initiative branch onto `main` regularly.** `origin/arch/adr-0005-revision`
is what happens otherwise: 359 commits ahead, 486 behind, effectively dead.
The tooling will not do it for you.

It takes an issue that is **open, labelled `ready`, unassigned, and not blocked
by anything still open** — `blockedBy` is a real GitHub field, so this is not a
convention that can drift. It never takes `epic` (a container), `icebox`
(deferred), or `needs-architecture` (a human has to decide something first).

**Know your own model before you claim.** The product-manager loop labels
issues `opus` or `sonnet` — its read on which model the work actually needs
(a concurrency race or a security-sensitive decision versus a mechanical fix
with the diff already in the issue body). You already know which model you
are from your own context; pass it so you only ever take work sized for you:

```bash
scripts/issue-claim.sh --label opus         # this session is Opus
scripts/issue-claim.sh --label sonnet       # this session is Sonnet
```

`--label` is an exact match, so an issue the loop hasn't labelled yet (new,
or an epic/container that's never claimable anyway) will not show up under
either flag — that is a labelling gap for the product-manager loop to close,
not something to work around by dropping the flag. If nothing matches, that
is the same "stop and say so" case as no ready issues at all: do not fall
back to claiming unlabelled work just to stay busy.

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

Cargo builds into this worktree's own `target/`. A reused tree is already
warm; a fresh one was seeded from a sibling's (#1102), and whatever is still
missing comes from the machine-wide sccache, wired in by `.cargo/config.toml`.
Copying a target is fine; *sharing* one compiles a worktree's crate against
another's (#178) — never point two trees at one.

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

**Do not edit source while a build or test run is in flight.** A backgrounded
`cargo test` is a tempting ten minutes to fill, and editing a file it has not
compiled yet either wastes the run or — worse — produces a result for a tree
that never existed, which you then reason about as if it were real. Cargo will
not warn you. Wait for it, or work on something outside the workspace: the
issue text, a commit message, a comment you owe somebody.

Two cargo invocations in **one worktree** serialise on that worktree's target
directory rather than running side by side. If a build seems to be taking far
longer than it should, check whether you left an earlier one running:

```bash
cat /proc/loadavg                                   # idle box, busy build?
ps -eo pid,etime,args | grep -E "cargo|rustc" | grep -v grep
```

An idle load average with two cargo processes minutes old is not a slow build,
it is one waiting on the other's lock. Kill the stale one. Compile jobs come
from one machine-wide pool (`scripts/jobserver.sh`, #1104): alone you get the
whole box, so `-j` is never the lever.

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

**Iterate cheaply; confirm expensively.** `scripts/test-fast.sh` runs the unit
tests of the crates you changed and links nothing else — seconds. One
`cargo nextest run -p postio-app --test app_suite` is ~20 s of running on a
warm tree (200 s under plain `cargo test`, which runs its binaries one at a
time), and TDD pays it twice per fix. Use the fast loop between edits and the
integration suites to confirm at the end. If a rule is hard to prove without
linking the application, that is usually a sign it wants to be a function in a
leaf crate rather than something buried in a widget.

Still true, and not negotiable: **TDD** — write the failing test, watch it
fail, make it pass, move on. The red run at the start is the proof the test
can fail; never re-break working code after going green to test the test.
Also non-negotiable:
**no network in the default suite**; **no personal data in fixtures**; the
[architectural invariants](../../../CLAUDE.md); and the perf and motion budgets.

## 3. Land

```bash
scripts/issue-land.sh --detach                      # the ordinary landing
scripts/issue-land.sh --status                      # what it did, or is doing
scripts/issue-land.sh --detach -m "feat(gtk): ..."  # with uncommitted work
scripts/issue-land.sh --detach --gates-only         # check without committing
scripts/issue-land.sh --detach --wip                # push a branch, no PR yet
scripts/issue-land.sh --detach --refs-only          # Refs, not Closes: this PR does not finish the issue
```

`--detach` runs the landing in a process no tool call can kill and returns
at once; the hook refuses a foreground run. Read the log with `--status`.

It formats, runs clippy **for the crates you actually changed** and the
**sanity tier** — the whole workspace's unit tests, 1,313 of them in about
five seconds — runs every repository invariant via `scripts/check.sh`,
commits, pushes the branch, opens a PR whose body says `Closes #<n>`, and
**arms GitHub's auto-merge**: the ruleset requires CI's checks, so it merges
the moment they are green and nobody waits (#1107). `--wait` watches and
merges the old way, for a landing you want to see through.

`--full` adds the per-crate integration suites, which is what this used to
always do. **Land on the default; `--full` needs a specific reason, and "my
change is about wiring" is not one** — that was this file's own advice until
#901 spent three ~25-minute runs on it and failed all three on other
people's bugs. Run the suites your diff touches directly instead
(`cargo nextest run -p <crate> --test <suite>`; doctests need `cargo test
-p <crate> --doc`) and let CI run the rest, which it does on every pull
request. `docs/engineering-notes.md` has the worked example.

The default is fast because several sessions share this machine and the
per-crate suites are minutes each — not because integration tests stopped
mattering. They are how this project
catches the bug it actually ships: layers that each pass and are not joined
up. Which is an argument for *writing* them, and for running the ones your
change is about; it is not an argument for running all of them twice.

A PR nobody merges is work that looks finished and is not, so a red one is
still yours:

- **Checks pass** → GitHub merges and deletes the branch. You have already
  claimed the next issue from inside this worktree.
- **Checks fail** → your next claim says so (`note: PR #N has failing
  checks`), and so does `/steward`. `scripts/issue-claim.sh --resume <n>`
  cuts a worktree from the branch, tracking it; fix, then land again onto
  the same PR. Do not open a second PR.
- `--no-merge` opens the PR and stops, for a change that genuinely needs a
  human to look first. Say in the PR why.

Merging is a **rebase**, not a squash: this history is linear and the commit
convention asks for small focused commits, so squashing a branch discards the
structure those rules exist to produce.

If the rebase onto `main` brings in a change under `scripts/`, the run says
`handing over to the landing machinery this rebase brought in` and starts
again from the top on the new copy — gates included. That is not a fault:
this script rebases the tree it lives in, so without the handover the run
that pulls a landing fix in is the one run that fix cannot protect. See #160.

GitHub's `--auto` merge waits only for *required* checks, which is why the
ruleset requires every `ci.yml` job: a job that skips itself for a docs-only
diff still reports, and counts as passed. On a repository without auto-merge
allowed the script watches and merges itself, as it always did.

The commit message rules are unchanged — conventional subject, a body that
explains **why**, wrapped at 72 columns. The footer becomes `Refs: #<n>`
and the script adds it.

**Push is now standing-authorised for issue branches only.** Pushing a branch
that exists to be reviewed cannot damage anything. Pushing `main` still
requires the user to ask, and remote changes and history rewrites are still
refused outright.

`--force-with-lease` is authorised on your own issue branch; bare `--force`
is not. This script rebases onto `origin/main` before pushing, so the second
push of a branch you have already pushed is necessarily non-fast-forward —
there is no non-forcing spelling of it. The leased form refuses if the remote
has moved; `--force` cannot tell your own rebase from a commit somebody else
landed. The guard hook enforces that split in every tree, private worktree
included, because the remote is shared even when the checkout is not.

## Tests that survive a loaded runner

**Wait for conditions, never for durations or turn counts.** Every
intermittent CI failure this project has had was a sleep or a fixed pump that
was long enough on an idle workstation and not on a loaded runner. Use the
shared `settle_until`, which says what it was waiting for when it times out.
A genuinely slow machine raises `POSTIO_TEST_PATIENCE` — that is the dial,
so nobody edits a constant and slows the suite for everyone.

**Tests live in suites, one binary per crate.** A new test file belongs in
the suite directory and must be named by a `mod` line in its `main.rs`: an
undeclared file is not an error, it is silence, and `check-suite-modules.py`
exists because silence is indistinguishable from passing. A test that
acquires the default GLib main context belongs in a `harness = false` suite,
which `check-parallel-main-context.py` enforces.

**A closed issue claims finished work.** If a PR auto-closes an issue whose
acceptance is not met, reopen it and say what landed and what remains.

**Say it before the PR merges, not after.** `scripts/issue-land.sh
--refs-only` writes `Refs: #<n>` instead of `Closes #<n>`, with the body
itself saying the omission was deliberate — for a measurement that changed
the question, a fix for one of several acceptance criteria, or any other PR
that does not finish its issue. With auto-merge (#1107) the PR merges itself
minutes after the script returns, so the reopen-and-explain path above is
the worse half of a trade you can avoid: the issue never goes to `CLOSED` in
the first place, and the session that knows the work is partial is the one
that says so, in the PR a reviewer actually reads.

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

When the PR merges, **claim the next issue from right here, without asking**:

```bash
scripts/issue-claim.sh                    # reuses this worktree, build and all
```

It moves the tree to the new issue's name; if that would strand something it
says why and claims a fresh, seeded tree instead. The worktree is released
only when you stop:

```bash
scripts/issue-release.sh <n>              # remove the worktree, release the claim
scripts/issue-release.sh <n> --abandon    # gave up: hand it back to the pool
scripts/issue-release.sh --stale          # sweep claims whose worktree is gone
```

It refuses to remove a worktree with uncommitted changes. A pushed branch is
recoverable; a deleted worktree is not.
 Finishing an issue is not
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
- **It needs a design or architecture call an agent can make.** Add
  `needs-architecture` — that is `/ux-architect`'s queue — comment with the
  question and the options, release with `--abandon`, and take something else.
- **It needs a decision only the maintainer can make** — scope, product
  direction, a trade-off with no defensible default. `needs-maintainer`, the
  same comment, the same release.
- **CI fails on your PR.** That is yours to fix, on the same branch. The
  compiler is pinned by `rust-toolchain.toml` on both sides, so a lint that
  fires only there means a `RUSTUP_TOOLCHAIN` in your shell is overriding the
  pin; `rustc --version` says which you have.
