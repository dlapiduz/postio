# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

## Issue tracking is GitHub

Every piece of work in this repository is a GitHub issue, and every issue is
worked in a **private git worktree** on its own branch. Start with the `/issue`
skill; it is three commands.

```bash
scripts/issue-claim.sh                  # take the next ready issue, get a worktree
cd ~/src/postio-worktrees/issue-<n>     # work there
scripts/issue-land.sh -m "feat(gtk): ..."   # gates, commit, push, PR
```

An issue is yours to take when it is open, labelled `ready`, unassigned, and
blocked by nothing still open. `blockedBy` is a native GitHub field, so that is
a fact rather than a convention. Never take `epic` (a container), `icebox`
(deferred), or `needs-architecture` (a human decides first).

Priority is `p0`…`p4`. The claim script takes the most important thing
available, not the newest.

- Do **not** use TodoWrite, TaskCreate, or markdown TODO lists. The issue is
  the tracker.
- Work that an issue reveals becomes `gh issue create`, labelled `ready` only
  if you would be happy for another session to start it unattended.
- Knowledge future sessions need goes in `docs/engineering-notes.md`, next to
  the reasoning it belongs with.

## Git authority for this repository

- **Commits are standing-authorised.** Commit as you go, and never end a
  session with uncommitted work. See **Commits** below.
- **Pushing an issue branch is standing-authorised.** A branch that exists to
  be reviewed cannot damage anything.
- **Pushing `main` is not.** Neither is adding a remote, force-pushing, or
  rewriting history. Ask in the current session.

## Build & Test

**Work per-crate. Reach for `--workspace` deliberately, not by habit.**

```bash
cargo test   -p <crate>                              # the inner loop
cargo clippy -p <crate> --all-targets -- -D warnings # before committing
rustfmt --edition 2024 <files you changed>           # ONCE, before committing
```

Occasional, not per-edit:

```bash
cargo test --workspace --no-fail-fast   # full picture; see the note below
cargo bench                             # perf budgets
cargo test -p <crate> -- --ignored      # live iCloud; needs POSTIO_TEST_* env
```

Three things measured across ~2000 tool calls in this project, which is why
the defaults above are what they are:

- **Roughly 30% of all tool calls were re-running a gate.** `cargo test` ran
  258 times, `cargo fmt` 177, `clippy` 92, `build` 68. Gates are cheap to type
  and expensive to run, so they get run reflexively.
- **`cargo fmt` was the single most-run gate.** It is not verification -- it is
  a formatter. Run it once before you commit, not after every edit.
- **`cargo test --workspace` compiles and runs all nine crates, including GTK.**
  With several sessions active it also serialises on the shared target
  directory, so a habitual workspace test is the largest wall-clock cost in the
  build. Verify your own crate; let CI prove the workspace.

Always pass `--no-fail-fast` to a workspace test. Plain `cargo test` aborts
remaining targets after the first failure, so one broken crate hides a thousand
passing tests and the totals look catastrophic. This has already caused a false
alarm.

**Format files, not crates.** `cargo fmt --all` and `cargo fmt -p <crate>` both
*write*, and both reach beyond what you changed — `-p` reformats every file in
the crate, including one another session has open and uncommitted. That has
already put whitespace churn into somebody else's diff. Use
`rustfmt --edition 2024 <files>` with your own paths named explicitly.
The guard hook refuses a file list produced by an unscoped `git diff`, which
reaches every crate anyone has edited. The `--check` forms are read-only and safe.

**GTK tests belong on their own display.** `postio-gtk` has about twenty test
binaries that call `window.present()`, and on a live session every one of them
throws a window onto the maintainer's desktop and steals focus mid-keystroke.
Run them through a headless compositor instead:

```bash
scripts/test-headless.sh cargo test -p postio-gtk
scripts/test-headless.sh --stop            # when you are done for the day
```

It starts `mutter --headless` on a display of its own and reuses it. mutter
rather than Xvfb because it is GNOME's own compositor and already installed, so
the tests keep running on Wayland against the thing the application targets
instead of under XWayland.

