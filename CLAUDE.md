# Project instructions for AI agents

The short version: **claim an issue, work it test-first in a worktree of your
own, land it, take the next one.** Hooks and checks enforce most rules at the
moment you'd break them; this file is the part a machine can't check. The
history and reasoning behind every rule here lives in
`docs/engineering-notes.md` and the issues it cites.

## The loop

```bash
scripts/issue-claim.sh                  # next ready issue → private worktree
cd ~/src/postio-worktrees/issue-<n>     # work there, not in ~/src/postio
scripts/issue-land.sh                   # gates, commit, push, PR, merge
scripts/issue-release.sh <n>            # remove the worktree
```

An issue is yours when it is open, labelled `ready`, unassigned, and blocked
by nothing still open. Never take `epic`, `icebox`, `needs-architecture`, or
`needs-maintainer`.
Priority is `p0`…`p4`; the claim script picks the most important thing, and
`scripts/issue-claim.sh 42` takes a specific one.

**Small issues can share one branch.** Claim the extra ones so no other
session takes them (`gh issue edit <n> --add-assignee @me --add-label
in-progress` — no second worktree), work them as separate commits, land
once; the PR closes its anchor issue, and you close the riders with
`gh issue close <n> -c "Landed with #<anchor>"`. An **initiative** — several
interdependent issues that would leave `main` half-migrated landing one at a
time — gets a feature branch instead: `scripts/issue-claim.sh --base
feature/<x> <n>` cuts the worktree from it and lands back onto it (details
in `/issue`); rebase the feature branch onto `main` regularly, merge it when
it is whole.

**Finishing an issue is not finishing a session** — claim the next one and
keep going. Never ask whether to continue; the answer is yes, and asking
costs a round trip that may not come back for hours. Stop only when:

1. The claim script says nothing is ready. Say so and stop — don't trawl the
   backlog for unlabelled work.
2. A decision is genuinely the maintainer's: scope, product direction, a
   trade-off with no defensible default. ("Which name is nicer" is yours.)
   Label it `needs-maintainer` and comment with the specific question and
   the options — don't just stop silently, and don't reach for
   `needs-architecture` instead: that one is `/ux-architect`'s queue, for a
   design or architecture call an agent can actually make. This is for the
   ones only the maintainer can.
3. Context is nearly gone.

Whichever applies, **land or commit what you have first**. Uncommitted work is
unprotected work — sessions get cut off mid-task, and this project once nearly
lost fifty files that way. Commit as you go (standing-authorised, no need to
ask); a work-in-progress commit marked as such beats loose files every time.

## TDD is red, green, repeat

Write the failing test, watch it fail, make it pass, move on. **The red run at
the start is the proof the test can fail. Never re-break working code after
going green to test the test.** If a test was never seen red — written after
the fix, say — the discipline slipped; tighten the assertion until it visibly
constrains the behavior, don't inject the bug again.

An issue is not done until its acceptance criteria are covered by tests. No
test in the default suite may touch the network — live-server tests are
`#[ignore]`. Protocol code tests against the `MailBackend` mock and the `.eml`
corpus in `crates/postio-model/tests/corpus/` (`/add-fixture` extends it).

## Build & test: verify what you touched, nothing more

```bash
cargo test   -p <crate>                              # the inner loop
cargo clippy -p <crate> --all-targets -- -D warnings # before landing
scripts/check.sh                                     # every repository invariant
```

**Test the crates you changed; the reconcile pass proves the rest.**
`issue-land.sh` runs the full gate chain (fmt, clippy, tests, `check.sh`)
over exactly your changed crates, and the steward loop periodically runs the
whole workspace against `main` — so a workspace build or test from an
ordinary session is almost always waste, and a red crate you didn't touch is
usually someone's in-flight TDD: note it on your issue and move on. Don't
re-run gates ritually either — a third of all tool calls ever made in this
repository were re-running a gate that `issue-land.sh` was going to run
anyway. `cargo fmt` is a formatter, not verification: inside your worktree
`cargo fmt --all` is fine (the land script runs it); in the shared checkout
format only files you changed, by name: `rustfmt --edition 2024 <paths>`.

- **Tests are headless automatically.** The cargo runner puts test binaries on
  a private mutter compositor; `cargo run -p postio-app` and examples reach
  the real display. `POSTIO_HEADLESS=0 cargo test` to watch a run;
  `scripts/test-headless.sh --stop` to stop the compositor. Headless is ~3.5x
  faster than a live display — a test that passes on the desktop and fails
  headless usually has a real race (see `docs/engineering-notes.md`).
