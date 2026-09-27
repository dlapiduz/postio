# Implementation Plan: Postio Focus

**Branch**: `feature/postio-focus` | **Date**: 2026-09-27 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/007-postio-focus/spec.md`

## Summary

Postio Focus is a third frontend on the engine: a GTK4/libadwaita app with its
own binary. It opens on a dense, keyboard-first inbox that shows mail as it
arrived. It does four things to mail:

- calls out invitations, questions and to-dos;
- holds some mail back into digests on the user's cadence;
- archives spam and automated updates, each with its reason and a one-key
  restore;
- later, links mail to an Obsidian vault.

**The app is one crate, `postio-focus`.** Like the terminal, it runs
`postio-host` in its own process and reaches mail only through
`postio-client` (ADR 0041). **What it needs from the classic app moves into a
new shared GTK crate, `postio-widgets`,** so Focus never depends on
`postio-gtk` (R1). That covers:

- the message view, built on spec 006's renderer, which has landed;
- the composer;
- the keycap, chip, action-bar, notice and toast widgets;
- the list model;
- the presenters.

**Classification is a new crate, `postio-classify`.** It is layered, returns a
fixed schema, and has no send path (ADR 0009). **Invitations use a small
adapter over calcard,** chosen after surveying Pimalaya first (R9).

**The host gains a Focus mode that only Focus switches on.** That makes the
spec's "only while Focus runs" literal. It adds three things (R8):

- a filing pass inside sync's transaction for new mail;
- a body-stage task, with a catch-up at start;
- a due timer for digests, reminders and RSVP windows.

**The engine gains:**

- three headers promoted from ADR 0025's allowlist, for new mail;
- a list sort key, `sort_at`, so a woken snooze comes back at the top;
- tables for markers, filter decisions, digest holds and deliveries,
  reminders and correspondents;
- Focus list scopes, with counting assertions;
- a real `Recovery::Window` for RSVP.

**The registry becomes one keymap for every app,** `KEYS.md`'s (R4).

**Nothing lands until the maintainer says so** (Clarifications). The
milestones are only the order of work:

1. Milestone 1: screens 01–20, invitations, the built-in needs-action detector,
   filtering by headers with the Filtered view, and sender digests.
2. Milestone 2: the user's own optional model, for needs-action, digest
   summaries and list, search and like-this rules.
3. Milestone 3: Obsidian and `postio://`.

## Technical Context

**Language/Version**: Rust 1.98.0, edition 2024 (pinned by `rust-toolchain.toml`).

**Primary Dependencies**:

- **Existing and reused:**
  - gtk4 0.11.4 (`v4_20`) and libadwaita 0.9.2 (`v1_7`, which already exposes
    `AdwDialog`, `AdwBanner`, `AdwToast` and the accent API);
  - webkit6 0.6.1, for the composer's editor only, since ADR 0039's native
    editor is not built;
  - `postio-render`, built on Blitz 0.3.0-beta.2;
  - the engine crates: `postio-host`, `postio-client`, `postio-core`,
    `postio-ui`, `postio-search`, `postio-body`, `postio-config`.
- **New in milestone 1:** `calcard` 0.3.x with default features off, inside
  `postio-calendar` (R9). No other new external crate.
- **Milestone 2:** an HTTP client to a loopback endpoint, drawn from what the
  workspace already carries. Pimalaya's `io-http` is surveyed first.
- **Milestone 3:** the filesystem only.

**Storage**: The encrypted Turso store gains tables and columns
([data-model.md](./data-model.md)). The schema's fingerprint moves, and an
existing store resyncs; there are no migrations (R0). The user's decisions go
in `config.toml` under `[focus]` ([contracts/config.md](./contracts/config.md)).

**Testing**:

- `cargo nextest` throughout.
- Unit tests, at the fast tier, in `postio-classify`, `postio-calendar`,
  `postio-search` and `postio-ui`.
- Counting assertions (`postio_storage::test_support::counting`) on every new
  read path.
- ADR 0008's differential test between the digest matcher and the executor.
- Registry enumeration across frontends.
- `focus_suite`: one binary on the `app_suite` custom harness, run on the
  headless compositor, asserting on the widget tree.
- A `shot` example that renders every screen, 01 to 20, for comparison with
  its PNG.

**Target Platform**: Linux with GNOME (the GNOME 50 SDK, libadwaita 1.9),
Wayland first, installed from the desktop Flatpak as a second launcher.

