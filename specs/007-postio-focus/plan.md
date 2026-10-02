# Implementation Plan: Postio Focus

**Branch**: `feature/postio-focus` | **Date**: 2026-09-27 | **Spec**: [spec.md](./spec.md)

**Input**: Feature specification from `/specs/007-postio-focus/spec.md`

## Summary

Postio Focus is Postio's desktop app (ADR 0043): a GTK4/libadwaita frontend
on the engine that opens on a dense, keyboard-first inbox showing mail as it
arrived. It does four things to mail:

- calls out invitations, questions and to-dos;
- holds some mail back into digests on the user's cadence;
- archives spam and automated updates, each with its reason and a one-key
  restore;
- links mail to an Obsidian vault.

**The app is one crate, `postio-focus`.** Like the terminal, it runs
`postio-host` in its own process and reaches mail only through
`postio-client` (ADR 0041). **The GTK it draws outside its own window lives
in `postio-widgets`**, which depends on no app (R1, ADR 0043): the message
view on spec 006's renderer, the composer, the settings window, the keycap,
chip, action-bar, notice and toast widgets, the pickers, the list model and
the presenters.

**Classification is `postio-classify`.** It is layered, returns a fixed
schema, and has no send path (ADR 0009). **Invitations use a small adapter
over calcard,** chosen after surveying Pimalaya first (R9).

**The host has a Focus mode that only Focus switches on**, which makes the
spec's "only while Focus runs" literal (R8):

- a filing pass inside sync's transaction for new mail;
- a body-stage task, with a catch-up at start;
- a due timer for digests, reminders and RSVP windows.

**The engine has:**

- three headers promoted for new mail by ADR 0025's path;
- a list sort key, `sort_at`, so a woken snooze comes back at the top;
- tables for markers, filter decisions, digest holds and deliveries,
  reminders and correspondents;
- Focus list scopes, with counting assertions;
- a real `Recovery::Window` for RSVP.

**The registry is one keymap for every app,** `KEYS.md`'s (R4).

**Nothing lands until the maintainer says so** (Clarifications). The
milestones group the work:

1. Milestone 1: the inbox's screens, invitations, the built-in needs-action
   detector, filtering by headers with the Filtered view, and sender
   digests.
2. Milestone 2: the user's own optional model, for needs-action, digest
   summaries and list, search and like-this rules.
3. Milestone 3: Obsidian and `postio://`.

The classic three-pane app is retired in the order
[`classic-parity.md`](./classic-parity.md) sets out; its removal waits for
the maintainer.

## Technical Context

**Language/Version**: Rust, edition 2024, pinned by `rust-toolchain.toml`.

**Primary Dependencies**:

- gtk4 0.11 (`v4_20`) and libadwaita 0.9 (`v1_7`, which exposes `AdwDialog`,
  `AdwBanner`, `AdwToast` and the accent API);
- webkit6, for the composer's editor only, since ADR 0039's native editor is
  not built;
- `postio-render`, on Blitz with Postio's patch queue;
- the engine crates: `postio-host`, `postio-client`, `postio-core`,
  `postio-ui`, `postio-search`, `postio-body`, `postio-config`;
- `calcard` (pinned, default features off), inside `postio-calendar` (R9);
- `io-http`, for the loopback model endpoint (`postio-ai`, R16);
- the filesystem only, for the vault.

**Storage**: The encrypted Turso store has Focus's tables and columns
([data-model.md](./data-model.md)). A schema change comes with a migration
where one can be written, and a store no migration reaches starts over
(R0). The user's decisions are in `config.toml` under `[focus]`
([contracts/config.md](./contracts/config.md)).

**Testing**:

- `cargo nextest` throughout.
- Unit tests, at the fast tier, in `postio-classify`, `postio-calendar`,
  `postio-search` and `postio-ui`.
- Counting assertions (`postio_storage::test_support::counting`) on every new
  read path.