- **The reconcile pass**, when you are the one doing it: `cargo test
  --workspace --no-fail-fast` — always `--no-fail-fast`, because plain cargo
  aborts remaining targets on the first failure and one red crate hides a
  thousand passing tests. `cargo bench` checks the perf budgets.
- **To see the app**: `scripts/run-isolated.sh [commit] [--inspect|--shot]`
  builds a pinned commit with its own target dir and throwaway store. It
  links `--release` — never run it while other sessions are building.
  `cargo run -p postio-app` runs whatever half-finished state is on disk.
- **To prove a change reaches the running app**, use the integration tests in
  `crates/postio-app/tests/app_suite/` (`wiring.rs` lists mail, `keystroke.rs`
  acts on it, `click_preview.rs` reads it) — the composition root is testable
  without a GUI. They are one binary behind a custom harness, so run them with
  `cargo test -p postio-app --test app_suite [name]`, and a new case is a
  module plus a row in `main.rs`'s `CASES`.
- **Assert on what a person would see, not on what a layer was handed.** Every
  layer here is tested and passes; the bugs that reach users live *between*
  them (#70 twice, `postio-bl2`). A reader test that checks the reader was
  told about a message cannot fail when nothing tells it, and a `shot` that
  draws rows it read itself cannot fail when the wiring is broken (#596).
- **Logging is `POSTIO_LOG`** (an `EnvFilter`: `debug`, or
  `postio_sync=debug`), not `RUST_LOG`. `[logging]` in `config.toml` retunes
  a running instance live.
- A test that needs a display goes in `tests/`, never in `src/` — a second
  `adw::init()` in a unit-test binary kills the whole process
  (`scripts/checks/check-no-gtk-init-in-unit-tests.py` enforces this).

System deps (Fedora 40+): see README. Rust is pinned by
`rust-toolchain.toml`; sccache is wired in via `.cargo/config.toml`, nothing
to export.

### It must feel instant

Enforced by `cargo bench`, not checked by hand: startup < 500 ms, interaction
< 16 ms, local search < 100 ms. Transitions ≤ 100 ms or absent; honor
`prefers-reduced-motion`. Never load a whole mailbox into memory — the list
is windowed over paged SQLite.

## Invariants the checks enforce

`scripts/check.sh` runs every repository invariant; each check in
`scripts/checks/` names its own fix when it fails. The architectural ones, in
one line each (the why is `docs/ARCHITECTURE.md` and the ADRs):

- `postio-core`, `postio-session`: no GTK. `postio-gtk`: no SQL, no protocol.
- `postio-search`, `postio-body`: pure leaves — no rusqlite, no gtk4.
- `postio-model`: no ammonia/html5ever, rusqlite, gtk4, or tokio — the whole
  workspace waits on it to compile.
- `postio-config`: no rusqlite, no gtk4.
- `postio-sync` talks to the `MailBackend` trait, never `io-imap` types.
- Every mutating action is local-first: SQLite write, enqueue, emit, repaint.
  **The UI never awaits the network.**
- Providers are data, not code: server settings live in the preset table,
  never as named constants or special-cased branches. Postio is not built
  for any one provider and the code must not say otherwise.
- **Pimalaya first** (maintainer, 2026-08-27): when a protocol or format
  need appears, check the Pimalaya family before writing wire code —
  Postio already stands on io-imap/io-smtp/io-http/io-sasl/io-oauth/
  io-pim-discovery/pimalaya-stream, and their release cadence means a
  survey more than a few weeks old is stale (#537 is what missing one
  cost). Replacing existing hand-rolled code with a Pimalaya crate is
  explicitly welcome.

## Privacy is a feature, and fixtures are public

**Nothing leaves this machine that the user did not ask for.** Remote images
blocked until allowed per sender; read receipts never sent automatically;
one-click unsubscribe only on deliberate activation; no prefetch, favicon
fetches, or speculative connections; the reader's WebKit view has JS and
network off. No telemetry. Credentials go in the OS keyring — never
`config.toml`, never a log. **Logs never carry message content**: ids,
counts, outcomes only.

This repo is public and its fixtures describe mailboxes: every email address
uses a reserved domain (`ada@example.com`; RFC 2606 or `.test`/`.invalid`/
`.example`/`.localhost`), and no real person's name or address appears
anywhere — least of all the maintainer's. Same rule for issues, PRs, and
commit messages, which are public and permanent: never paste real mail,
addresses, or unread logs. `check-no-personal-data.py` redacts its findings
by default; `--reveal` locally while fixing.

## Commits and git

Small, focused commits; commit as you go. Format (template in `.gitmessage`):
`<type>(<scope>): <summary>` — type from `feat fix docs test refactor perf
chore ci build revert`, scope the crate without prefix (or `workspace`, `ci`,
`docs`), summary imperative and ≤ 50 chars. Body explains **why**, wrapped at
72. Every commit ends with `Refs: #<issue>`; the PR body's `Closes: #<issue>`
does the closing. Every commit is green for the crates it touches.

Standing authorisation: committing, pushing your own issue branch, and
`--force-with-lease` on it after the land script rebases. Not authorised
without asking: pushing `main`, adding remotes, rewriting shared history,
bare `--force`.

**Fetch before you reason about the tree** (`git fetch origin main`) — other
sessions land while you work, and stale comparisons have produced confident
wrong reports. Rebase long-running branches as you go; re-read your issue
before finishing. A non-fast-forward rejection means main moved: fetch,
rebase, push again.

## One machine, several sessions

Parallel sessions are the normal state. Worktrees isolate the files; three
things stay shared:

- **CPU**: `.cargo/config.toml` pins `jobs = 2`. Raise per-command
  (`cargo build -j8`) only when you're alone.
- **The compile cache**: sccache, wired in automatically, one cache
  machine-wide. Each worktree keeps its own `target/` (sharing one compiled
  crates against a sibling's — #76).
- **The main checkout** `~/src/postio` is for coordination, not work. A hook
  refuses the destructive commands there (`git add -A`, `reset --hard`,
  `stash`, `cargo fmt --all`, editing the root `Cargo.toml`, …) because other
  sessions' uncommitted work lives in it; inside your worktree those same
  commands are safe and allowed.

## Say it where it persists

GitHub is where this project talks to itself; terminal output is read once by
one person and gone. Don't narrate progress — write it down where it belongs,
for a stranger who can't ask follow-ups:

| What | Where |
|---|---|
| Why the fix is shaped this way | the commit body |
| What you discovered on the way | a comment on the issue |
| Work this revealed | `scripts/issue-file.sh` — **search first** (`ready` only if startable unattended; post-v1 → `roadmap`, under its epic) |
| Something needing a design/architecture call an agent can make | `needs-architecture` — `/ux-architect`'s queue |
| Something only the maintainer can decide | `needs-maintainer`, plus a comment naming the question and the options |
| A constraint future sessions must respect | `docs/engineering-notes.md` |
| An architectural decision | an ADR in `docs/decisions/` |

**File through the script, because you will not think to search.** One bug
collected three issue numbers in two days (#332, #392, #406), both duplicates
filed by sessions that had just watched it happen. That is exactly when
searching feels redundant — you are not wondering whether the bug exists, you
saw it — so `issue-file.sh` searches for you and stops if it finds anything,
open or closed. When it does, **comment on what is there instead**: a new
occurrence on an existing issue is worth more than a second issue, because it
is evidence the bug survived a fix or has come back. `--anyway` files if yours
is genuinely different, and `--search-only` just looks.

## CI is paused

`ci.yml`/`bench.yml` are `workflow_dispatch`-only until the repo goes public
(Actions minutes). Landing therefore merges promptly without waiting — do not
add your own wait. The workspace is proven by the reconcile pass instead: the
steward loop runs `cargo test --workspace --no-fail-fast` against `main`
periodically. If it is ever red: pull `ready` from open issues, fix on a
branch, land it, restore the labels. A release needs a local full-suite run
first — `release.yml` ships without testing. To restore CI, uncomment the
triggers `ci.yml` and `bench.yml` name and delete this section.

## Skills and design authorities

`/issue` (the loop), `/lanes` (who else is here), `/preflight` (true state of
the tree), `/add-fixture`, `/ux-architect` (designing any surface),
`/gtk-design` (building it).

Product truth: `docs/PRODUCT.md`. Visual truth: the design canvas
(`Design/Mail Client.dc.html`, direction PLATE 1b) — spacing, color,
proportion defer to it. Keys: `e` reply, `a`/`A` archive, `u` undo, `t`
thread; all rebindable, table generated into `docs/keybindings.md`. Compose
takes over the reading pane. The sidebar says "Flagged". v1 scope: Linux,
IMAP+SMTP, one provider preset table, no AI (deferred to epic E12). OAuth is
in scope — ADR 0006, tracked under #2.
