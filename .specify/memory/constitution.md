# Postio Constitution

Postio is a local-first, keyboard-first email client for people who have too
much email: **read less, find anything, act faster.** This document holds the
rules that outrank convenience. Product truth is `docs/PRODUCT.md`;
architecture is `docs/ARCHITECTURE.md` and `docs/decisions/`. Where this
document and another disagree, this one wins.

## Core Principles

### I. Local-First, and the UI Never Awaits the Network

- Every mutating action is: write to the store, enqueue the remote effect,
  emit the change, repaint.
- The UI never awaits a network round trip for a visible outcome. Navigation
  works with the network absent.
- Sync state is visible. Destructive operations are confirmed or undoable
  (the command registry machine-checks this).

### II. The Keyboard Is a System, Not a List of Shortcuts

- Every command has a shortcut, a command-palette entry and an accessible
  action, all derived from one table, `postio-core::registry`. The keymap,
  command bar, `?` map, row menu, keycaps and `docs/keybindings.md` are
  generated from it; a test fails when the doc drifts.
- A command not in the registry does not exist. A feature is complete when
  its command is registered.
- Every binding is overridable from `[keys]` in `config.toml`, keyed by command
  id. Command ids are a file format: do not rename them casually.
- The mouse stays excellent and is never required.
- The list has a cursor and a selection; never conflate them. The cursor is
  where the keyboard is, the selection is what `a` would archive.

### III. Search Is Navigation, and There Is One Query Language

- Search answers locally within the principle V budget.
- One query language everywhere: the search bar, a saved search and
  `config.toml` mean the same string the same way. A virtual folder is a
  pinned saved search; a rule is a saved search plus actions. Natural-language
  search lowers to this language; no second one is introduced.
- The parser never errors on a half-typed query (`is:`, `after:2026-`).
  Unrecognised text is free text. Results appear while typing.