- ADR 0008's differential test between the digest matcher and the executor.
- Registry enumeration across frontends, and Focus's `registry_parity`.
- `focus_suite`: one binary on the custom harness, run on the headless
  compositor, asserting on the widget tree with keys and clicks delivered
  through GTK.
- A `shot` example that renders every screen, for comparison with its PNG.

**Target Platform**: Linux with GNOME (the GNOME 50 SDK, libadwaita 1.9),
Wayland first, installed from the desktop Flatpak.

**Project Type**: A desktop application frontend in a Cargo workspace. Focus
added six crates: `postio-widgets`, `postio-focus`, `postio-classify`,
`postio-calendar`, `postio-ai` and `postio-vault`.

**Performance Goals**: The constitution's budgets: startup under 500 ms,
interaction under 16 ms, search under 100 ms. Their causes are gated as
counts:

- a Focus inbox page takes at most 3 statements at the storage layer, with
  no scans;
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
- `postio-focus` never depends on `postio-gtk` or `postio-app`.
- Focus's rules act only while Focus runs.
- A command's key is the same in every app.
- No language model and no inference engine ships in any package.
- Nothing lands until the maintainer says so.

**Scale/Scope**:

- 25 drawn screens, plus the message dialog's, Settings and the reading
  pane, which the handoffs or `/ux-architect` designed.
- About 60 new or changed registry commands.
- The reference store holds about 100,000 messages.

## Constitution Check

| Principle | Gate | Status |
|---|---|---|
| I. Local-first; the UI never awaits the network | Writes land locally first; no visible outcome waits on a server | **Pass.** Every mutation is a `Command` through the host's store, enqueue, emit order. An RSVP is queued with a not-before time and never awaited. Classification runs in sync's transaction or on a background task, never on the UI path (FR-131). |
| II. The keyboard is a system | Every command has a key, a palette entry and an accessible action, all from the registry | **Pass by construction.** Every Focus action, picker keys included, is a registry command. The key map, the bar and the keycaps are derived. The one keymap is enumerated across frontends (SC-015), and every command Focus is offered reaches a handler. The key-map groups table holds ids, not bindings (R4). |
| III. One query language | Same query, same meaning, everywhere | **Pass.** Digest rules are lists of queries. Plain English lowers to chips of the language. The digest matcher is held equal to the executor by ADR 0008's differential test. The promoted headers each have an operator (`is:bulk`, `is:automated`), as ADR 0025 requires. |
| IV. Test-first | Every task's test seen red first | **Pass.** `focus_suite` asserts what a person sees. Screens are compared with their PNGs and every difference recorded. |
| V. Performance, gated as counts | New read paths carry statement and row assertions | **Pass.** `sort_at` on the hottest path left the list's counting tests unchanged (R7). The Focus scopes add a predicate at every membership site, each asserted (contracts/engine.md). The filing pass is bounded per message and counted in the sync suite. |
| VI. Privacy is a feature | Nothing leaves the machine unasked; no content in logs | **Pass.** An RSVP leaves only on a key press, after a cancel window. Promoted headers and calendar parts come from the user's own server, for new mail and with bodies. The model is loopback-only, off unless configured, never probed, and egress-logged. Obsidian is local files. `postio://` only navigates. Excerpts and reasons live only in the encrypted store. |
| VII. Boundaries are enforced | New crates have checked rules | **Pass.** `check-crate-boundaries.py` has rules for `postio-widgets`, `postio-focus`, `postio-classify`, `postio-calendar` and `postio-ai`. The checks that scan GTK code scan the new crates too (R1). Pimalaya was surveyed first; calcard was chosen with its reason recorded (R9). Automated senders and model runtimes are data. |
| Additional Constraints: Scope | Work outside scope belongs on the roadmap | **Amended in 1.3.0** (approved 2026-09-27): the Scope names Focus and allows its optional, user-supplied model. Its "two desktop apps" changes with the one-app decision (C27), an amendment that lands with the branch and needs the maintainer. |
| Additional Constraints: No backwards compatibility | Clean versions, no shims | **Pass.** Renamed ids (`toggle_read`, `go_to_folders`) have no aliases. A store no migration reaches starts over. |
| Additional Constraints: One fact, one home | ADRs for rules, specs for features | **Pass.** ADR 0043 is the `postio-widgets` rule and the one-app decision. ADR 0032 records Focus's one-message view, and ADR 0025 the three promoted headers. The spec carries the reasoning. |
| Development Workflow | Spec-driven work lands once on one feature branch | **Pass.** It lands once, when the maintainer says so. |

