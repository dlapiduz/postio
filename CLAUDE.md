# Project instructions for AI agents

Claim an issue, work it test-first in your own worktree, land it, take the
next. Hooks and `scripts/check.sh` enforce most rules and print the remedy;
this file holds what they cannot check. Traps by area: `docs/gotchas.md`.

## The loop

```bash
scripts/issue-claim.sh                  # next ready issue -> its own worktree
cd ~/src/postio-worktrees/issue-<n>     # work there, never in ~/src/postio
scripts/issue-land.sh --detach          # gates, commit, push, PR, auto-merge
scripts/issue-land.sh --status          # what the landing did
```

`/issue` has claim flags, batches, feature branches and releases.

- **Keep going.** Stop only when nothing is ready, when a decision is the
  maintainer's (label `needs-maintainer`, comment the question and options),
  or context is nearly gone. Commit as you go.
- **Spec-driven work** (`specs/<nnn>-<name>/`, after `/speckit-specify` and
  `/speckit-plan`) runs on `feature/<name>`: one commit per task ending
  `Refs: specs/<nnn>-<name>` and the task id, landed with
  `issue-land.sh --detach --title '…'`. No issue per task.
- A change too small for an issue: branch `fix/`, `docs/` or `chore/<slug>`.

## Tests

Red, then green. A test never seen red gets a tighter assertion, never a
re-broken fix. An issue is done when its acceptance is covered by tests.

| Tier | Command | When |
|---|---|---|
| fast | `scripts/test-fast.sh` | between edits |
| suite | `cargo nextest run -p <crate> --test <suite> [case]` | what you touched |
| main | full suite and frontends | after each merge, unattended |

- **Each layer asserts its own question (ADR 0049).** A behaviour's branches
  are tested once, in a rule crate or the controller (`postio-focus`). Each
  frontend has one case per surface a person reaches, plus a key-binding
  table check. Put logic where it is cheap to test: a function in
  `postio-core`, `postio-ui` or `postio-body`.
- **Assert what a person sees**, not what a layer was handed.
- **No network, no clock.** Fake clocks (`start_paused = true`); a slow case
  goes to the nightly tier with `POSTIO-MEASUREMENT:`. `#[ignore]` only means
  "this machine may lack what the test needs".
- **Keyboard behaviour changes** ship with a storyboard (`storyboards/`),
  red on the base, and `/ux-review` before landing.
- Display tests live in `tests/`; protocol code tests against the
  `MailBackend` mock and the `.eml` corpus (`/add-fixture`).
- Logs: `POSTIO_LOG` (an `EnvFilter`), not `RUST_LOG`.

**Landing costs at most five minutes.** `issue-land.sh` runs the cheap gates,
pushes, opens the PR and arms auto-merge. Report the PR number and stop or
claim the next issue; never wait on CI. A red PR check is yours
(`issue-claim.sh --resume <n>`); a failure outside your diff is probably
someone else's. While `main` is red only a `fixes-nightly` PR merges.

## What the code must do

The constitution (`.specify/memory/constitution.md`) holds the principles;
these are the ones every diff meets:

- **Instant:** startup < 500 ms, interaction < 16 ms, local search < 100 ms,
  transitions ≤ 100 ms or none. Never load a whole mailbox. Budgets are
  gated as counts: add a count assertion when you touch a read path.
- **Local-first:** store write, enqueue, emit, repaint. The UI never awaits
  the network.
- **Providers are data** (the preset table). **No backwards compatibility**:
  write the clean version and its migration. **Pimalaya first** for protocol
  code. **Crate boundaries** are in `docs/ARCHITECTURE.md`.
- **Privacy:** nothing leaves the machine unasked; credentials only in the
  keyring; logs carry ids, counts and outcomes, never content. The repo is
  public: reserved domains (`ada@example.com`) and no real names.

## Comments

1. A comment states a why the code cannot show: a constraint, ordering,
   race, safety or privacy rule, workaround, or the bug it prevents. At most
   three lines, on the line it protects.
2. Never restate a name, a signature or the code below. Rename instead.
3. Private items, fields and variants get no doc. A public item gets one
   only for a caller's contract (units, errors, panics, ordering, `None`).
4. No history, narrative or FR/task ids; that is the commit body's job. A
   reference is a bare id after a why: `(ADR 0049)`, `(#751)`.
5. A module `//!` is at most three lines; test files get none. `POSTIO-*`
   markers stay. No dividers, no commented-out code.
6. If the why can be a type, a test name or an assert message, make it that.

## Commits and git

- The body says **why**. Format and trailers are `.gitmessage`; the hook
  names any rule you break. PRs merge squashed; the PR title is the subject.
- Fetch before you reason about the tree; other sessions land all the time.
- Never force-push (a hook refuses it). A conflicted PR runs no checks: merge
  `origin/main` into it and push. Ask before pushing `main`, adding remotes
  or rewriting shared history.

## One machine, many sessions

Work only in your own worktree. Never pass `-j` (a machine-wide jobserver
hands out jobs). Seed a tree made by hand with `scripts/worktree-seed.sh`.
Never put a worktree path where rustc sees it. `scripts/target-sweep.py`
trims a target that grew across rebases. The Mac's loops: `macos/CLAUDE.md`.

## Say it where it persists

| What | Where |
|---|---|
| Why the change is shaped this way | the commit body |
| What you found along the way | a comment on the issue |
| Related work | fix it now, in its own commit |
| Tangential work | `scripts/issue-file.sh` |
| A design call an agent can make | label `needs-architecture` |
| A call only the maintainer can make | label `needs-maintainer` |
| A trap future sessions must know | `docs/gotchas/<area>.md` |
| A rule other work must obey | an ADR in `docs/decisions/` |

## Sources of truth

Skills: `/issue`, `/steward`, `/lanes`, `/preflight`, `/add-fixture`,
`/ux-architect`, `/gtk-design`, `/ux-review`, `/speckit-*`. Product:
`docs/PRODUCT.md` (the app says "Flagged", never "Starred"). Keys:
`docs/keybindings.md` (generated). Decisions: `docs/decisions/`, then
`specs/`. Visual truth: spec 007's `screens.md`. v1 scope: Linux, IMAP and
SMTP with OAuth, one preset table, no AI inside Postio. A release is a PR:
`scripts/release-prepare.sh X.Y.Z`.