- `header:` matches every indexed header, bounded by a per-message budget,
  never by an allowlist of names (an allowlist answers "no match" silently
  and bakes one provider's vocabulary into code). A header that must match
  before the body arrives earns its own operator.

### IV. Test-First (NON-NEGOTIABLE)

- Write the failing test and observe it fail before the code that passes it.
- Never re-break working code to prove a test. Tighten a test never seen red
  until it visibly constrains the behaviour.
- An issue is done when its acceptance criteria are covered by tests.
- Assert on what a person would see, not on what a layer was handed: defects
  live between layers that each pass.
- The default suite never touches the network; live-server tests are
  `#[ignore]`. Protocol code tests against the `MailBackend` mock and the
  `.eml` corpus in `crates/postio-model/tests/corpus/`.

### V. Performance Is a Functional Requirement

- Budgets: startup to usable UI < 500 ms, interaction < 16 ms, local search
  < 100 ms, transitions <= 100 ms or none; `prefers-reduced-motion` always
  honoured. `docs/PERFORMANCE.md` says how each is gated.
- Never load a mailbox into memory. Lists are windowed over the paged store;
  "select all" is a predicate, not a set of ids.
- Budgets are gated as counts (statements, rows, trigger firings via
  `postio_storage::test_support::counting`), not timings. A change to a read
  path carries count assertions. Wall-clock benches run nightly and report.

### VI. Privacy Is a Feature

- Nothing leaves this machine that the user did not ask for. The test for
  anything that could make a request is "did the user ask for it".
- Remote images are blocked until allowed per sender. Read receipts are never
  sent automatically. One-click unsubscribe fires only on deliberate
  activation. No link prefetch, favicon fetch or speculative connection.
- The reader's renderer cannot run script or reach the network (ADR 0042).
- No telemetry, crash reporting or update ping.
- Credentials live in the OS keyring, never in `config.toml` or a log. The
  local store is encrypted at rest.
- Logs carry ids, counts and outcomes, never message content.
- The repository is public. Fixtures, tests, issues, PRs and commits use
  reserved domains (RFC 2606, `.test`, `.invalid`, `.example`, `.localhost`)
  and no real person's name or address.

### VII. Boundaries Are Enforced, Not Advised

Boundaries are checks in `scripts/checks/` (`check-crate-boundaries.py`);
satisfy them rather than argue with them. The crate-level rules are in
`docs/ARCHITECTURE.md`. In short:

- Every interface (`postio-gtk`, `postio-tui`, `postio-ffi`, any that
  follows) holds no SQL and no protocol code: it takes no store engine and no
  mail protocol as a direct dependency and reaches mail through the engine
  crates and `postio-client`. One interface holds the store at a time
  (ADR 0041).
- `postio-widgets` takes no store engine, no protocol and no store-owning
  crate at any depth, and depends on no interface (ADR 0043).
- `postio-core` and `postio-session` take no GTK; `postio-search`,
  `postio-body` and `postio-calendar` are pure leaves; `postio-sync` talks to
  the `MailBackend` trait, never to `io-imap` types.
- No app links an inference engine, directly or transitively.
- **Providers are data, not code.** Server settings live in the preset table.
  No provider-named constants or branches.
- **Pimalaya first.** Survey the Pimalaya crates before writing wire code; a
  survey older than a few weeks is stale. Replacing hand-rolled protocol code
  with a Pimalaya crate is welcome.

## Additional Constraints

- **Scope.** One product, several interfaces on the same engine and store:
  GTK (`postio-gtk`, the Focus design), the terminal (`postio-tui`) and macOS
  (`macos/` over `postio-ffi`). v1 ships on Linux (GTK4, libadwaita, Wayland
  first, and the terminal) with IMAP, SMTP, OAuth and one provider preset
  table (ADR 0006). Work outside scope belongs on the roadmap.
- **No AI in Postio itself.** Postio embeds, bundles, downloads and starts no
  model or inference engine. A model the user runs on this machine and names
  in `config.toml` may be used: optional, off until named, local only, never
  required (a rule-based detector answers without it; spec 007, FR-165 to
  FR-170). Other AI features are deferred to epic E12.
- **No backwards compatibility.** There are no installs to protect: no shims
  or deprecation paths. Migrations are still written.
- **One fact, one home.** Cite a rule recorded elsewhere. Decisions go in
  ADRs; traps in `docs/gotchas.md` or `docs/notes/`.
- Logging is `POSTIO_LOG` (an `EnvFilter`), not `RUST_LOG`.
- A test that needs a display goes in `tests/`, never `src/`.

## Development Workflow and Quality Gates

- **The loop:** claim an issue, work it test-first in your own worktree, land
  it, take the next. An issue is available when it is open, labelled `ready`,
  unassigned and unblocked. `epic`, `icebox`, `needs-architecture` and
  `needs-maintainer` are never claimed.
- **Spec-driven work** under `specs/<nnn>-<name>/` (past `/speckit-specify`
  and `/speckit-plan`) is not split into one issue per task. `tasks.md` is
  the queue and `spec.md` the acceptance. It lands on `feature/<name>`, one
  commit per task, as one PR reviewed against the spec.
- **Tiers:** `scripts/test-fast.sh` between edits, the changed crates' units
  at landing, the full suite on CI. Iterate at the cheapest layer that can
  fail.
- **Landing** is `scripts/issue-land.sh --detach`. CI decides and is not
  routed around: a red check is that branch's work to fix. A failure in code
  the diff does not touch is probably not the branch's; reproduce it alone
  and search for prior art first.
- **Commits** are `<type>(<scope>): <summary>` with a body that says why,
  ending `Refs: #<issue>` or, for spec work, `Refs: specs/<nnn>-<name>` and
  the task id. No closing keyword, even negated; the PR body closes.
  Commit as you go.
- **Say it where it persists:** why in the commit body, findings on the issue,
  new work through `scripts/issue-file.sh`.
- **Parallel sessions are normal.** A worktree belongs to one session;
  `~/src/postio` is for coordination. No worktree path enters anything rustc
  or a build script sees.

## Governance

- This constitution supersedes conflicting practice. `CLAUDE.md` elaborates
  it and must not contradict it.
- **Amendment:** a PR that changes this file and states the version bump.
  Narrowing or removing a principle needs the maintainer's explicit approval.
  An amendment that leaves a principle unenforceable lands its check in the
  same PR or names who enforces it.
- **Versioning:** MAJOR for a removal or redefinition of a principle or of
  governance; MINOR for a new principle or materially expanded section; PATCH
  for wording.
- **Compliance:** every PR is reviewed against these principles;
  `scripts/check.sh` is the machine-checked subset. Complexity a principle
  does not obviously permit is justified in the commit body or ADR. A call
  only the maintainer can make is labelled `needs-maintainer`; one an agent
  can make, `needs-architecture`.

**Version**: 1.4.1 | **Ratified**: 2026-09-08 | **Last Amended**: 2026-11-22
