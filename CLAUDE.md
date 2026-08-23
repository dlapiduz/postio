# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:6cd5cc61 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->

## Git authority for this repository

The Beads block above describes a conservative default profile. **This project
overrides it**, and the Beads block itself defers to explicit repository
instructions:

- **Commits are standing-authorised.** Commit each bead as you finish it. Do not
  ask first, and do not leave work uncommitted. See **Commits** below.
- **Pushes are not.** Never `git push`, add a remote, or rewrite history unless
  the user asks in the current session.



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
`rustfmt --edition 2024 <files>`; `/land` derives the list from your own
changes. The `--check` forms are read-only and safe.

System dependencies (Fedora 40+; this box is Fedora 44 / GNOME 50 / Wayland):

```bash
sudo dnf install gtk4-devel libadwaita-devel webkitgtk6.0-devel \
                 sqlite-devel libsecret-devel glib2-devel pkgconf-pkg-config
```

Verified working against: gtk4 4.22.4, libadwaita-1 1.9.3, webkitgtk-6.0 2.52.5,
sqlite3 3.51.2, libsecret-1 0.21.7, glib-2.0 2.88.3.

## Development rules

> **You are probably not alone in this repository.** Other Claude sessions work
> other crates in this same working tree, concurrently. Claim your bead with
> `bd update <id> --claim`, stay inside your crate, and never run a command that
> touches files you do not own — `git add -A`, `git reset --hard`, `git stash`
> and `cargo fmt --all` all destroy or corrupt other sessions' work. See
> **Working in parallel** at the end of this file before your first commit.


### Test-driven development is mandatory

**Write the failing test first, then the implementation.** This is not a
preference — it is how this repository is built, and it applies to every crate.

- A bead is not done until its acceptance criteria are covered by tests.
- Verify with `cargo test -p <your-crate>` while working, and run the full
  gate chain once in `/land` before committing. See **Build & Test** above
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

Providers are **data, not code**. `spec.md` §3 requires provider configuration
to be extensible rather than hard-coded, so server settings belong in a preset
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

When adding anything that could make a network request, the question is not
"is this useful" but "did the user ask for it". If the answer is no, it does
not ship. `postio-qhz.2` tracks proving this with a request log rather than
asserting it.

### Architectural invariants (CI enforces these)

- `postio-core` must not depend on `gtk4`/`libadwaita`. It is the UI-agnostic
  runtime: commands in, events out. This is what makes a macOS frontend possible.
- `postio-gtk` must not depend on `rusqlite` or `io-imap`. No SQL, no protocol.
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
- **footer**: every commit carries `Refs: <bead-id>`. Use `Closes: <bead-id>`
  when the commit completes the bead. Note `BREAKING CHANGE:` when applicable.

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

Commit each bead as you finish it. Do not batch a session's work into one commit
at the end, and never end a session — or go idle waiting on the user — with
uncommitted changes in the tree.

Uncommitted work is *unprotected* work. Sessions get cut off by usage limits
mid-task, and anything not committed is one `git reset --hard` away from being
gone, in a tree that other sessions are editing. This has already cost this
project a scare: roughly fifty files of finished work sat loose in the tree
after four sessions were interrupted at once.

If you are interrupted or must stop mid-bead, commit what you have rather than
leaving it loose. Mark it plainly and keep the bead open:

```
feat(storage): begin the operation queue drainer

Work in progress -- retry classification is not implemented yet.

Refs: postio-abc
```

Never commit secrets — no passwords, no tokens, no real email addresses in
fixtures. Two CI checks enforce this; run them before you commit.

## Design and scope

- **Approved plan:** `~/.claude/plans/ethereal-fluttering-kettle.md`
- **Product spec:** `spec.md`
- **Design canvas:** `Design/Mail Client.dc.html` — the chosen direction is
  **PLATE (option 1b)**: airy desktop, 40px rows, key hints on the focused row only.

Where `spec.md` and the design canvas disagree, **the canvas wins** (it is newer):

- Keys are `e` reply, `a` archive, `A` archive thread, `u` undo, `t` thread —
  *not* spec.md §8's `r` reply. All bindings are overridable via `[keys]`.