**Project Type**: A desktop application frontend in a Cargo workspace. The
workspace has 25 crates, and becomes 29 in milestone 1: `postio-widgets`,
`postio-focus`, `postio-classify` and `postio-calendar`. Milestones 2 and 3
add `postio-ai` and `postio-vault`.

**Performance Goals**: The constitution's budgets: startup under 500 ms,
interaction under 16 ms, search under 100 ms. Their causes are gated as
counts:

- a Focus inbox page takes at most 3 statements, with no scans;
- `focus_counts` takes at most 5 statements;
- the filing pass takes at most 4 statements per new message, plus its writes;
- one message view serves every open;
- SC-011's first pass (under 5 minutes over 100,000 messages, on one core) is
  measured nightly.

**Constraints**:

- The UI never awaits the network.
- A mailbox is never loaded into memory.
- Drawing a row reads no body.
- Logs carry ids, counts and outcomes only.
- `postio-focus` never depends on `postio-gtk`.
- Focus's rules act only while Focus runs.
- A command's key is the same in every app.
- No language model and no inference engine ships in any package.
- Nothing lands until the maintainer says so.

**Scale/Scope**:

- 20 screens in milestone 1, and 5 later.
- About 60 new or changed registry commands: 40 new, including the picker
  and Filtered-tab keys, and 20 moved to new keys.
- About 7,000 lines move: `body_view`, the reader, seven widgets, the
  composer, and presenters taken from `postio-app`.
- The reference store holds about 100,000 messages.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | Gate | Status |
|---|---|---|
| I. Local-first; the UI never awaits the network | Writes land locally first; no visible outcome waits on a server | **Pass.** Every new mutation is a `Command` through the host's store, enqueue, emit order. An RSVP is queued with a not-before time and never awaited. Classification runs in sync's transaction or on a background task, never on the UI path (FR-131). |
| II. The keyboard is a system | Every command has a key, a palette entry and an accessible action, all from the registry | **Pass by construction.** Every Focus action, picker keys included, is a registry command. The key map, the bar and the keycaps are derived. The one keymap is enumerated across frontends (SC-015). The key-map groups table holds ids, not bindings (R4). |
| III. One query language | Same query, same meaning, everywhere | **Pass.** Digest rules are lists of queries. Plain English lowers to chips of the language. The digest matcher is held equal to the executor by ADR 0008's differential test. The promoted headers each gain an operator (`is:bulk`, `is:automated`), as ADR 0025 requires. |
| IV. Test-first | Every task's test seen red first | **Pass.** Tasks are written red-first. `focus_suite` asserts what a person sees. Screens are compared with their PNGs and every difference recorded. |
| V. Performance, gated as counts | New read paths carry statement and row assertions | **At risk → action.** `sort_at` touches the hottest path, so spike S6 measures it before anything depends on it, and a cheaper alternative is on offer (R7). The Focus scopes add a predicate at every membership site, each asserted (contracts/engine.md). The filing pass is bounded per message and counted in the sync suite. |
| VI. Privacy is a feature | Nothing leaves the machine unasked; no content in logs | **Pass, with new surfaces handled.** An RSVP leaves only on a key press, after a cancel window. Promoted headers and calendar parts come from the user's own server, for new mail and with bodies. The model (milestone 2) is loopback-only, off unless configured, never probed, and egress-logged. Obsidian (milestone 3) is local files. `postio://` only navigates. Excerpts and reasons live only in the encrypted store. |
| VII. Boundaries are enforced | New crates have checked rules | **Action required.** New rules for `postio-widgets`, `postio-focus`, `postio-classify` and `postio-calendar`, and `postio-ai` later. `postio-gtk` gains "not postio-focus". Eight checks that scan only `crates/postio-gtk` widen (R1). Pimalaya was surveyed first; calcard was chosen with its reason recorded (R9). Automated senders and model runtimes are data. |
| Additional Constraints: Scope | Work outside scope belongs on the roadmap | **Amendment required, maintainer-directed, at landing.** The Scope must name Focus, as version 1.2.0 named the terminal. Milestone 2's optional model sits outside "no AI (deferred to E12)". The maintainer chose to build it on this unlanded branch (Clarifications), so the amendment lands with the branch and needs the maintainer's approval. |
| Additional Constraints: No backwards compatibility | Clean versions, no shims | **Pass.** Schema changes resync. Renamed ids (`toggle_read`, `go_to_folders`) have no aliases. |
| Additional Constraints: One fact, one home | ADRs for rules, specs for features | **Pass.** ADR 0043 is kept to the `postio-widgets` rule. ADR 0032 gains a note on Focus's one-message dialog, and ADR 0025 gains the three promoted headers. The spec carries the reasoning. |
| Development Workflow | Spec-driven work lands once on one feature branch | **Pass.** It lands once, when the maintainer says so. |

