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


## Build & Test

```bash
cargo build --workspace
cargo test  --workspace          # must never touch the network
cargo test  --workspace -- --ignored   # live iCloud tests; needs POSTIO_TEST_* env
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo bench                      # perf budgets; see below
```

System dependencies (Fedora 40+; this box is Fedora 44 / GNOME 50 / Wayland):

```bash
sudo dnf install gtk4-devel libadwaita-devel webkitgtk6.0-devel \
                 sqlite-devel libsecret-devel glib2-devel pkgconf-pkg-config
```

Verified working against: gtk4 4.22.4, libadwaita-1 1.9.3, webkitgtk-6.0 2.52.5,
sqlite3 3.51.2, libsecret-1 0.21.7, glib-2.0 2.88.3.

## Development rules

### Test-driven development is mandatory

**Write the failing test first, then the implementation.** This is not a
preference — it is how this repository is built, and it applies to every crate.

- A bead is not done until its acceptance criteria are covered by tests.
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

**Every commit must be green** — `cargo build --workspace`, `cargo test
--workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo fmt --all --check`, and `python3 scripts/check-crate-boundaries.py`.
Do not commit a broken intermediate state; use `git stash` or keep working.
The one exception is the initial import, which reconstructs parallel work.

**Do not commit unless the user asked.** Never `git push` without being asked.
Never commit secrets — no passwords, no tokens, no real email addresses in
fixtures. `crates/postio-model/tests/corpus/` has a test that enforces this.

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

Several sessions may run at once. Claim before you start so others skip it:

```bash
bd ready                      # only genuinely unblocked work (ignore [epic] rows)
bd show <id>
bd update <id> --claim
bd close <id> --suggest-next
```

Take one epic per session where possible — the crates are deliberately disjoint.
Avoid editing the workspace root `Cargo.toml` and CI config concurrently; if you
must add a shared dependency, add it and say so in the bead notes.