- Compose takes over the reading pane; it is not a separate window.
- The sidebar says "Flagged", not "Starred".

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

## Architecture

```
postio-gtk    GTK4 + libadwaita + WebKitGTK. Widgets, CSS, keymap, palette.
     |        Command down / Event up. No SQL, no IMAP.
postio-core   UI-agnostic runtime: command bus, registry, event stream,
     |        app state, undo stack, tokio<->glib bridge.
     +-- postio-sync     operation queue, QRESYNC resync, IDLE, backoff
     |     +-- postio-imap (io-imap)   postio-smtp (io-smtp)
     +-- postio-storage  SQLite, migrations, repositories, blob store
     +-- postio-search   FTS5 index, query-operator parser
     +-- postio-config   TOML schema, validation, watcher, live reload
postio-model  pure domain types + JWZ threading. No storage, no protocol.
```

## Working in parallel

**Assume other Claude sessions are editing this repository right now.** That is
the normal state of this project, not an exception. Several sessions work
different crates at the same time, in the *same* working tree, on the *same*
branch, sharing one git index and one cargo target directory.

### Tooling

Four project skills encode the routines below — use them rather than
reconstructing the commands:

| Skill | Use it when |
|---|---|
| `/lanes` | Starting up: who else is here, what is safe to claim |
| `/preflight` | Checking the real state of the tree, or when it looks broken |
| `/land` | A bead is done: gates, staging, message, `bd close` |
| `/add-fixture` | Adding `.eml` test mail to the corpus |
| `/ux-architect` | Designing any surface, flow, or interaction — hold the experience coherent |
| `/gtk-design` | Building it: tokens, GTK traps, motion, render-to-PNG |

A `PreToolUse` hook (`.claude/hooks/guard-shared-tree.py`) refuses the
destructive commands listed below rather than trusting anyone to have read this
far. Its own test suite is `.claude/hooks/test-guard-shared-tree.py`.

Check who is active before you start:

```bash
bd list --status=in_progress   # claimed by someone; leave it alone
bd ready                       # unblocked work (ignore [epic] rows)
bd show <id>
bd update <id> --claim         # claim BEFORE writing code
bd close <id> --suggest-next
```

### Never do these — they destroy other sessions' work

| Don't | Why | Do instead |
|---|---|---|
| `git add -A`, `git add .`, `git commit -a` | Commits other sessions' half-written files | `git add crates/<your-crate> Cargo.lock` |
| `git reset --hard`, `git checkout .` | **Irrecoverably deletes** uncommitted work across every crate | Revert only your own files, by path |
| `git stash` | Stashes *everyone's* changes, not just yours | Leave the tree alone; commit your own work |
| `git rebase`, `git filter-repo`, history rewrites | Others hold refs that become invalid | Only when the user confirms the tree is quiet |
| `cargo fmt --all` | Reformats crates being edited right now, creating phantom diffs | `cargo fmt -p <your-crate>` |
| Editing the workspace root `Cargo.toml` | Four sessions colliding on one manifest | `cargo add -p <your-crate> <dep>` |
| "Fixing" a failure in a crate you don't own | It is almost always someone's in-flight TDD | Note it in your bead and move on |

### Expected friction, not breakage

- **`cargo test --workspace` may fail in a crate you don't own.** That is another
  session mid-TDD. Verify your own work with `cargo test -p <your-crate>`, and
  only require the full workspace green for the crates you touched.
- **`Cargo.lock` churns constantly.** Expected — it is a resolved superset and
  last-writer-wins is fine. Stage it with your commit.
- **`index.lock` contention** means another session is mid-commit. Wait and retry.
- **Cargo serialises on the target directory**, so builds feel slower than usual.
  That is contention, not a problem to debug.

### Scope discipline

Take one epic, and stay inside its crates. The crate split exists partly so
sessions do not collide: `postio-model`, `postio-storage`, `postio-search`,
`postio-config`, `postio-imap`, `postio-smtp`, `postio-sync`, `postio-core` and
`postio-gtk` are deliberately disjoint. If your bead genuinely requires touching
a crate another session owns, say so in the bead notes rather than editing it.