**No unjustified violations.** The Scope amendment, the new crates and the
hot-path change are under Complexity Tracking.

**Re-check after Phase 1**: Unchanged. The design adds no second query
language, keymap, composer or reader. The contracts
([engine](./contracts/engine.md), [keymap](./contracts/keymap.md),
[config](./contracts/config.md), [surface](./contracts/focus-surface.md))
carry commands, scopes and reads the registry and host already have the shape
for.

## Project Structure

### Documentation (this feature)

```text
specs/007-postio-focus/
├── spec.md              # Written, clarified (2026-09-26), corrected (2026-09-27)
├── plan.md              # This file
├── research.md          # Phase 0: R0–R18
├── data-model.md        # Phase 1
├── quickstart.md        # Phase 1
├── contracts/
│   ├── engine.md        # host, sync, classifier, commands, reads, boundaries
│   ├── keymap.md        # the one keymap
│   ├── config.md        # [focus] and [keys]
│   └── focus-surface.md # screens 01–25: frames, sizes, anatomy, copy
├── checklists/
│   └── requirements.md  # Written, passing
├── screens.md           # During implementation: each PNG comparison and its differences
└── tasks.md             # /speckit-tasks
```

### Source Code

```text
crates/
├── postio-widgets/            # NEW  shared GTK for both desktop apps (R1, ADR 0043)
│   ├── src/body_view/         #   ← postio-gtk/src/body_view/
│   ├── src/reader/            #   ← Reader, message_header, banner, notices, parts::Chips
│   ├── src/composer/          #   ← composer.rs, editor.rs, data/editor.js; ComposerHost
│   ├── src/widgets/           #   ← keyhint, keycap, action_bar, button, chip, notice, toast
│   │                          #   + pickers, date entry, label pill, recipient chips (new)
│   ├── src/list_model.rs      #   ← postio-gtk/src/list.rs MessageList, generalised
│   ├── src/present/           #   ← postio-app compose/reading presenters; config service;
│   │                          #     credential and add-account dialogs
│   ├── src/style.rs, data/    #   widgets.css; metrics tokens; resources
│   └── tests/widgets_suite/
├── postio-focus/              # NEW  the Focus app (R3)
│   ├── src/{main,app,startup,window,config}.rs
│   ├── src/list/              #   model, one-line and two-line rows, surfaced-row splice
│   ├── src/{open,compose,bar,folders,pickers,filtered,keymap_sheet,states}.rs
│   ├── src/digest/            #   window, rule dialog (24), rules list (g d)
│   ├── data/                  #   dev.postio.Postio.Focus.desktop, focus-colours.css
│   ├── examples/shot.rs       #   screens 01–20 from a seeded store
│   └── tests/focus_suite/     #   one binary, custom harness
├── postio-classify/           # NEW  guards, corrections, rules, detector (R8, R10)
├── postio-calendar/           # NEW  calcard adapter + RFC 5546 layer (R9)
├── postio-gtk/                # − moved code; + depends on postio-widgets
├── postio-app/                # − presenters; wires postio-widgets'
├── postio-host/               # + enable_focus; Focus reads
├── postio-client/             # + ClientKind::Focus; focus_counts, surfaced, marker, …
├── postio-core/               # one keymap; Availability.frontend; Requirement::Focus;
│                              #   contexts Picker/Digest/Filtered; new commands
├── postio-ui/                 # finder's blended mode; key-map groups; presets;
│                              #   mark_html; splice positions; label colours
├── postio-search/             # natural::lower; date::parse_when; matcher (+ differential test)
├── postio-body/               # own-text extraction; quote folds with ids and counts
├── postio-model/              # ListScope::Focus; ThreadSummary.marker; calendar part;
│                              #   Draft.labels and remind_at
├── postio-storage/            # schema additions; repositories; counting tests
├── postio-runtime/            # Focus scopes; ticker: deliveries, reminders, windows
├── postio-sync/               # FilingPass; calendar parts in backfill; correspondents
├── postio-account/            # promoted header fields on incremental IMAP fetches
├── postio-jmap/, postio-gmail/# the same fields
├── postio-config/             # [focus]; ConfigChanged.focus
└── postio-tui/                # Ctrl+Z is undo; spec 005's suspend claim corrected
flatpak/dev.postio.Postio.json          # + postio-focus and its desktop file
.github/workflows/release.yml           # the flatpak job builds both apps
scripts/checks/                         # four new boundary rules; eight widened scopes
docs/decisions/0043-…                   # NEW  the postio-widgets rule
docs/decisions/0032-…, 0025-…           # notes: Focus's dialog; three promoted headers
docs/PRODUCT.md, docs/ARCHITECTURE.md, docs/keybindings.md
.specify/memory/constitution.md         # Scope amendment, drafted for the maintainer's approval
```