It is also *faster* than the real session, which is not free: a headless run
finishes `gtk_accessibility` in 0.40s against 1.44s on the live display, and
that gap is enough to expose tests that `pump()` once and then assert on
content which arrives asynchronously. If a test passes on your desktop and
fails headless, suspect the test before the harness — see `postio-9112`.

System dependencies (Fedora 40+; this box is Fedora 44 / GNOME 50 / Wayland):

```bash
sudo dnf install gtk4-devel libadwaita-devel webkitgtk6.0-devel \
                 sqlite-devel libsecret-devel glib2-devel pkgconf-pkg-config
```

Verified working against: gtk4 4.22.4, libadwaita-1 1.9.3, webkitgtk-6.0 2.52.5,
sqlite3 3.51.2, libsecret-1 0.21.7, glib-2.0 2.88.3.

## Running the app

The working tree is edited continuously by several sessions, so building from
it gives you whatever half-finished state is on disk at that moment. To look at
a real, running Postio:

```bash
scripts/run-isolated.sh                 # build and run HEAD
scripts/run-isolated.sh <commit>        # a specific commit
scripts/run-isolated.sh HEAD --inspect  # with the GTK Inspector
scripts/run-isolated.sh HEAD --shot     # render a PNG instead of opening
scripts/run-isolated.sh --clean         # discard the worktree and its store
```

It builds from a **git worktree pinned to a commit**, with its own
`CARGO_TARGET_DIR` and its own `XDG_DATA_HOME`/`XDG_CONFIG_HOME`. So the app is
a known commit rather than a moving tree, the build cannot contend with or
poison the shared `target/`, and the running app reads a throwaway store that
cannot reach real mail.

**Do not run this while other sessions are building.** It uses its own
`CARGO_TARGET_DIR`, so it duplicates the whole build rather than sharing one,
and it links `--release`, which is the heaviest thing that happens on this
machine. Four concurrent sessions already saturate eight cores; adding an
isolated release build on top is what put the box into swap. It is a tool for
looking at the app when you are present, not something to run unattended.

**To prove a change reaches the running application, use the integration
tests** in `crates/postio-app/tests/` instead — `wiring.rs`, `keystroke.rs`,
`search_index.rs`. That is exactly what `postio-bl2` built them for: the
composition root is testable without launching a GUI, and it is the layer
where eight wiring bugs hid.

**`POSTIO_LOG` controls logging, not `RUST_LOG`.** `postio-app/src/logging.rs`
installs a real `tracing` subscriber before anything else runs; `POSTIO_LOG`
takes an `EnvFilter` directive (`debug`, or scoped like
`postio_sync=debug,postio_runtime=debug`), and `[logging]` in `config.toml`
does the same to an already-running instance, live. The script sets
`POSTIO_LOG=info` and `RUST_BACKTRACE=1` by default. It deliberately does
*not* set `G_MESSAGES_DEBUG=all` — that produced two hundred lines of Vulkan
and portal noise and not one line about mail; set it yourself when the
problem is actually GTK's. `--inspect` attaches the GTK Inspector on top of
all that — live widget tree, CSS, and property inspection.

For a quick visual check of a widget without running the app,
`cargo run -p postio-app --example shot` renders straight out of GSK and needs
no display server. It lives in `postio-app` rather than beside the widgets
because its `demo` mode reads a seeded store, and `postio-gtk` may not depend
on `rusqlite` — not even as a dev-dependency, which is what an example is
built from.

## Development rules

> **You are probably not alone in this repository.** Other Claude sessions are
> working other issues right now. Claim yours with `scripts/issue-claim.sh` and
> work in the worktree it gives you — inside it the destructive git commands are
> safe, and in the shared `main` checkout they are not. See **How work happens
> here** at the end of this file before your first commit.


### Test-driven development is mandatory

**Write the failing test first, then the implementation.** This is not a
preference — it is how this repository is built, and it applies to every crate.

- An issue is not done until its acceptance criteria are covered by tests.
- Verify with `cargo test -p <your-crate>` while working, and run the full
  gate chain once via `scripts/issue-land.sh` before committing. See **Build & Test** above
  for why the inner loop is per-crate.
- `postio-model`, `postio-storage`, `postio-search`, `postio-config` and the sync
  reconciliation logic are pure logic and must have thorough unit coverage.
