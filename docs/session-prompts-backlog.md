# Backlog-burn prompt

Paste the block below to an agent. It assumes a checkout of this repository
and nothing else; everything it needs to know that is not in `CLAUDE.md` is
stated inline.

Written 2026-09-02 against 93 open issues, 38 of them `ready`.

---

You are working the Postio backlog. Read `CLAUDE.md` first — it is the
contract, and this prompt only adds what it does not say.

**Your goal is issues closed, not commits written.** Landing has a fixed
cost, so the difference between a fast session and a slow one is almost
entirely how much you land per gate run.

## The loop

```bash
scripts/issue-claim.sh --label opus        # or --label sonnet, whichever you are
cd ~/src/postio-worktrees/issue-<n>
# work
scripts/issue-land.sh                      # backgrounded — see below
scripts/issue-release.sh <n>
```

Then **claim the next one without asking**. Finishing an issue is not
finishing a session. Stop only when the claim script says nothing is ready,
when a decision is genuinely the maintainer's, or when your context is nearly
gone — and land or commit what you have first in every case.

macOS work is already excluded: those issues carry `ready-mac`, never plain
`ready`, so `issue-claim.sh` will not offer them. You do not need to filter
for it.

## Batch, because landing is the expensive part

The single biggest throughput lever. A gate chain plus a CI run costs the
same whether it carries one issue or four.

```bash
scripts/issue-claim.sh 813                 # the anchor: names the branch
gh issue edit 585 --add-assignee @me --add-label in-progress
mkdir ~/.cache/postio/claims/issue-585     # the lock — NOT mkdir -p
```

Work each as its own commit with its own `Refs: #<n>`. Land once. The PR
closes the anchor; close the riders yourself with
`gh issue close <n> -c "Landed with #<anchor> in <url>"` and `rmdir` their
locks.

Use plain `mkdir` for the lock. `mkdir -p` succeeds on a directory that
already exists, which means it will silently steal another session's claim.

Batch small and *compatible*. Two issues in the same crate share a build and
read as one reviewable PR; a script fix and a GTK fix do not, and a batch
that stops being reviewable has cost you the review rather than saved you a
gate run.

**Do not batch #478–#484.** That is the rules cluster: seven interdependent
issues that would leave `main` half-migrated if they landed one at a time.
It is an *initiative* and wants a feature branch —
`scripts/issue-claim.sh --base feature/rules <n>` — not a batch. Same for
any set where landing one alone would leave the tree in a state nobody
wants.

## Test tiers: know which one you need

| | what it runs | cost |
|---|---|---|
| `scripts/test-fast.sh` | changed crates, `--lib` | seconds |
| `scripts/test-sanity.sh` | whole workspace, `--lib` — 1,313 tests | ~5s warm |
| `scripts/issue-land.sh` | sanity tier + clippy + invariants | the default |
| `scripts/issue-land.sh --full` | the above plus per-crate integration suites | minutes |

Default to the sanity tier. Reach for `--full` when your change is about
**wiring or test structure** rather than logic — moving test files, touching
a harness, changing what the composition root builds. CI runs the whole
workspace on every PR regardless, so `--full` buys you an earlier answer, not
a safer merge.

The first run in a fresh worktree pays a cold dependency build whatever tier
you pick. That is not the tier failing.

## Run the land in the background

```bash
setsid nohup sh -c 'scripts/issue-land.sh > /tmp/land.log 2>&1; echo "EXIT=$?" > /tmp/land.done' >/dev/null 2>&1 </dev/null &
```

A full chain can outlive a foreground tool call, and a run killed mid-gates
commits nothing. Launch it, do something else, act on the result. **Do not
poll it in a loop** — wait for the completion signal your harness gives you.
A run that was killed is cheap to retry: green gates are recorded against the
exact tree, so an unchanged retry skips to the landing.

## Things that cost real time on 2026-09-02

Each of these is a day's worth of tuition. Do not re-learn them.

- **Never truncate test output.** `cargo test ... | head -8` showed eight
  green lines and hid a failing ninth suite. Write the log to a file and
  grep it for `FAILED`, or count the failures. This happened twice, in two
  disguises.
- **`cargo build` is not the gate.** Unused imports are warnings to the
  compiler and errors to `clippy -D warnings`. Removing a helper usually
  orphans an import; the gate will find it if you do not.
- **Doctests are not in the sanity tier.** An indented block in a `///`
  comment is a Rust code block to rustdoc, and it will try to compile your
  prose. Fence quoted output as ` ```text `.
- **A green run does not prove a fix for an intermittent bug.** Baseline the
  unfixed build the same way before you claim anything. Prefer testing the
  deterministic mechanism underneath the flake — a leak, a missing
  disconnect — over reproducing the crash.
- **Measure before optimising.** Two confident diagnoses were wrong this way.
  The suite spends 108s executing tests inside a ~497s step; the cost is
  linking, not running.
- **A red check on your PR may not be yours.** Read it before assuming: one
  was a cancelled runner, one was a pre-existing flake in a crate the branch
  never touched. Fix it if it is yours, record evidence on the issue if it is
  not, and say which.
- **Fetch before reasoning about the tree.** `git fetch origin main` first.
  Stale comparisons have produced confident wrong reports here.

## Recording what you find

`GitHub is where this project talks to itself.` Terminal output is read once
and gone.

- Work you discovered: `scripts/issue-file.sh` — it **searches first** and
  stops if something already exists. When it finds one, comment on that
  instead. A second occurrence on an existing issue is worth more than a
  duplicate, because it is evidence the bug survived a fix.
- A constraint future sessions must respect: `docs/engineering-notes.md`.
- Only the maintainer can decide it: label `needs-maintainer` and comment
  with the specific question and the options. Do not stop silently.
- A design call an agent could make: label `needs-architecture`.

If a PR auto-closes an issue whose acceptance criteria are not met, reopen it
with what actually landed and what remains. A closed issue claiming finished
work is worse than an open one.

## Do not take

`epic`, `icebox`, `needs-architecture`, `needs-maintainer`, anything already
`in-progress`, and anything not labelled `ready`. If `issue-claim.sh` reports
nothing ready, say so and stop — do not trawl the backlog for unlabelled
work, and do not drop `--label` to find more.

Every claimable `ready` issue carries `opus` or `sonnet` as of 2026-09-02, so
an empty pool under your flag means your queue really is empty, not that the
labelling is lagging. If you ever find a `ready` issue with neither label,
that is a gap worth naming on the issue rather than working around.