**Structure Decision**:

- **Two crates carry the app.** `postio-widgets` holds what both desktop apps
  draw. `postio-focus` holds what only Focus draws, on the terminal's
  one-crate pattern. A classic-style split is not needed, because Focus reads
  through the client (R3).
- **Two leaves carry the new engine logic.** `postio-classify` is where
  "cannot send" is checked, and `postio-calendar` keeps calcard out of
  `postio-model`, which every crate waits on.
- **Everything else is an addition to a crate that already owns the concern.**

### Order of work (for `/speckit-tasks`)

The branch is rebased onto `main` as it goes. Every commit keeps the classic
app, the terminal and macOS green. Nothing lands until the maintainer says so.

0. **Spikes** (research R18). Each settles a risk with a number, recorded in
   `research.md`:
   - S1: calcard on invitation fixtures, its graph and its licence.
   - S2: promoted-header bytes per new message.
   - S3: a two-height list with spliced rows at 100,000 conversations.
   - S4: detector precision.
   - S5: highlighting by excerpt.
   - S6: `sort_at` against the list's counting tests.
1. **The shared crate, with no change in behaviour.**
   - Create `postio-widgets`, write ADR 0043, add the boundary rules and
     widen the checks.
   - Move, in order: `body_view`, the widgets, the reader, the list model,
     the presenters, and last the composer behind `ComposerHost`.
   - Fix the three bugs the move exposes, each red first (R1).
   - The classic suites pass after every step.
   - The moves are pure `git mv` commits, because `feature/contacts` edits the
     same files (R0).
2. **One keymap.**
   - The registry: `Availability.frontend`, `Requirement::Focus`, the new
     contexts, new and renamed ids, and every remap in
     [keymap.md](./contracts/keymap.md).
   - With it: the tests, the golden file and `docs/keybindings.md`; the
     terminal's `Ctrl+Z` and spec 005's correction; the key-map groups table.
   - This comes before Focus's interface, so Focus is built on its final keys.
3. **The Focus shell** (US1, US6, US7, US11).
   - The crate, and startup through the host, including the store-in-use
     screen.
   - The window chrome.
   - The list over a Focus inbox scope, which equals the unified inbox until
     phase 11, with its two row heights, the selection and the bulk bar.
   - Toasts and undo, banners and states, the key map and `shot`.
   - Compare screens 01, 02 and 15–20.
4. **Open** (US2). The dialog on the shared `Reader`, the highlight API, quote
   folds with counts, and `view_source`. Compare screen 04.
5. **Compose** (US3). `DialogHost`, recipient chips, the draft's labels and
   `remind_at`, correspondents, and completion ranking. Compare screens 05
   and 06.
6. **The bar and places** (US4). The blended finder, `natural::lower`,
   `parse_when`, saved searches on `Alt+1`–`Alt+4`, and the folders popover.
   Compare screens 07–10.
7. **Pickers, snooze and reminders** (US5). Presets, the four pickers,
   `Snooze{until}`, `sort_at` (or the cheaper alternative, per S6), and
   reminders with their surfaced rows. Compare screens 11–14.
8. **The classification engine.** `enable_focus`, `FilingPass`, the body task
   and its catch-up, the promoted headers (IMAP, JMAP and Gmail, with their
   columns and operators), the guards and `correspondents`, and
   `focus_classified`, all with counting.