- Protocol code is tested against the `MailBackend` mock and the `.eml` corpus in
  `crates/postio-model/tests/corpus/`. **No test in the default suite may touch
  the network.** Live-server tests are `#[ignore]`.

### It must feel instant

Performance is a functional requirement, enforced by benches in CI rather than
checked by hand at the end:

| Budget | Target |
|---|---|
| Startup to usable UI (populated DB) | < 500 ms |
| Ordinary UI interaction | < 16 ms |
| Local search | < 100 ms |

Motion budget: **transitions are <= 100 ms or absent.** Pane switches and thread
drill-in use *no* transition. Always honor `prefers-reduced-motion`. Never load a
whole mailbox into memory — the message list is windowed over paged SQLite.

### No personal data, no provider hard-coding

This project is open source and its fixtures describe mailboxes, so a
maintainer's own identity leaks in easily. Two rules, enforced by
`scripts/check-no-personal-data.py` in CI:

- **Every email address in the repository uses a reserved domain** — RFC 2606
  `example.com`/`.net`/`.org`, or the `.test`, `.invalid`, `.example`,
  `.localhost` TLDs. Invent the people: `Ada Lovelace <ada@example.com>`.
  Hostnames such as `imap.example.com` are not addresses and are unaffected.
- **Never use a real person's name or address in a fixture**, least of all the
  maintainer's. The check reads this checkout's git identity at run time and
  fails if it appears in a tracked file. In CI that list comes from the
  `POSTIO_DENY_NAMES` repository secret instead, since a runner's git config
  is the bot's.

The check's output is **redacted by default** — it prints the location and the
rule, never the value — because CI logs on a public repository are public, and
a check that printed what it found would publish exactly what it protects. Use
`python3 scripts/check-no-personal-data.py --reveal` locally while fixing.

Providers are **data, not code**. `docs/PRODUCT.md` §3 requires provider
configuration to be extensible rather than hard-coded, so server settings
belong in a preset
table where every provider is one row — never a named constant, a special-cased
branch, or an identifier like `ICLOUD_IMAP_HOST`. Postio is not an iCloud
client; iCloud is one preset among many, and the maintainer's own provider must
not be visible in the shape of the code.

Naming a provider in a comment is fine where it explains a real-world
compatibility quirk ("some servers spell it `Sent Messages`"), as is a test
fixture named for the *behaviour* it replays rather than the vendor.

### Privacy is a feature, not a setting

Email is the most sensitive thing on most people's machines, and mail is
attacker-controlled content that actively tries to phone home. Postio's
commitment is one sentence: **nothing leaves this machine that the user did not
ask for.**

- Remote images and tracking pixels are blocked until the user allows them,
  per sender. Never a global default-on.
- **Read receipts are never sent automatically.** `Disposition-Notification-To`
  is tracking with a friendly name.
- `List-Unsubscribe` One-Click only fires on deliberate activation -- sending it
  confirms to a spammer that the address is live.
- No link prefetch, no favicon fetch, no speculative connections from the
  reader. The hardened WebKit view has JavaScript off and network access off;
  `cid:` images resolve from the local blob store.
- No telemetry, no crash reporting, no update ping.
- Credentials live in the OS keyring, never in `config.toml`, never in a log.
- **Logs never carry message content** — no bodies, subjects, or recipient
  addresses, at any level. Log ids, counts and outcomes. A debug log full of
  someone's mail is the same leak as shipping their address in a fixture.

When adding anything that could make a network request, the question is not
"is this useful" but "did the user ask for it". If the answer is no, it does
not ship. `postio-qhz.2` tracks proving this with a request log rather than
asserting it.

### Architectural invariants (CI enforces these)

- `postio-core` must not depend on `gtk4`/`libadwaita`. It is the UI-agnostic
  runtime: commands in, events out. This is what makes a macOS frontend possible.
- `postio-gtk` must not depend on `rusqlite` or `io-imap`. No SQL, no protocol.
  This is why `postio-runtime` and `postio-app` are separate crates rather than
  features of `postio-core`. Cargo resolves features as a *union* across
  everything being built, so a `postio-core/runtime` feature would put SQLite in
  the graph of every crate depending on `postio-core` the moment anything turned
  it on — the view layer included, which in a workspace build really would link
  the SQL. `postio-core` therefore has no optional dependencies at all.