The contracts ([engine](./contracts/engine.md), [keymap](./contracts/keymap.md),
[config](./contracts/config.md), [surface](./contracts/focus-surface.md))
carry commands, scopes and reads the registry and host have the shape for.
The design adds no second query language, keymap, composer or reader.

## Project Structure

### Documentation (this feature)

```text
specs/007-postio-focus/
├── spec.md              # stories, requirements, decisions C1–C27
├── plan.md              # this file
├── research.md          # findings R0–R18
├── data-model.md        # store and config shapes
├── quickstart.md        # how to watch it hold
├── contracts/
│   ├── engine.md        # host, sync, classifier, commands, reads, boundaries
│   ├── keymap.md        # the one keymap
│   ├── config.md        # [focus] and [keys]
│   └── focus-surface.md # frames, sizes, anatomy, copy
├── checklists/
│   └── requirements.md
├── screens.md           # each PNG comparison, and the designs with no PNG
├── classic-parity.md    # what the classic app does, where Focus does it, and its retirement
├── macos.md             # a starting brief for Focus on the Mac
└── tasks.md             # the queue
```

### Source Code

```text
crates/
├── postio-widgets/            # the GTK Focus draws outside its window (R1, ADR 0043)
│   ├── src/body_view/         #   BodyView: tiles, find, zoom, selection, accessibility
│   ├── src/reader/            #   Reader, banner, notices, attachment chips, render-mode line
│   ├── src/composer/          #   the composer, behind ComposerHost
│   ├── src/widgets/           #   keyhint, keycap, action_bar, buttons, chip, notice,
│   │                          #   toast, pickers, date entry, label pill, recipient chips
│   ├── src/settings.rs        #   the settings window (ADR 0031)
│   ├── src/{state,drag_out,editor,onboarding,capture}.rs
│   ├── src/list_model.rs      #   the GListModel over postio-ui's ListWindow
│   ├── src/present/           #   compose, reading, config, onboarding, export,
│   │                          #   and settings (accounts, credentials, privacy)
│   └── tests/widgets_suite/
├── postio-focus/              # the app (R3)
│   ├── src/{main,app,startup,window,chrome,keys}.rs
│   ├── src/list/              #   model, rows, the surfaced-row splice, the reading pane
│   ├── src/{open,open_header,source,chooser}.rs   # the open message
│   ├── src/compose/           #   the composer's host and frame
│   ├── src/{bar,places,label_picker,move_picker,row_menu,bulk}.rs
│   ├── src/{digest,rule_dialog,rules,filtered,capture,settings,keymap_dialog}.rs
│   ├── data/                  #   the desktop entry, focus.css, focus-colours.css
│   ├── examples/shot.rs       #   every screen from a seeded store
│   └── tests/focus_suite/     #   one binary, custom harness
├── postio-classify/           # guards, corrections, rules, detector (R8, R10)
├── postio-calendar/           # calcard adapter + RFC 5546 layer (R9)
├── postio-ai/                 # the loopback model client (R16)
├── postio-vault/              # Obsidian Tasks lines and notes (R16)
├── postio-host/               # enable_focus; Focus reads
├── postio-client/             # ClientKind::Focus; focus_counts, surfaced, marker, …
├── postio-core/               # one keymap; Availability.frontend; Requirement::Focus;
│                              #   contexts Picker/Digest/Filtered/Capture
├── postio-ui/                 # finder's blend; key-map groups; presets; focus_dialog,
│                              #   focus_row, focus_state, dwell; label colours
├── postio-search/             # natural::lower; date::parse_when; matcher
├── postio-body/               # own text; quote folds; treatment
├── postio-model/              # ListScope::Focus; ThreadSummary.marker; calendar part;
│                              #   Draft.labels and remind_at
├── postio-storage/            # Focus tables; migrations; counting tests
├── postio-runtime/            # Focus scopes; the due timer
├── postio-sync/               # FocusFiling; calendar parts in backfill; correspondents
├── postio-account/            # promoted header fields on incremental IMAP fetches
├── postio-gmail/              # the same fields from its metadata
├── postio-config/             # [focus]; ConfigChanged.focus
└── postio-tui/                # ctrl+z is undo
flatpak/dev.postio.Postio.json          # builds postio-focus and its desktop file
scripts/checks/                         # boundary rules for the new crates
docs/decisions/0043-…                   # the one desktop app, and postio-widgets
.specify/memory/constitution.md         # Scope 1.3.0
```

