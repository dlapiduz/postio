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
scripts/test-fast.sh                                 # between edits: changed crates, --lib
scripts/test-sanity.sh                               # before landing: whole workspace, --lib
cargo test   -p <crate>                              # that crate, integration included
cargo clippy -p <crate> --all-targets -- -D warnings # before landing
scripts/check.sh                                     # every repository invariant
```

**Three tiers, and picking the right one is most of what makes iterating here
cheap.** Measured, warm, on this workstation:

| | tests | cost |
|---|---|---|
| `scripts/test-fast.sh` — changed crates, `--lib` | varies | seconds |
| `scripts/test-sanity.sh` — whole workspace, `--lib` | 1,313 | ~5s, 19 binaries |
| the full suite — integration too | 3,169 | ~497s on CI; `app_suite` alone ~11 min to link |

`issue-land.sh` runs the **sanity tier** by default; `--full` adds the
per-crate integration suites. That default exists because several sessions
share this machine with `jobs = 2`, so landing had become something you
queued for.

**Land on the default. `--full` needs a specific reason, and "this change is
about wiring" is not one** (maintainer, 2026-09-03: *"dont run the full gate
if you dont need to"*). Before landing, run the suites your diff actually
touches — `cargo test -p <crate> --test <suite>` — which is seconds, aimed at
what changed, and re-runnable; then let CI run the rest. `--full` re-runs what
you already ran, inside a ~25-minute chain where any unrelated flake restarts
the whole thing.

#901 is the worked example: three consecutive `--full` runs failed, none of
them on a defect in the change — a `gtk_suite` segfault (#988), an `e2e`
timeout at 121s that passed in 12s alone, and four SQLCipher `PRAGMA key`
failures (the documented #710/#699). The fourth attempt, on the default,
found a **real** compile error in two minutes: another session had landed a
second construction site for a struct the branch had added a field to. Note
what that means — `--full` could not have caught it and the default did not
need to. **The rebase is what finds a shared type's new callers**, and
`issue-land.sh` rebases on every attempt whatever the tier.

**A gate failure in code your diff does not touch is probably not yours.**
Check before re-running: reproduce it alone, read the backtrace
(`coredumpctl debug` for a segfault), search for prior art in the issues and
in `docs/engineering-notes.md`. Three of #901's four gate failures were
pre-existing and two of them became issues. Re-running without looking turns
somebody else's bug into your twenty-five minutes, repeatedly.

**It is safe only because CI still runs the whole workspace on every pull
request**, and the nightly job runs it again. Unit tests are precisely the
tier that cannot see this project's characteristic bug — layers that each
pass and are not joined up, like the Reader that was built, tested and never
mounted. Do not read the fast default as permission to skip integration
tests: write them, and let CI be the thing that runs them.

**Iterate at the cheapest layer that can fail.** The tests are not what costs;
compiling and linking is. On this workstation `postio-body`'s 49 unit tests run
in 0.00s and `postio-gtk`'s 330 in 0.42s, while one `postio-app --test
app_suite` run is eleven and a half minutes, nearly all of it link. TDD pays
that twice — once for red, once for green — so a cycle through an integration
binary costs twenty minutes for two minutes of thinking.

`scripts/test-fast.sh` runs `--lib` for the crates you changed and links
nothing else; use it between edits. Run the integration suites to *confirm*, at
the end, not to iterate. This is also an argument about where logic lives: a
rule expressed as a function in `postio-core`, `postio-ui` or `postio-body` can
be proven red in a second, and the same rule buried in a widget cannot. It does
not license asserting on what a layer was handed instead of what a person would
see — that is what the integration suites and `issue-land.sh` are still for.

**Test the crates you changed; the reconcile pass proves the rest.**
`issue-land.sh` runs the gate chain (fmt, clippy, the sanity tier,
`check.sh`) over exactly your changed crates — plus one `cargo check --workspace
--all-targets`, because a shared type's blast radius is wider than the crate
list describes (#419) — and the steward loop periodically runs the
whole workspace against `main` — so a workspace build or test from an
ordinary session is almost always waste, and a red crate you didn't touch is
usually someone's in-flight TDD: note it on your issue and move on. Don't
re-run gates ritually either — a third of all tool calls ever made in this
repository were re-running a gate that `issue-land.sh` was going to run
anyway. `cargo fmt` is a formatter, not verification: inside your worktree
`cargo fmt --all` is fine (the land script runs it); in the shared checkout
format only files you changed, by name: `rustfmt --edition 2024 <paths>`.

**Run `issue-land.sh` in the background, always** — a full gate chain can
outlive a foreground tool call's 10-minute cap, and a run killed mid-gates
commits nothing and leaks every live test's `/dev/shm` scratch. Launch it
backgrounded, do something else or nothing, and act on the completion
notification; never poll for it and never re-run it because it is quiet. A
run that *was* killed is cheap to retry: green gates are recorded against
the exact tree, so an unchanged retry skips straight to the landing.

- **Tests are headless automatically.** The cargo runner puts test binaries on
  a private mutter compositor; `cargo run -p postio-app` and examples reach
  the real display. `POSTIO_HEADLESS=0 cargo test` to watch a run;
  `scripts/test-headless.sh --stop` to stop the compositor. Headless is ~3.5x
  faster than a live display — a test that passes on the desktop and fails
  headless usually has a real race (see `docs/engineering-notes.md`).
- **The reconcile pass**, when you are the one doing it: `cargo check
  --workspace --all-targets` first — it is what catches a *test* target that
  stopped compiling, which is how `main` went red twice in one day (#419),
  and it answers before the test run has finished linking — then `cargo test
  --workspace --no-fail-fast`, always `--no-fail-fast`, because plain cargo
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

Startup < 500 ms, interaction < 16 ms, local search < 100 ms. Transitions
≤ 100 ms or absent; honor `prefers-reduced-motion`. Never load a whole
mailbox into memory — the list is windowed over paged SQLite.

**Gated as counts, not as timings.** `bench.yml` compiles the bench targets
nightly and deliberately times nothing, because a shared runner cannot defend
16 ms — so what gates a PR is the *cause* of each budget, counted:
`postio_storage::test_support::counting` reads statements, rows and trigger
firings off SQLite's trace hook, and those are the same numbers on any
machine. When you touch a read path, that is the thing to add an assertion to;
`docs/engineering-notes.md` has what the three counts can and cannot see.

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
- **No backwards compatibility** (maintainer, 2026-09-03): *"we shouldnt
  worry about backwards compatibility, im the only user so far."* There are
  no deployed installs to protect, so write the clean version. A new column
  takes a plain default and old rows may be rebuilt or resynced — say that in
  one line rather than arguing it at length; no compatibility shims, no
  deprecation paths, no "in case something relied on this" branches. Still
  write the migration, and still explain what a column *means*: what goes is
  the argument about not disturbing what came before.
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

  **So claim with `--reuse` by default.** A *fresh* worktree is a cold
  `target/`: Postio's own ~20 crates rebuilt before the first gate result —
  595s of sanity tier plus 289s of workspace check on #1012's own landing,
  and that was a change with no Rust in it at all. `scripts/issue-claim.sh
  --reuse` takes the next issue in the worktree you are already in and keeps
  that build. It is **not** the sharing #76 forbids: that is two worktrees
  writing one target, and this is one worktree with a different branch
  checked out.

  Claim fresh only when you need the old tree kept — an unlanded branch you
  mean to come back to, or work you have not finished with. You do not have
  to decide carefully: `--reuse` refuses on its own if the tree is dirty,
  holds commits that are not on `main`, or is the shared checkout, and each
  refusal names its reason and changes nothing. Trying it first costs
  nothing when it does not apply.
- **The main checkout** `~/src/postio` is for coordination, not work. A hook
  refuses the destructive commands there (`git add -A`, `reset --hard`,
  `stash`, `cargo fmt --all`, editing the root `Cargo.toml`, …) because other
  sessions' uncommitted work lives in it; inside your worktree those same
  commands are safe and allowed.
- **A worktree belongs to one session**, and the same hook enforces it —
  wherever you arrived from, not only through `issue-claim.sh` (#412). The
  first session to work in a worktree holds it; another session's commands
  there are refused, and so is a write reaching in from outside. A claim frees
  itself after 45 minutes of silence, so a dead session strands nothing.

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

## CI runs on every pull request

`ci.yml` gates each PR and each push to `main`; `bench.yml` compiles the bench
targets nightly. Both were `workflow_dispatch`-only while this repository was
private and paying for its own minutes — that ended when it went public.

**`issue-land.sh` waits for the checks and merges when they pass.** Do not add
a wait of your own, and do not merge around a red one: a check that fails on
your PR is your work to fix, on the same branch, however green the crates you
touched were locally. The gate chain proves the crates a branch changed; CI is
the only thing that proves the *combination*, which is the failure two branches
that are each green alone can produce together.

The steward loop's periodic `cargo check --workspace --all-targets` and
`cargo test --workspace --no-fail-fast` against `main` are now a backstop
rather than the only proof. If either is ever red: pull `ready` from open
issues, fix on a branch, land it, restore the labels. A release still needs a
local full-suite run first — `release.yml` ships without testing.

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