- `postio-sync` talks to the `MailBackend` trait, never to `io-imap` types
  directly — that crate is pre-1.0 and moving fast.
- Every mutating action is local-first: SQLite write, enqueue the operation, emit
  the event, repaint. **The UI never awaits the network.**
- Secrets go in the Secret Service keyring. Never in `config.toml`, never logged.

## Commits

This is an open-source project. The history is part of the product — someone
will read it to understand why the code is the way it is.

**Small, focused commits.** One logical change each. A commit should be
reviewable in a single sitting and revertable without collateral damage. If you
find yourself writing "and" in the subject, it is two commits.

`git config commit.template .gitmessage` is set up in this repo; the template
carries the full format. In short:

```
<type>(<scope>): <summary>

Why this change exists, wrapped at 72 columns. Skip the body when the
subject genuinely says everything.

Refs: postio-abc
```

- **type**: `feat` `fix` `docs` `test` `refactor` `perf` `chore` `ci` `build` `revert`
- **scope**: the crate without its `postio-` prefix (`model`, `storage`, `search`,
  `config`, `imap`, `smtp`, `sync`, `core`, `gtk`), or `workspace`, `ci`, `docs`
- **summary**: imperative mood, lower case, no trailing period, <= 50 chars —
  "add draft autosave", not "Added draft autosave."
- **body**: explain **why**, not what. The diff already says what. Write one
  whenever the change encodes a decision, a trade-off, or a constraint that is
  not obvious from the code.
- **footer**: every commit carries `Refs: #<issue>`. The PR body carries
  `Closes: #<issue>`, which is what actually closes it on merge. Note
  `BREAKING CHANGE:` when applicable.

**Every commit must be green** for the crates you touched — `cargo build`,
`cargo test -p <your-crate>`, `cargo clippy -p <your-crate> --all-targets --
-D warnings`, `cargo fmt -p <your-crate> --check`,
`python3 scripts/check-crate-boundaries.py`, and
`python3 scripts/check-no-personal-data.py`. Keep working until it is green
rather than committing a broken state — and never `git stash` to get there,
which would stash every other session's work too.

### Never leave work uncommitted

**Committing is standing-authorised in this repository — you do not need to ask.**
Pushing still does: never `git push`, add a remote, or rewrite history without
being asked.

Commit each piece of work as you finish it. Do not batch a session's work into one commit
at the end, and never end a session — or go idle waiting on the user — with
uncommitted changes in the tree.

Uncommitted work is *unprotected* work. Sessions get cut off by usage limits
mid-task, and anything not committed is one `git reset --hard` away from being
gone, in a tree that other sessions are editing. This has already cost this
project a scare: roughly fifty files of finished work sat loose in the tree
after four sessions were interrupted at once.

If you are interrupted or must stop mid-issue, commit what you have rather
than leaving it loose. Mark it plainly and leave the issue open:

```
feat(storage): begin the operation queue drainer

Work in progress -- retry classification is not implemented yet.

Refs: postio-abc
```

Never commit secrets — no passwords, no tokens, no real email addresses in
fixtures. Two CI checks enforce this; run them before you commit.

## Design and scope

- **Approved plan:** `~/.claude/plans/ethereal-fluttering-kettle.md`
- **Product spec:** `docs/PRODUCT.md`
- **Design canvas:** `Design/Mail Client.dc.html` — the chosen direction is
  **PLATE (option 1b)**: airy desktop, 40px rows, key hints on the focused row only.

`docs/PRODUCT.md` records the resolved product decisions and already agrees
with the canvas — where the two once differed, the canvas was newer and won,
and the resolution is now simply what the document says:

- Keys are `e` reply, `a` archive, `A` archive thread, `u` undo, `t` thread.
  Every binding is overridable via `[keys]`; the table is `docs/keybindings.md`,
  generated from the registry.
- Compose takes over the reading pane; it is not a separate window.
- The sidebar says "Flagged", not "Starred".

The **canvas remains the authority on visual detail** — spacing, colour,
proportion. `docs/PRODUCT.md` §19 defers to it rather than restating it.