**Structure decision**:

- **Two crates carry the app.** `postio-widgets` holds the GTK that can be
  tested without Focus's window and keeps the presenters on
  `postio-client`. `postio-focus` holds Focus's window, rows and dialogs, on
  the terminal's one-crate pattern (R3).
- **Leaves carry the new engine logic.** `postio-classify` is where "cannot
  send" is checked, `postio-calendar` keeps calcard out of `postio-model`,
  which every crate waits on, and `postio-ai` and `postio-vault` keep the
  model client and the filesystem writer behind their own boundaries.
- **Everything else is an addition to a crate that already owns the
  concern.**

The order of work is [tasks.md](./tasks.md).

## Defaults the maintainer may override

Each is recorded in [research.md](./research.md), and each is a small change
if overridden.

- **A woken snooze comes back at the top, in every app,** through `sort_at`
  (R7).
- **Invitations are parsed by calcard behind an adapter,** not Pimalaya's
  `ical-rs`, until `ical-rs` resolves zones without a `VTIMEZONE` (R9).
- **Three headers are promoted for new mail** (`List-Unsubscribe`,
  `Precedence`, `Auto-Submitted`), with `is:bulk` and `is:automated` (R8).
- **Filtering applies to mail that arrives after it is on.** The existing
  inbox is filtered only by a deliberate sweep (FR-118).
- **The built-in detector is rules,** with no table of weights (R10).
- **Completion ranks by "wrote N times"** (R15).

## Complexity Tracking

| Cost accepted | Why | What was rejected |
|---|---|---|
| Six new crates | FR-006/007: shared GTK that depends on no app; ADR 0009's "cannot send" as a checked boundary; calcard kept out of `postio-model`; the model client and the vault writer behind their own boundaries | Classification inside the host (no boundary to check); the calendar in `postio-model` (every crate waits on it) |
| Code moved out of `postio-gtk` and `postio-app` | One implementation of the reader, the composer, settings and their presenters | Copying them; making `postio-gtk` the shared crate (a move of every classic file) |
| One keymap changes the other apps' defaults | The maintainer's decision (Clarifications) | A separate Focus profile |
| `sort_at` on the hottest list path | Snooze keeps one meaning in every app, and comes back at the top as screen 11 says | In-place return with changed copy |
| Headers promoted for new mail | Filtering must decide before the body arrives, or mail is seen and then moved | Deciding after the body arrives; a header allowlist (rejected by ADR 0025) |
| Focus scopes touch every membership site | Totals, seek marks and rows must agree on held mail and surfaced reminders | A SQL `UNION` of surfaced rows; reusing `snoozed_until` for holds, which would hide held mail from the other apps and conflate snoozes |
| calcard instead of a Pimalaya crate | Zone resolution and maturity (R9) | `ical-rs` today |
| A constitution Scope amendment | A desktop app the Scope did not name, and an optional local model | Building it without saying so; this branch does not land until the amendment does |