9. **Invitations** (US8). `postio-calendar`, calendar parts in the backfill,
   markers, and RSVP (the outgoing calendar part, `queue_send_at`, and the
   window's undo).
10. **The built-in needs-action detector** (US12, milestone 1's part).
    Own-text extraction, the detector, the labelled corpus and its precision
    gate, and dismissals.
11. **Filtering** (US9). The senders table, reasons, the Filtered view with
    its tabs and `R`, the sweep, and the counts. Compare screen 21.
12. **Digests by sender** (US10). Rules in config, the matcher and its
    differential test, holds, deliveries, surfaced digest rows, the digest
    window, the rule dialog (24), the rules list after `/ux-architect`, and
    the empty state's next digest. Compare screen 24.
13. **Closing milestone 1.**
    - `screens.md` is complete.
    - `PRODUCT.md`, `ARCHITECTURE.md` and the ADR notes are updated.
    - Packaging: the second launcher and the release job.
    - The constitution amendment is drafted for the maintainer.
    - The branch does not land.
14. **Milestone 2.** `postio-ai` (with the `model` egress subsystem), the
    model answering needs-action, digest summaries (22, 23), and list, search
    and like-this rules.
15. **Milestone 3.** `postio-vault`, the capture sheet (25), and `postio://`.

## Decisions taken with a default: the maintainer may override

Each is recorded in [research.md](./research.md), and each is a small change if
overridden.

- **A woken snooze comes back at the top, in every app,** through `sort_at`.
  The cheaper alternative keeps today's in-place return and changes screen
  11's copy (R7).
- **Focus ships as a second launcher in the desktop Flatpak** (R3), with app id
  `dev.postio.Postio.Focus`.
- **Invitations are parsed by calcard behind an adapter,** not Pimalaya's
  `ical-rs`, until `ical-rs` resolves zones and settles (R9).
- **Three headers are promoted for new mail** (`List-Unsubscribe`,
  `Precedence`, `Auto-Submitted`), with `is:bulk` and `is:automated` (R8).
- **Filtering applies to mail that arrives after it is on.** The existing
  inbox is filtered only by a deliberate sweep (spec FR-118).
- **The built-in detector is rules.** A small table of weights is added only if
  spike S4 says rules miss the precision bar (R10).
- **Recipient chips are opt-in for the classic composer.** Completion ranks by
  "wrote N times" in both apps (R15).
- **The one keymap's new homes for classic-only commands** include Flag on
  `*`, Darken on `alt+d`, Flagged on `g *`, and result order on `O`
  ([keymap.md](./contracts/keymap.md)).

## Found while planning

Each was checked in code:

- **The detached composer stays light in dark mode:** its window never calls
  `style::track`. Fixed in phase 1.
- **`BodyView` carries a message's selection, folds and darkening into the
  next one.** Fixed in phase 1, because Focus's dialog reuses one view.
- **A render that times out shows no plain-text fallback.** Fixed in phase 1.
- **Spec 005 says the terminal suspends on `Ctrl+Z`, and no code does.**
  Corrected in phase 2.
- **`CLAUDE.md` and the constitution say migrations are still written, and the
  store has none** (`schema.rs:3-15`). For the maintainer; not changed here.

## Complexity Tracking

| Cost accepted | Why | What was rejected |
|---|---|---|
| Four new crates in milestone 1 | FR-006/007: shared GTK without Focus depending on `postio-gtk`; ADR 0009's "cannot send" as a checked boundary; calcard kept out of `postio-model` | Classification inside the host (no boundary to check); the calendar in `postio-model` (every crate waits on it) |
| About 7,000 lines move out of `postio-gtk` and `postio-app` | One implementation of the reader, the composer and their presenters | Copying them; making `postio-gtk` the shared crate (a move of every classic file) |
| One keymap changes the classic and terminal defaults | The maintainer's decision (Clarifications) | A separate Focus profile |
| `sort_at` on the hottest list path | Snooze keeps one meaning in every app, and comes back at the top as screen 11 says | In-place return with changed copy, which is offered |
| Headers promoted for new mail | Filtering must decide before the body arrives, or mail is seen and then moved | Deciding after the body arrives; a header allowlist (rejected by ADR 0025) |
| Focus scopes touch every membership site | Totals, seek marks and rows must agree on held mail and surfaced reminders | A SQL `UNION` of surfaced rows; reusing `snoozed_until` for holds, which would hide held mail from the classic app and conflate snoozes |
| calcard instead of a Pimalaya crate | Zone resolution and maturity (R9) | `ical-rs` today |
| A constitution Scope amendment at landing | A maintainer-directed third frontend, and an optional local model | Building it without saying so; this branch does not land until the amendment does |