Visual target: keep the Industry design system's *identity* — Barlow / Barlow
Condensed / IBM Plex Mono, steel accent `#5980a6`, hairline dividers, airy rows,
accent-tinted selected row with a 3px left border. Drop its *wireframe chrome* —
no blueprint corner registration marks, no transparent line-drawing cards. Keep
real Adwaita window chrome so it reads as a GNOME application.

v1 scope: Linux only, IMAP+SMTP only, iCloud with an app-specific password (no
OAuth). SQLite for metadata/threading/sync-state/FTS5 plus a content-addressed
blob directory for raw messages and attachments — no maildir/mbox/notmuch.
**No AI in v1** — it is a founding principal but deliberately deferred to epic
E12 so the core mail experience lands first.

### Docs site and landing page

`docs/PRODUCT.md`, the design canvas, and this file are written for
contributors, not users — none of them are where someone deciding whether to
try Postio should land. The project has two more surfaces, both GitHub Pages,
both tracked as GitHub issues:

- **A docs site** that documents the *app*: what it does, keyboard shortcuts,
  the `config.toml` reference, the privacy/security posture. Shortcut and
  config references should be generated from the same sources of truth the
  app itself uses (the command registry that drives the in-app `?` cheat
  sheet; the TOML schema) rather than hand-maintained a second time.
- **A landing page** that is deliberately more human than the docs site or the
  README: the north star line as the actual headline, plain language, real
  screenshots of the running app, and a link into the docs site for anyone
  who wants depth. Not a restatement of `docs/PRODUCT.md`.

### The roadmap is grouped into epics

Post-v1 work — multi-account, OAuth, AI, filters, the docs site and landing
page above — lives under themed epics in the
[Postio Roadmap](https://github.com/users/dlapiduz/projects/2) project, wired
with real GitHub sub-issue links. File new post-v1 work as an issue under the
relevant epic, labelled `roadmap`, and leave `ready` off it: the roadmap is a
plan, not a queue.

### Persistent knowledge lives in docs/engineering-notes.md

Hard-won lessons go in `docs/engineering-notes.md`, organized by topic — the
file explains its own conventions at the top. Product-scope decisions and
post-v1 ideas belong in its "Product scope & design decisions" section, or as
a GitHub issue if they describe work rather than a lesson.

## Architecture

Layers, not a tree: a crate's rank is its position, and shared leaves are
shared rather than owned by one parent.

```
  frontend    postio-app     composition root + GTK binary. The only crate
      |                      that knows both halves exist.
      +------ postio-gtk     GTK4 + libadwaita + WebKitGTK. Widgets, CSS,
      |                      keymap. Command down / Event up. No SQL, no
      |                      protocol.
      |
  engine  +-- postio-runtime The database half: the store, and the loop that
          |                  drains the queue, backfills bodies, reconnects.
          +-- postio-sync    operation queue, QRESYNC resync, IDLE, backoff
          |     +-- postio-imap (io-imap)   postio-smtp (io-smtp)
          +-- postio-storage SQLite, migrations, repositories, blob store
          +-- postio-index   FTS5 index and executor  (owns rusqlite)

  contract  postio-core      commands, events, registry, app state, undo,
      |                      tokio<->glib bridge. No GTK -- CI enforced.
      +------ postio-config  TOML schema, validation, watcher, live reload

  domain    postio-model     pure domain types + JWZ threading
            postio-search    query parser, highlighter, facets. Pure: no SQL,
                             no toolkit. A SHARED leaf, not a GTK detail --
                             postio-gtk, postio-index, postio-runtime and
                             postio-app all depend on it.
```

`postio-search` is the query *language*; `postio-index` is the FTS5 *index*
that executes it. They were one crate once and the tree above used to say so.
Keep them apart: the same query string has to mean the same thing in the
search bar, in the sidebar, and in `[filters]` in `config.toml`.

See `docs/ARCHITECTURE.md` for the decisions behind this shape,
`docs/decisions/` for the ADRs, and `docs/architecture-review-2026-08.md`
for the known gaps.

## How work happens here

Every session runs the same loop. Take an issue, work it in a worktree of your
own, land it as a PR, take the next one.

```bash
scripts/issue-claim.sh                  # next ready issue, highest priority first
cd ~/src/postio-worktrees/issue-<n>
export CARGO_TARGET_DIR=~/src/postio/target   # keeps GTK and WebKit warm
# ... write the failing test, then the code ...
scripts/issue-land.sh -m "feat(gtk): ..."     # gates, commit, push, PR, merge
scripts/issue-release.sh <n>            # remove the worktree
```

`issue-land.sh` **waits for CI and merges on green**. Opening a PR is not
finishing: an unmerged branch goes stale, conflicts with whatever lands next,
and leaves the issue open. If a check fails, that is yours to fix on the same
branch — not something to hand to whoever reads the PR list next.

### Skills

Use these rather than reconstructing the commands:

| Skill | Use it when |
|---|---|
| `/issue` | The loop above, in full — claim, work, land, release |
| `/lanes` | Starting up: who else is here, what is safe to take |
| `/preflight` | Checking the real state of the tree, or when it looks broken |
| `/add-fixture` | Adding `.eml` test mail to the corpus |
| `/ux-architect` | Designing any surface, flow, or interaction |
| `/gtk-design` | Building it: tokens, GTK traps, motion, render-to-PNG |


**Finishing an issue is not finishing a session.** Run
`scripts/issue-claim.sh` and keep going. Keep going for as long as you can.

**Do not ask whether to continue.** Nobody is waiting to be asked, and a
session that stops to check has thrown away the rest of its context for
nothing. "Shall I pick up another?", "Would you like me to keep going?" and
"Let me know if you want me to take the next one" are all the same mistake:
the answer is yes, it is written here, and asking costs a round trip that may
not come back for hours.

There are exactly three reasons to stop, and none of them is politeness:

1. **`scripts/issue-claim.sh` says there is nothing ready.** Say so and stop.
   Do not go hunting in the backlog; an issue without `ready` has not been
   triaged, and `epic`, `icebox` and `needs-architecture` are deliberately
   closed to you.
2. **A decision is genuinely the maintainer's** — scope, product direction, or
   a trade-off with no defensible default. Not "which of these two names is
   nicer": make that call and say what you chose.
3. **Context is nearly gone.** Land your work first, then say where you got to.

Land what you have before you stop, whichever reason applies. Uncommitted work
is unprotected work.

### Stay current

`main` moves while you work. Several sessions land to it, and a long piece of
work can easily be five commits behind by the time it is ready — one recent
change was rebased onto four commits that arrived mid-task.

The scripts handle the two moments that matter: `issue-claim.sh` cuts your
branch from a freshly fetched `origin/main`, and `issue-land.sh` fetches and
rebases onto it before pushing. You do not need to think about either.

What is left to you:

- **Fetch before you reason about the tree.** `git log`, `git diff` and
  anything comparing against `origin/main` are reading a snapshot that may be
  hours old. `git fetch origin main` first, or you will draw conclusions from
  a repository that no longer exists — which has already produced one confident
  and wrong report here.
- **Rebase a long-running branch as you go**, not only at the end. A day's
  work rebased once is a merge conflict; rebased as you go it is nothing.
- **Re-read an issue before you finish it.** Someone may have commented,
  decided something, or closed it while you worked.
- **If a push is rejected as non-fast-forward, that is this**, not a mistake:
  fetch, rebase, push again. Never force-push — the guard hook refuses it, and
  on a shared branch it discards whatever landed since you last looked.

### Say it in the issue, not in the terminal

**GitHub is where this project talks to itself.** An issue comment, a PR body,
a commit message: those persist, they are searchable, they are attached to the
thing they describe, and the next session finds them without anyone relaying
anything. Output printed into a Claude session is read once, by one person, and
is gone.

So the default is to write it down where it belongs:

| What you found | Where it goes |
|---|---|
| Why the fix is shaped this way | the commit body |
| What you discovered while doing the work | a comment on the issue |
| Work this revealed | a new issue |
| A constraint future sessions must respect | `docs/engineering-notes.md` |
| An architectural decision | an ADR in `docs/decisions/` |

**Printing to the session is superfluous unless a decision is needed.** Do not
narrate progress, restate what the diff already says, or summarise work that
is already recorded in a PR. Speak up when — and only when — you need the
maintainer to choose something you cannot choose yourself, and then say what
you need and why, briefly.

### Work in the open

This repository is open source. Assume every issue, comment, PR, branch name
and commit message is public the moment you write it, and permanent after
that — deleting it later does not un-publish it.

Practically:

- **Never write an address, a name, or a credential** into an issue, a commit,
  or a fixture. `scripts/check-no-personal-data.py` enforces this on tracked
  files; it cannot see an issue comment you wrote by hand.
- **Never paste real mail** — no subjects, no bodies, no recipients — into a
  bug report. Describe the shape of the message and use the reserved-domain
  corpus in `crates/postio-model/tests/corpus/`.
- **Logs and stack traces get read before pasting**, for the same reason.
- **Write for a stranger.** The reader is a contributor who was not here, has
  no context, and cannot ask you a follow-up question. That is the standard
  every issue comment and commit body is held to.
- Do not name a provider as though Postio were built for it, and do not
  describe peer projects as competitors. See the rules above on providers.

### Several sessions run at once

That is the normal state of this project. Each session has its own worktree, so
the file-level collisions are gone — you can `git add -A`, `git commit -a`,
`git stash` and `cargo fmt --all` inside your worktree, because nothing else is
writing there. Isolation is by branch, so an issue spanning several crates is
one piece of work rather than a handoff.

Three things are still shared, and still bite:

- **One cargo target directory.** Builds serialise on it rather than running in
  parallel. That is contention, not breakage. `Cargo.lock` churn is expected.
- **One machine.** Four concurrent builds saturate eight cores; `.cargo/config.toml`
  pins `jobs = 2` for that reason. Never run `scripts/run-isolated.sh` while
  others are building — it links `--release` in a target directory of its own,
  which is what put this box into swap.
- **One `main` checkout at `~/src/postio`.** It is for coordination, not for
  work. The commands below are still refused there, by
  `.claude/hooks/guard-shared-tree.py`, because uncommitted work of other
  sessions lives in it.

### In the main checkout, never do these

| Don't | Why | Do instead |
|---|---|---|
| `git add -A`, `git commit -a` | Commits other sessions' half-written files | Work in your worktree, where it is safe |
| `git reset --hard`, `git checkout .` | **Irrecoverably deletes** uncommitted work across every crate | Revert your own files, by path |
| `git stash` | Stashes *everyone's* changes | Leave it alone |
| `cargo fmt --all` or `-p <crate>` | Reformats files being edited right now | `rustfmt --edition 2024 <named paths>` |
| `rustfmt $(git diff --name-only ...)` | Same hazard, wider — it reaches every crate anyone touched | Name your paths, or a scoped pathspec |
| `git rebase`, history rewrites | Others hold refs that become invalid | Only when the maintainer confirms the tree is quiet |
| Editing the workspace root `Cargo.toml` | Sessions colliding on one manifest | `cargo add -p <crate> <dep>` |

The hook refuses these rather than trusting anyone to have read this far, and
exempts your worktree, where they are correct. Its test suite is
`.claude/hooks/test-guard-shared-tree.py`.

### Testing

**Run GTK tests on a compositor of their own**, or ~20 test binaries throw
windows onto the maintainer's desktop:

```bash
scripts/test-headless.sh cargo test -p postio-gtk
```

It is ~3.5x faster than a live session and will expose races a real compositor
hides. If something passes on the desktop and fails there, suspect the code.

**A test that needs a display goes in `tests/`, never in `src/`.** GTK may be
initialized once per process and `cargo test` runs a crate's unit tests on a
thread pool in one binary, so a second `adw::init()` does not fail a test — it
kills the process, and every other test in that crate goes unreported.
`crates/postio-gtk/tests/gtk_toast.rs` is the worked example;
`scripts/check-no-gtk-init-in-unit-tests.py` enforces it. See #41.

**Verify your tests can fail.** Inject the regression each one exists to catch
and confirm it goes red. A session once closed a bug on four green runs of a
test that failed half the time — four coin flips. An await-for-condition test
can silently become one that cannot fail.

**CI is the workspace's judge, not you.** Verify your own crates with
`cargo test -p <crate>`; a red crate you do not own is usually someone's
in-flight TDD. Note it on your issue and move on.
