---

description: "Task list for Postio Focus"
---

# Tasks: Postio Focus

**Input**: Design documents from `/specs/007-postio-focus/`

**Prerequisites**: spec.md (clarified 2026-09-26), plan.md, research.md,
data-model.md, contracts/ and quickstart.md, all written and committed

**Tests**: Test-first is constitution IV and NON-NEGOTIABLE.

- Every task below that changes behaviour names its test *first*, and the code
  second.
- The test is **observed red** before the code is written. A task whose test
  was never seen red has not been done.
- Focus's integration tests (`crates/postio-gtk/tests/focus_suite/`) assert
  on the widget tree, which is what a person would see, never on what a widget
  was handed.
- Read paths assert statements and rows through
  `postio_storage::test_support::counting`.

**Organization**: By user story, in the spec's priority order. Milestone 1 is
Phases 1–13 and 16. Milestones 2 and 3 are Phases 17 and 18. **Nothing lands
on `main` until the maintainer says so** (spec, Clarifications). The branch is
rebased onto `main` as it goes. Nobody runs `issue-land.sh` for it until the
maintainer asks.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: touches files nothing else in its phase touches, so it can run beside
  its siblings.
- **[US#]**: the user story it serves. Setup, foundational and polish tasks
  carry none.

## Path conventions

- **Paths** are workspace-relative, from `~/src/postio-worktrees/postio-gtk`.
- **Commits** end with `Refs: specs/007-postio-focus` and the task id, never
  `Refs: #<issue>`.
- **The root `Cargo.toml`** is edited in this worktree, never in the main
  checkout.

## Six rules that hold for every task

1. **Nothing is taken from the other apps (FR-005).** The classic app's, the
   terminal's and macOS's suites pass with no edit beyond import paths and the
   one keymap's keys. A task that has to weaken one of those tests has found a
   regression, and stops.
2. **Focus reaches mail only through `postio-client`,** and never depends on
   `postio-gtk` or `postio-app`. The boundary check proves it.
3. **Drawing a row reads no body (FR-020).** Every read path added or changed
   carries counting assertions.
4. **A phase that builds a screen is not done until the screen has been
   compared with its PNG** (`shot`, then read the image back). The comparison
   is recorded in `specs/007-postio-focus/screens.md`, with every difference
   and its reason (FR-095).
5. **Fixtures use reserved domains and fictional people** (FR-152), and are
   added through `/add-fixture`. The design PNGs are never committed.
6. **Moves are pure `git mv` commits,** followed by a separate wiring commit.
   `feature/contacts` edits the same files (research R0).

---

## Phase 1: Setup

- [X] T001 Create the four crates with empty `lib.rs` (and `main.rs` for the app), and add each to the workspace's `members` and `default-members` (#1500):
  - `crates/postio-widgets/`
  - `crates/postio-gtk/` (bin `postio-gtk`)
  - `crates/postio-classify/`
  - `crates/postio-calendar/`
- [X] T002 Add the boundary rules of contracts/engine.md to `scripts/checks/check-crate-boundaries.py`, with their reasons: postio-widgets, postio-gtk, postio-classify and postio-calendar, and postio-gtk's "not postio-gtk". Add a workspace-wide ban on inference engines to `scripts/checks/check-dependency-policy.py`: candle, ort, tch, tract, burn, and llama.cpp bindings (FR-165). Test first: a fixture graph where `postio-gtk` depends on `postio-gtk`, one where `postio-classify` depends on `postio-smtp`, and one where any crate depends on `candle-core`, each fails its check
- [X] T003 [P] Widen the eight checks that scan only `crates/postio-gtk` so they also scan `crates/postio-widgets` and `crates/postio-gtk` (research R1):
  - `check-key-hints-are-derived.py`
  - `check-buttons-have-a-kind.py`
  - `check-no-dead-css.py`
  - `check-shadows-use-tokens.py`
  - `check-spacing-literals-ratchet.py`
  - `check-reader-header-has-one-home.py`
  - `check-blocking-now-sites.py`
  - `check-uncalled-pub-fn.py`'s `FRONTENDS`

  Test first: a literal key hint planted in `crates/postio-widgets/src` fails `check-key-hints-are-derived.py`
- [X] T004 [P] Write `docs/decisions/0043-focus-is-the-one-desktop-app.md`, kept to the rule: what may live there, what may not, and who depends on it. List it in `docs/decisions/README.md`
- [X] T005 [P] Create `crates/postio-gtk/tests/focus_suite/main.rs` on the `app_suite` custom harness (`CASES`, `IGNORED`, the `--list` contract of `list_contract.rs`) on the headless compositor, and `crates/postio-widgets/tests/widgets_suite/main.rs` the same way. Test first: an empty case is listed and runs. `widgets_suite` is done. `focus_suite` comes with T038, when there is a Focus to run
- [X] T006 [P] Create `specs/007-postio-focus/screens.md`: one row per screen from 01 to 20, with the columns "compared on", "differences" and "reason". Pre-fill the known differences C1–C23 from the spec

---

## Phase 2: Foundational (blocks every user story)

### Spikes first: each writes its answer into research.md (R18)

- [X] T007 **S1.** Add invitation fixtures through `/add-fixture`:
  - an Outlook request with a Windows TZID and its VTIMEZONE;
  - a Google request with an IANA TZID;
  - an Apple request, and a Zoom request;
  - an update (SEQUENCE+1), and a cancellation;
  - a weekly event with an EXDATE;
  - a TZID with no VTIMEZONE.

  In `crates/postio-calendar/tests/`, parse each with calcard (default features off) and assert the start, end and zone. Record per fixture in research R9, together with `cargo tree -d` and the verdict of `check-dependency-policy.py`. **If calcard fails the zones, T107 wraps ical-rs instead, behind the same adapter**
- [ ] T008 [P] **S2.** Against a real account, measured locally and never committed, record the bytes per message that `HEADER.FIELDS (LIST-UNSUBSCRIBE PRECEDENCE AUTO-SUBMITTED)` adds, in research R8
- [X] T009 [P] **S3.** In a throwaway `crates/postio-gtk/examples/list_spike.rs` (deleted after), put 100,000 synthetic rows of two fixed heights, plus 50 spliced rows, in a `gtk::ListView`. Measure rows built per frame while scrolling and jumping, and record them in research R3
- [X] T010 [P] **S4.** Build the labelled needs-action dataset as one data file, `crates/postio-classify/tests/data/needs_action.toml`: at least 150 items, each a message's own text with its addressing (direct, copied, list or automated) and a label (question, to-do or none, with any due date). Cover pleasantries, rhetorical questions, quoted history, signatures, list mail, copied-only mail, and instruction-shaped text. Sentence-level labels are what the detector reads, and a data file keeps the shared `.eml` corpus from swelling. Reserved domains and fictional names only. Measure precision and recall for rules alone, and record them in research R10. **If precision is under 0.9, T116 adds the small weights table**
- [X] T011 [P] **S5.** Across the render corpus, locate a chosen sentence by excerpt with `TextIndex::find` and the offset tiebreak. Record the success rate in research R2. Done, over 69 fixtures: an excerpt is found as read 90.5% of the time when read from the text part and 92.3% when read from what is drawn, and 98.7% once whitespace is collapsed (R2). `crates/postio-render/tests/excerpt_locate.rs` holds the floors, nightly
- [X] T012 [P] **S6.** On a spike commit (reverted after), switch the list's order to `sort_at` in `crates/postio-storage/src/repository/threads.rs` and `messages.rs`. Run `storage_suite`'s list counting tests, and record the result and the size of the diff in research R7. **The maintainer's default holds unless the spike says otherwise; T093 follows the result**. Done: do it, on three conditions (R7)

### `postio-widgets`: the shared crate, with no change in behaviour

- [X] T013 `git mv crates/postio-gtk/src/body_view crates/postio-widgets/src/body_view`, then a wiring commit. The `gtk_suite` reader tests pass with import paths changed and nothing else
- [X] T014 Fix: `BodyView::set_content_from_top` resets the selection, focused link, toggled folds and darkened key. Test first in `crates/postio-widgets/tests/widgets_suite/body_view_resets.rs`: darken message A, then show message B, and B is not darkened
- [X] T015 Fix: a render that times out shows the message's plain text (`Place.plain`, research R1). Test first: a `test-hooks` render that never finishes shows the plain text
- [X] T016 [P] `git mv` `widgets/{keyhint,keycap,action_bar,button,chip,notice,toast}.rs` into `crates/postio-widgets/src/widgets/`, and move their rules out of `crates/postio-gtk/data/shell.css` into `crates/postio-widgets/data/widgets.css`. The classic app loads it. `check-no-dead-css.py` and the CSS parse tests stay green
- [X] T017 Split the colour layer from the metrics (research R11). The widget CSS reads `--postio-*` variables, and the classic app keeps defining them from `tokens.css`. Test first: the CSS parse assertions, and the classic `shot` is unchanged pixel for pixel on the demo store
- [X] T018 [P] Move `mark_html` (`crates/postio-gtk/src/search.rs:1568`) to `crates/postio-ui/src/search.rs` with its tests
- [X] T019 `git mv` the reader (`reader/{view,message_header,banner,notices}.rs`, and `parts::Chips`) into `crates/postio-widgets/src/reader/`. Then a wiring commit in which the verb bars become configuration and the classic app passes its three. `gtk_suite` and `app_suite` pass unchanged
- [X] T020 One remote-image allowlist per app, shared by its readers. Test first: two readers in one app see one "always allow"
- [X] T021 [P] `git mv` `MessageList` (`crates/postio-gtk/src/list.rs:283-376`) to `crates/postio-widgets/src/list_model.rs`, generalised over its row type. The classic list's tests are unchanged
- [X] T022 Move the presenters to `crates/postio-widgets/src/present/`:. Partial: the config and remote-image presenters moved. Done: the compose seams and the cid blob source now go through `postio-client`, in `postio_widgets::present::{compose, reading}`, which the classic app and Focus both call. The credential dialogs moved with T165, and the frontend's blocking bridge moved to `postio_core::blocking`. The classic reader still resolves `cid:` with that bridge, while Focus prefetches; unifying them would reshape the classic reading pipeline, and was left
  - the compose seams (`crates/postio-app/src/compose.rs:77-98`);
  - the reading wiring (`reading.rs`);
  - the config service and watcher glue (`crates/postio-gtk/src/config.rs:111`);
  - the credential and add-account dialogs.

  `app_suite` passes unchanged
- [X] T023 Introduce `ComposerHost` in `crates/postio-gtk/src/composer.rs`. It replaces the composer's uses of `Window`: the pane, the context, the command broadcast, the keymap, the file dialogs' parent, and the autosave gate. Make `dispatch` public. Test first: a composer mounted on a test host autosaves after 1.5 s and sends
- [X] T024 `git mv` `composer.rs`, `editor.rs` and `data/editor.js` into `crates/postio-widgets/src/composer/`, and move `composer.rs`'s line in the spacing ratchet's baseline. The composer suites in `gtk_suite` and `app_suite` pass unchanged
- [X] T025 Fix: the detached composer's window follows dark mode (`style::track`). Test first: switch the style manager to dark, and the detached window carries `.postio-dark`

### One keymap: before any Focus surface, so Focus is built on its final keys

- [X] T026 `Availability.terminal: bool` becomes `Availability.frontend: Frontend {Classic, Terminal, Focus, Macos}`, and `Requirement::Focus` is added (`crates/postio-core/src/registry.rs:85-230`). Test first: a Focus-only row is unreachable to Classic and Terminal availability, and reachable to Focus
- [X] T027 [P] Add `Context::{Picker, Digest, Filtered}`, with their fallback chains (each falls back to Global only), in `crates/postio-core/src/context.rs` and `crates/postio-ui/src/keymap.rs:561-589`. Test first: `x` in Picker resolves to nothing, and `Escape` resolves to `back`
- [X] T028 [P] Add a `!` punctuation alias to `crates/postio-ui/src/keymap.rs:196-216`. Test first: `"!"` parses and resolves
- [X] T029 Apply contracts/keymap.md to `crates/postio-core/src/{command,registry}.rs`:. It added `Requirement::ThreePane` (flag, sidebar toggle, pane cycling, parts panel) and `Resolver::from_commands_for(keymap, Frontend)`, so each app binds only what it offers
  - every default and remap it lists;
  - the new ids;
  - `mark_unread` becomes `toggle_read`, and `focus_sidebar` becomes `go_to_folders`.

  Update, in the same commit:
  - `core_suite/command_registry.rs:75-164`;
  - `registry.rs:2256-2275` and `:2327-2340`;
  - `crates/postio-gtk/tests/logic_suite/keymap_defaults.rs:144-175`;
  - the golden `linux-bindings.txt`;
  - `docs/keybindings.md` (`POSTIO_UPDATE_DOCS=1`);
  - the terminal `app.rs` tests;
  - `docs/PRODUCT.md` §8.

  Test first: the cross-frontend enumeration (SC-015) in `crates/postio-core/tests/core_suite/one_keymap.rs`, red against today's registry
- [X] T030 [P] The terminal: `ctrl+z` resolves to `undo`, and spec 005's suspend claim is corrected (`contracts/tui-surface.md:103-105`, `spec.md:360`, and T028's note). Test first in `crates/postio-tui/tests/`: after an archive, `ctrl+z` restores
- [X] T031 [P] Add the key-map groups table in `crates/postio-ui/src/keymap_sheet.rs`. Test first: every command reachable with `Frontend::Focus` has a group
- [X] T032 Give the classic app handlers for the new commands every app offers: `toggle_read`, `go_to_folders`, `go_to_archive`, `go_to_snoozed` and `saved_search_1`–`4`. Test first: `app_suite/command_wiring.rs` lists them as unwired
- [X] T162 The terminal's handlers for the commands every app now offers: `go_to_archive`, `go_to_snoozed` and `saved_search_1`–`4` (in its GAPS today), and its signature delete moves off a hard-coded `d` onto the keymap. Test first: each key does in the terminal what it does in the classic app
- [X] T163 `mod+z` undoes in `Context::Digest` and `Context::Filtered` (contracts/keymap.md). Test first: `mod+z` resolves to `undo` in both contexts, for Focus

### Engine seams several stories need

- [X] T033 Add `ListScope::Focus(FocusScope::Inbox)` and `ThreadSummary.marker` (serde) in `crates/postio-model/src/{scope,listing}.rs`, with a Focus scope in `crates/postio-runtime/src/store/local.rs` equal to the unified inbox, and a predicate seam at every membership site (contracts/engine.md). Test first, with counting: a Focus inbox page is at most 3 statements, with no scans, and rows equal to those returned
- [X] T034 Add `Host::enable_focus`, `ClientKind::Focus`, a no-op `FilingPass` in `crates/postio-sync`, and empty body-stage and due-timer tasks. Test first: a probe counts calls, and it is called under Focus but never under the classic or terminal host
- [X] T035 [P] Build the `crates/postio-classify` skeleton: the `Outcome` schema, the `Facts` and `Rules` traits, and the layer pipeline with no rules yet. Test first: an empty pipeline returns an empty `Outcome`, and the boundary rule is green
- [X] T036 [P] Add `postio_search::date::parse_when(text, now)`, public, forward-looking, with a time of day. Test first: a table that includes "tue 9am", "thu 2pm", "tomorrow 8", "in 2 days", "oct 3 14:00", a date inside a daylight-saving change, and nonsense, which gives `None`
- [X] T037 [P] Extend the preset table in `crates/postio-ui/src/schedule.rs` with Snooze and Remind, in one wording that `/ux-architect` chooses (spec C14). Test first, at Saturday 16:09: the four snooze times of spec US5 scenario 1, and the four remind times. Done, except C14's wording, which T091 settles: `snooze_presets`' "Later today" and `schedule_presets`' "This evening" are pinned to one instant, so the choice is a change of words

**Checkpoint:** the classic app, the terminal and macOS are green on the one keymap and the shared crate. Focus's surfaces can start.

---

## Phase 3: User Story 1: work the inbox as it arrived (P1)

**Goal**: Screens 01, 02, 03 and 15. **Independent test**: spec US1.

- [X] T038 [US1] `crates/postio-gtk/src/{main,app,startup}.rs`: an `AdwApplication` with id `dev.postio.Postio.Focus`. The store opens on a thread behind a window that says what it waits for. `enable_focus` is called. Test first, in `focus_suite/starts_offline.rs`: a fixture store, no network, and the inbox is listed (scenario 1). Call `Host::enable_focus` before `start_syncing`, and connect as `ClientKind::Focus` (T034). Build the resolver with `Resolver::from_commands_for(.., Frontend::Focus)`, and set `Availability.frontend = Focus`. Call `host.enable_focus(FocusSetup::default().with_config(config.focus))`, and again on every `[focus]` change (T060). Pass the config path (`FocusSetup::with_config_path`), because Focus's config writes refuse without it, and call `Host::stop` on quit, which keeps the filing mark (T164)
- [X] T039 [US1] `window.rs`, the chrome of contracts/focus-surface.md:
  - the top bar: compose, the command-bar field with `/` and `Ctrl K`, the sync label, the menu and close;
  - the header strip: "Inbox ▾" `g o`, the counts, and "Has action" `!`.

  The filtered and digest counts stay hidden until those features exist. Test first: the widget tree carries each control and its key
- [X] T040 [P] [US1] `data/focus-colours.css`: the `--postio-*` colour variables from libadwaita's named colours, and the accent from `AdwStyleManager`. Test first: switching to dark changes the resolved colours, and a CSS assertion shows the accent only on markers, focus and the has-action toggle
- [X] T041 [US1] The list model: `FocusRow` over `postio_widgets::list_model` and `Paging` over `ListScope::Focus(Inbox)`. Test first: scrolling reads only the visible window (scenario 7, counted)
- [X] T161 [US1] Focus's inbox folds a conversation that reached two inboxes into one row, as Unified does (spec Edge Cases: "one inbox across all accounts"). T033 shows one row per inbox. The partner statements run only when more than one account is enabled, so one account keeps the page at 3. Test first: a conversation delivered to two accounts' inboxes is one row, archiving it archives both copies, and a one-account page is still at most 3 statements
- [X] T042 [P] [US1] `postio_ui::label_colour(name, accent_hue)`. Test first: a stable colour, never within the accent's hue band, and a label's stored colour when it has one
- [X] T043 [US1] The one-line row (40 px, one `snapshot()`): the gutter, sender, subject, up to two pills, first line, attachment icon, count and time, with bold for unread, and day headings. Test first: a row shows "RE: Q3 numbers!!" and "Hi all —" verbatim (scenario 2), and a third label draws no third pill (scenario 8)
- [X] T044 [US1] The two-line row (72 px) from `MarkerSummary`, with the kind chip, date, quote and actions with their keycaps. Test first: a marked row is 72 px, and its height does not change with focus or selection. An invitation's marker is not flipped to past as time passes, so read past from `ends_at` when drawing (T110)
- [X] T045 [US1] Cursor and selection: `x`, `J`/`K`, `X` (a predicate), `Escape`, and the bulk bar with keycaps. Test first: select three rows, put the cursor on a fourth, press `a`, and exactly the three are archived (scenario 3). Aim a folded row at `MessageTarget::Threads(id + copies)` (T161)
- [X] T046 [US1] Toast and undo: "Archived 3 messages · Undo", and `Ctrl+Z` returns all three, including after the toast has gone. Test first: scenario 3's undo half
- [X] T047 [US1] `j` and `k` move only the cursor: nothing opens and nothing is marked read. Test first: scenario 4
- [X] T048 [US1] The has-action filter: `FocusScope::HasAction`, the toggle's count, "Showing 7 of 312 · ! again to show all", the selection cleared, and the cursor kept. Test first: scenario 5, and the count is counted. `idx_markers_open` was not built, because the planner does not read partial indexes. Give the has-action scope its own counted seek
- [X] T049 [US1] Light and dark follow the system at once. Test first: scenario 6
- [X] T050 [US1] `crates/postio-gtk/examples/shot.rs`: a seeded demo store (the storage seed, plus markers written through the host) that renders a named screen, light or dark, at a given size. Test first: `shot 01` writes a PNG, and an unknown screen exits non-zero with `NO IMAGE WAS WRITTEN`
- [X] T051 [US1] Compare screens 01, 02, 03 and 15 with their PNGs, and record them in `screens.md`

---

## Phase 4: User Story 6: know the app's state (P1)

**Goal**: Screens 16–19. **Independent test**: spec US6.

- [X] T052 [US6] One `AdwBanner` for first sync, offline or a sign-in error, chosen in that priority, with the sync label to match. Test first: drive each state through the host's test seam, and the banner and label read as contracts/focus-surface.md says
- [X] T053 [US6] With no network, archiving, labelling and searching take effect at once and queue. Test first: scenario 1. The archive half is done: offline, an archive takes effect at once and queues. The label and search halves come with the label picker (T097) and the command bar (US4)
- [X] T054 [US6] First sync: what has arrived can be read and searched, and progress shows. Test first: scenario 2. The listing half is done. Reading and searching during a first sync come with T069 and US4
- [X] T055 [US6] Update password… opens the shared credential dialog. Test first: scenario 3. The credential dialog is still in `postio-gtk` (T022): route onboarding through `postio-client` first. Blocked on T165
- [X] T056 [US6] The empty inbox lists only what exists. Test first: scenario 4, with and without digests or filtering
- [X] T057 [US6] Compare screens 16–19, and record them

## Phase 5: User Story 7: one key map, taught everywhere (P1)

**Goal**: Screen 20. **Independent test**: spec US7.

- [X] T058 [US7] The key map dialog (1100×760), generated from the registry and the groups table: `?` and `Escape`, and the footer naming `[keys]` in `config.toml`. Test first: scenario 4, and every row's key equals the key the keymap resolves. Group with `postio_ui::keymap_sheet::{group, Group, KEY_MAP_CONTEXTS}`
- [X] T059 [US7] `focus_suite/registry_parity.rs`: every command reachable with `Frontend::Focus` has a key, a command-bar row and a visible control. It starts with a `NOT_YET` list that each story empties. Test first: the list is non-empty, and fails with names. `NOT_YET` starts with 131 commands, each missing its command-bar row. US4 empties it
- [X] T060 [US7] `ConfigChanged.focus`, and live reload of `[keys]` and `[focus]` in Focus. Test first: scenario 1 (override archive, save, and the key map, bar and button all change). `[focus]` is one struct, `postio_config::FocusConfig` (T132). Extend it and `change.rs`: `ConfigChanged` does not report `[focus]` yet
- [X] T061 [US7] Scenarios 2, 3 and 5 as `focus_suite` cases. The classic defaults are the one keymap's
- [X] T062 [US7] Compare screen 20, and record it

## Phase 6: User Story 11: one store, either desktop app (P1)

- [X] T063 [US11] The store-in-use screen: "Postio is already open in another window. Close it to open Postio here.", with Try again. Test first, in the pattern of `crates/postio-tui/tests/store_in_use.rs`: a second process holds the store, the sentence shows, and the store is byte-for-byte unchanged (scenario 1)
- [X] T064 [US11] Across apps: archive in Focus and the classic app shows the archive; a draft left in either opens in the other. Test first: scenarios 2 and 3 over one temporary store. The archive half is done. The draft half comes with Focus's composer (US3)
- [X] T165 Move the credential and add-account dialogs (`postio_gtk::onboarding`) and their probe and persist (`postio_app::onboarding`) behind `postio-client`, into `postio-widgets`: T022's remainder, which T055 needs. Test first: the classic app's onboarding cases pass unchanged, and Focus opens the dialog from its banner
- [X] T166 Commands Focus offers no surface for become `Requirement::ThreePane`: the conversation rail, the folder-list keys, the parts-panel keys and the account-list keys. `toggle_fold` and `expand_all` stay for Focus's dialog (FR-034). Test first: registry parity, with none of them in `NOT_YET`
- [X] T167 A select-all (`X`) in Focus is a predicate over Focus's own scope. Today it aims at the unified view, so it takes in held digest mail, and its exceptions remove only representatives. Give `postio-core` and `postio-session` a Focus view scope. Test first: `X` then `a` archives exactly what the list shows
- [X] T168 Folded copies in Focus's counts: the has-action page folds copies across accounts, as the inbox does (T161), and `focus_unread` counts a folded conversation once. Test first: a conversation in two inboxes counts once in each figure

## Phase 7: User Story 2: open a message, come back to the same place (P1)

**Goal**: Screen 04. **Independent test**: spec US2.

- [X] T065 [US2] `BodyView` gains a public highlight of a `TextIndex` range, and scrolls to it. Test first in `widgets_suite`: the overlay covers the range's rectangles, and the range is scrolled into view
- [X] T066 [P] [US2] A locator from excerpt to range in `crates/postio-ui/src/reader/`, using `TextIndex::find` with the offset tiebreak (research R2). Test first: a table that includes duplicates and diacritics. S5 (T011) says: collapse the excerpt's whitespace, and treat any run of whitespace as any other, on both sides. Done, in `postio-render` (`TextIndex::locate`), since `postio-ui` must not take on the renderer (R2)
- [X] T067 [US2] Quote folds with ids and line counts in single-message documents (`crates/postio-body/src/quote.rs`, `crates/postio-ui/src/reader/document.rs`). Test first: "31 quoted lines from v2 folded" shows and opens on activation, and the classic single-message reader gains it too (a `gtk_suite` case). The document half is done: ids `q0`…, and "N quoted lines" on every fold. The GTK half waits for T019's reader move. Also make the terminal's fold line count the same way
- [X] T068 [US2] `view_source`: `Req::RawSource` reads the raw blob, fetched on the key press if it is not local, and the dialog shows it. Test first: `v` shows the raw message's header lines (scenario 5). The engine half is done: `Client::raw_source(message)`, which fetches only on request. The dialog remains
- [X] T069 [US2] The dialog (980×820): the header with Close, the title, the position, and `k`/`j`; the toolbar with its keys; the shared `Reader` in single-message mode with the Focus header card, the marker slot, attachment cards and the fold line. Test first: 100 opens build one surface (`surfaces_created` = 1, scenario 7), and the dialog opens within one frame of `Enter`
- [X] T070 [US2] `Escape` returns with the selection kept. `j`/`k` step the list and move its cursor, and `[`/`]` step the thread. Test first: scenarios 1–3
- [X] T071 [US2] `open_attachment_or_link`: a chooser over the snapshot's links and the message's parts. Nothing opens without a choice, and a link's target shows first. Test first: scenario 9
- [X] T072 [US2] HTML-only mail with remote images: sanitised, images blocked per sender, no script, no request. Test first: scenario 4, reusing the reader's no-request fixtures
- [X] T073 [US2] The marker card, and the sentence highlighted in the body (through T065 and T066). Test first: scenario 6, with a seeded question marker. Pass `own_text`'s length as the excerpt's `source_len` (R2)
- [X] T074 [US2] Compare screen 04, and record it (research R2's scrolling difference included)

## Phase 8: User Story 3: write and reply with the existing composer (P1)

**Goal**: Screens 05 and 06. **Independent test**: spec US3.

- [X] T075 [US3] The `correspondents` table, maintained at local send (`crates/postio-sync/src/send.rs:525-575`) and when Sent syncs. Test first: sending to three addresses adds one to each, counted. Fill `sent_count` in `recipient_directory()`: the rows carry it since T076. `sent_count` reached the directory only with T076 (`CorrespondentRepository::sent_counts`)
- [X] T076 [P] [US3] `RecipientDirectory` rows carry `sent_count`, and completion ranks by it, with one rule for both apps (`crates/postio-ui/src/recipients.rs`). Test first: an address written to 42 times ranks above one seen 100 times and never written to (scenario 6). The rule is done: `postio_ui::recipients::suggest`, with ADR 0007's band (R15). Still open: the classic composer calls `Directory::suggest`, and the terminal ranks in SQL. Move both to the one rule after T024
- [X] T077 [US3] `Draft.labels` (`crates/postio-model/src/draft.rs`), and the host applies them to the Sent copy's conversation. Test first: scenario 4
- [X] T078 [US3] `DialogHost` for `ComposerHost`, in the 980×820 frame of screens 05 and 06: the header, the fields with Labels, the footer, and "Draft saved locally". Test first: `E` fills every recipient, "Re:", the thread's labels, and a folded quote (scenario 1). The compose seams are still in `postio-gtk` (T022): Focus reaches compose through `postio-client`
- [X] T079 [P] [US3] An opt-in recipient chip entry in `crates/postio-widgets/src/widgets/recipients.rs`, which Focus turns on. Test first: choosing a suggestion adds a chip with name and address (scenario 6)
- [X] T080 [US3] Drafts: `Escape` saves locally, and a draft opens in either app. Test first: scenario 3
- [X] T081 [US3] Focus has no composer of its own. Test first: the same content from Focus and from the classic app queues byte-identical messages (scenario 2)
- [X] T082 [US3] Sending offline goes to the Outbox and leaves at most once. Test first: scenario 7
- [X] T083 [US3] Compare screens 05 and 06, and record them (C7 and C23)

## Phase 9: User Story 4: search, go to and run commands from one bar (P1)

**Goal**: Screens 07–10. **Independent test**: spec US4.

- [X] T084 [P] [US4] `postio_search::natural::lower(text, today, names)`. Test first: a phrase table that includes screen 07's sentence, the words it cannot lower, and determinism (scenarios 1 and 8)
- [X] T085 [P] [US4] The finder's blended mode in `crates/postio-ui/src/finder.rs`: commands, places and one search row, grouped, with `>` for commands only. Test first: typing "arch" gives the three groups of screen 09, and a command acts on the aim held before the bar opened (scenario 3)
- [X] T086 [US4] The bar overlay (860 px): the saved row (`Alt+1`–`4`), the input with chips, `Tab` into the chips, `back_to_words`, the echo line, the results, and the footer. Test first: half-typed operators show no error (scenario 2), and `in:Rec` lists Receipts newest first (scenario 4). The bar is built: blended rows, names read as senders, plain words kept as words, a dimmed list, and a click outside closes it. Still to do: `Tab` into the chips, `back_to_words`, and `Ctrl+S` to save the search
- [X] T087 [US4] `saved_search_1`–`4` in Focus. Test first: `Alt+2` shows its results (scenario 5)
- [X] T088 [US4] The folders popover (`g o`, or clicking "Inbox ▾"): mailboxes with their keys, folders, labels and counts, a filter, and `Enter` goes there. The header then names the place. Test first: scenario 6
- [X] T089 [US4] Search stays within its budget, counted: scenario 7
- [X] T090 [US4] Compare screens 07–10, and record them ("invoice" as free text included)

## Phase 10: User Story 5: snooze, remind, label and move from a picker (P1)

**Goal**: Screens 11–14. **Independent test**: spec US5.

- [X] T091 [US5] The pickers in `crates/postio-widgets/src/widgets/pickers/`: a popover anchored to the row, its title and target, preset rows with number keys, a date entry (`parse_when`), a footnote, and the picker commands in `Context::Picker`. Test first in `widgets_suite`: `2` picks the second preset, and `Tab` focuses the date entry. Settle C14's wording first (`/ux-architect`). This task gives `snooze_presets`, `remind_presets` and `parse_when` their first callers, so delete their lines in `scripts/checks/uncalled-pub-fn-baseline.txt`
- [X] T092 [US5] `Command::Snooze { until }` (core, session and host). Test first: scenarios 1 and 2 at a fixed clock. `Command::Snooze` has no `until` yet (T029 kept the payloads minimal)
- [X] T093 [US5] `messages.sort_at` (per T012): the schema, the list's order, seek marks and indexes, and a woken snooze setting it. Test first: a woken snooze lists at the top, and `list_statement_count.rs` and `threads.rs:340` are unchanged. **If T012 chose the alternative:** change screen 11's copy instead, and record it. S6's conditions (R7): the folder and conversation lists move while the query views and search stay on `received_at`; `write_update` keeps `sort_at` at least `received_at` so drafts still rise, with a test; raw test inserts name the column. Move Focus's own window too: `focus_arm`'s `ORDER BY`, cursor and `focus_at`, and `representative_filter`
- [X] T094 [US5] The `reminders` table, `remind_if_no_reply { at }` (undoable), cancellation by the filing pass on a reply from someone else, and firing on the tick. Test first: scenarios 3 and 4, including Focus closed at the due time and offline. `Command::RemindIfNoReply` has no `at` yet. The due timer does not fire reminders yet
- [X] T095 [US5] Surfaced reminder rows: splice positions in `crates/postio-ui/src/list.rs`, `FocusRow::Reminder` as a two-line "No reply since …" row, and the Focus scope leaving out the conversation's ordinary row. Test first: the position is 1 statement, and a surfaced conversation is not listed twice. The host already fires reminders and lists them through `surfaced()`. Settling a reminder when the person replies or archives is still to do here. The rows are done: `FocusRow::Reminder` spliced at its place, listed once, and gone when archived, with undo. Still to do: settling when the person themselves replies, in the filing pass in `postio-sync`, which today settles only on other people's replies
- [X] T096 [US5] `Draft.remind_at` (Remind if no reply, `mod+h`, in the composer) becomes a reminder on send. Test first: US3's scenario 5
- [X] T097 [US5] The label picker: filter, `Space` toggles (`add_label` on or off), create, "✓ applied", counts. Test first: scenario 5
- [X] T098 [US5] The move picker: filter, Recent (`settings` key `focus.move_recent`), All folders, `Enter` moves, and `Ctrl+Z` undoes. Test first: scenario 6
- [X] T170 [US5] The label picker reads the labels of the row's own account. Today it reads the first account's, because a row does not say which account it belongs to. Test first: with two accounts, labelling a row of the second offers the second's labels
- [X] T099 [US5] `Escape` closes any picker without a change. Test first: scenario 7
- [X] T100 [US5] Compare screens 11–14, and record them

**Checkpoint:** all P1 stories are done. Focus is a complete mail client on the one keymap, with no differentiator yet.

---

## Phase 11: The classification engine (blocks US8, US12, US9 and US10)

- [X] T101 `postio-classify` guards through `Facts`: `wrote_to` (from `correspondents`), `took_part`, `own_domain` and `never_filter`. Test first: a fixture table in `crates/postio-classify/src/` unit tests
- [X] T102 `FilingPass` in `commit_batch` (`crates/postio-sync/src/initial.rs:630-713`), for incremental passes only. Test first:
  - a first sync never calls it, and an incremental pass does;
  - an error leaves the mail in the inbox, and the insert commits;
  - at most 4 statements per new message plus writes, counted
  - one message type crossing the seam: `postio_classify::FiledMessage` (T035) and the pass's own (T034) become one, in `postio-model` or at the host. It carries the thread, because a message with no `thread_id` is guarded and never filtered (T101)
  - T034 put the pass in `resync::incremental`'s write unit, not `commit_batch`, which only first syncs, rebuilds and re-enumerations reach; an error there still rolls the unit back, and this task makes it commit the insert (contracts/engine.md)
  - Gmail and JMAP accounts never reach `resync::incremental`, because they have no MODSEQ, so the pass must also run on their incremental paths; otherwise Focus files nothing for them
  - a unit test that the store's reason vocabulary (`filter_decisions`' CHECK) agrees with `postio_classify::ReasonKind`, where this task maps one onto the other
- [X] T103 The body-stage task: `BodyLoaded`, debounced, and a catch-up over `focus_classified`, newest first, at background priority. Test first: rows with no record are processed, and a version bump runs them again
- [X] T104 [P] Promote three headers:. Done for IMAP and Gmail. JMAP learns the three from the body: io-jmap 0.3 cannot ask for single headers (contracts/engine.md)
  - the IMAP fetch on incremental syncs (`crates/postio-account/src/imap/fetch.rs:213-248`), and the same fields for JMAP and Gmail;
  - `messages.unsubscribe_offered` and `messages.automation`, filled from the body's headers otherwise;
  - `is:bulk` and `is:automated` in `postio-search` and `postio-index`.

  Test first: the fixtures' operators match, and a first sync's fetch is unchanged
- [X] T105 [P] The automated-senders table, shipped as TOML and loaded as data (`crates/postio-classify/data/senders.toml`). Test first: patterns match their fixtures, and the classifier holds no provider constant
- [X] T106 [P] Add a note to ADR 0025 on the three promoted headers, and a line to ARCHITECTURE §6

## Phase 12: User Story 8: answer an invitation from the row (P2)

**Goal**: Invite markers on 01, 03 and 04. **Independent test**: spec US8.

- [X] T107 [US8] The `postio-calendar` adapter (per T007): `parse`, `reply` and `supersedes`. Test first: T007's fixtures, and a REPLY's `ATTENDEE;PARTSTAT` for accept and for decline
- [X] T108 [US8] The backfill fetches `text/calendar` parts of 256 KiB or less (`crates/postio-sync/src/backfill.rs:1509`). Test first: against the mock backend, the part is stored with the body, and a 1 MiB part is not
- [X] T109 [US8] The `markers` table and its repository, and markers read in one batched statement per Focus page. Test first: a page with markers is still at most 3 statements, with no scans. The runtime layer adds the inbox witness to the page's statements (contracts/engine.md)
- [X] T110 [US8] The body task turns an invitation into a marker: open, updated, cancelled or past. Test first: scenarios 1, 4 and 5. Place floating and all-day times with `Invitation`'s `instant_in`, in the user's zone. Add the series' last occurrence to the adapter for a recurring "past". Delete the `instant_in` and `supersedes` lines in `scripts/checks/uncalled-pub-fn-baseline.txt`. `MarkerRepository::invitations(uid)` is built. Delete its line in `scripts/checks/uncalled-pub-fn-baseline.txt`
- [X] T111 [US8] A calendar part in `outgoing::build` (`crates/postio-model/src/outgoing.rs`). Test first: the REPLY sits in the `multipart/alternative`
- [X] T112 [US8] `accept_invite` and `decline_invite`: a reply from the matching identity, `queue_send_at(now + 10 s)`, the marker `accepting`, and a window entry on the undo stack that expires with the window. The due timer does not expire an RSVP window yet. Test first:
  - nothing reaches the transport before the window ends (scenario 2);
  - undo within it sends nothing;
  - after it, exactly one reply leaves (scenario 3)
- [X] T113 [US8] The row and the dialog: the Invite line with Accept `y` and Decline `Y`, and the "Accepted · Undo" toast. Test first: a `focus_suite` case over an invitation fixture
- [X] T114 [US8] Compare invitation markers on screens 01, 03 and 04, and record them

## Phase 13: User Story 12, milestone 1's part: the built-in needs-action detector (P2)

- [X] T115 [US12] Own-text extraction in `crates/postio-body`: the newest message's text without quoted history or signature, with stable offsets. Test first: fixtures with top-posted and bottom-posted replies. S5 (T011) says: read the part the reader draws, the HTML flattened when there is HTML. Leave out what is never drawn: `<title>`, hidden preheaders, `alt` text
- [X] T116 [US12] The detector's rules in `crates/postio-classify`: questions and to-dos, the exclusions, due dates through `parse_when`, and one marker per message (research R10). Test first: the labelled corpus's precision gate of at least 0.9 (SC-013), red against the empty detector. Add the weights table only if T010 said so. S4 (T010): rules with R10's four fixes reach precision 0.901 on 201 items. Build those, and keep the weights table in reserve for a failed gate. Done: precision 0.985, recall 0.823, and no weights table (R10)
- [X] T117 [US12] The body task writes question and to-do markers for mail sent directly to the user. The catch-up covers the inbox and the last 30 days. Test first: scenarios 1–3 and 7 (no model configured: markers still appear, and nothing connects). Cut the excerpt as a plain prefix of the sentence, at most 200 characters, with no ellipsis, or the locator cannot find it (R2). Pass `BodyMessage.identities`, and `Senders::shipped()` through `Rules::senders()`. Cut own text with `postio_body::own_text`, and switch the gate test in `crates/postio-classify/tests/needs_action.rs` from the spike's approximation to it
- [X] T118 [US12] `dismiss_marker { dismissed }` (undoable), and three dismissals write `[focus.filter] stop_markers`. Test first: scenario 5
- [X] T119 [US12] Instruction-shaped text produces no action and no request. Test first: scenario 4, on ADR 0009's fixture
- [X] T120 [US12] Compare question and to-do markers on screens 01 and 03, and record them

## Phase 14: User Story 9: spam and updates filtered, with reasons (P2)

**Goal**: Screen 21, and the counts on 01, 10 and 16. **Independent test**: spec US9.

- [X] T121 [US9] The `filter_decisions` table, its repository, and the reason vocabulary. Test first: the CHECK refuses an unknown reason
- [X] T122 [US9] The filing pass's header rules: list, bulk and automated signals, the senders table, and the server's `$Junk`. With the guards, it writes a decision, archives through the storage verbs in the transaction, and queues the server move. Test first, on the known-answer corpus: scenarios 1–3 and 6, and zero guarded messages filtered (SC-006). Set `thread_id` at filing: without one, a message is guarded and never filtered (T101)
- [X] T123 [US9] `[focus] filtering`, and `[focus.filter] never` in `crates/postio-config`. Test first: `filtering = false` files nothing new, and a `never` sender is not filtered. Extend `FocusConfig` (T132)
- [X] T124 [US9] The Filtered scope, its tab counts, and the view (screen 21): the header bar, tabs `1`–`7`, reason pills, the focused row's restore button, and the footer. Test first: scenarios 5 and 7, counted
- [X] T125 [US9] `restore_filtered { restored }` (undoable): the move to the inbox, the decision deleted, and the sender added to `never` through `toml_edit` and `write_atomically`. Test first: scenario 4, where undo reverses all three
- [X] T126 [US9] "Filtered today" in the header strip, the popover's Filtered row, and the empty state's shortcut. Test first: the count equals decisions since local midnight
- [X] T127 [US9] Catching up on Focus's open: mail filed while another app ran is sorted. Test first: scenario 8
- [X] T164 [US9] Focus keeps `focus.filed_through` current when it stops, not only on its five-second tick. Today, up to one tick of its own sync's mail sits past the mark and is sorted again at the next open, which would filter a new account's first-sync rows against FR-118. Test first: stop Focus between ticks, reopen, and a first sync's rows are not filed away
- [X] T128 [US9] `sweep_inbox`: a preview count, then one undo unit, with its key chosen with the enumeration test. Test first: FR-118. The sweep reads each row once per batch of 50, one statement per row. That is acceptable in the background, but it is the one known N+1
- [X] T129 [US9] Focus never notifies for mail it filtered or held. Test first: FR-153. The host half is done: `NewMail` names only the arrivals that stayed in Focus's inbox, and filing mail away emits `MessageListChanged`. The frontend half waits for Focus's notifications
- [X] T130 [US9] Compare screen 21, and record it (C4's copy included)

## Phase 15: User Story 10: digests on the user's cadence, by sender (P2)

**Goal**: Screen 24, the digest rows on 01 and 16, and the digest window in 22's frame. **Independent test**: spec US10.

- [X] T131 [P] [US10] A matcher for `from:` and `list:` in `crates/postio-search`. Test first: ADR 0008's differential test against the executor over the corpus. Done: `postio_search::matcher::Matcher`. It mirrors the executor's `from:` bug, #1699, as ADR 0008 requires, so fixing #1699 changes both in one commit
- [X] T132 [P] [US10] `[[focus.digests]]`: parse, validate, and compute the next due time, safe across daylight-saving changes. Test first: contracts/config.md's validation list, and a weekly rule across a DST change. Step calendar days (`date_naive() + Days`), never `+ Duration::days`, which is 24 hours (#1700). Resolve the time with `parse_when`'s rule: a skipped time is pushed forward, a repeated one is its first occurrence
- [X] T133 [US10] The `digest_holds` and `digest_deliveries` tables. The filing pass holds matching mail, but never mail with an invitation or question or to-do, and never a conversation the user took part in. Test first: scenarios 1 and 2. The tables and `DigestRepository` are built (hold, release, release_rule, deliver, archive_delivery). The filing half waits for T102
- [X] T134 [US10] The Focus inbox scope leaves out held mail at every membership site, and its counts do too. Test first: counting assertions, and totals, seek marks and rows agree
- [X] T135 [US10] The due timer creates deliveries. One missed while Focus was closed is delivered once on start, and an empty one is not created. Test first: scenarios 3 and 6
- [X] T136 [US10] Surfaced digest rows (`FocusRow::Digest`) with the senders line, spliced into the list. Test first: a delivery's row sits where its count places it. Add `Event::SurfacedChanged` (contracts/engine.md): nothing yet tells a frontend that a digest was delivered. The engine half is done: `Event::SurfacedChanged`, and `Client::surfaced()` returning `Surfaced::{Digest, Reminder}` with position. `FocusRow::Digest` remains. `focus_position` can overcount a conversation filed as copies in two accounts
- [X] T137 [US10] The digest window (22's frame, the plain list): the header, Archive all `⇧A` as one undo, `d` to edit, `D` to stop (in config, releasing what is held), `U`, and `Escape`. Test first: scenarios 4 and 5. Delete `archive_delivery`'s line in `scripts/checks/uncalled-pub-fn-baseline.txt`. The engine half is done: `Command::ArchiveDigest` (one undo) and `StopDigestingSender` (the undo restores a removed rule in its place). The window remains
- [X] T138 [US10] The rule dialog (screen 24), from `d` or from "Digest these…" in the bulk bar: the address, cadence, day and time, the preview through the executor over the last 90 days, the note, and Create, which writes the config. Test first: scenario 1's preview count equals the executor's. The engine half is done: `digest_preview` and `save_digest_rule`. The preview adds up each query's count, so a message two queries match counts twice: count distinct messages instead. The dialog remains
- [X] T139 [US10] `/ux-architect` designs the `g d` rules list (spec C15). Then build it: rows, edit in the dialog, and `Delete` removes the rule and releases what it held (FR-126). Test first: removing a rule releases its held mail into the inbox. Delete `release_rule`'s line in `scripts/checks/uncalled-pub-fn-baseline.txt`. The engine half is done: `delete_digest_rule` releases what the rule held. The design and the list remain
- [X] T140 [US10] The empty state's "Next digest", and "N digest rules" in the header strip. Test first: scenario 7 (held mail is found by search, and results say where it waits)
- [X] T141 [US10] Compare screen 24 and the digest rows on 01 and 16, and record them

---

## Phase 16: Polish and cross-cutting (closes milestone 1)

- [X] T142 [P] Accessibility (FR-096): each row announces the sender, subject, first line, unread and marker, and keycaps are exposed as shortcuts. Test first: an accessible-tree assertion in `focus_suite`, followed by an Orca pass by hand. Done in code, with an accessible-tree case in `focus_suite`. The Orca pass by hand is the maintainer's
- [X] T143 [P] No transition over 100 ms, and reduced motion honoured (FR-094)
- [X] T144 [P] Packaging: `flatpak/dev.postio.Postio.json` builds `postio-gtk`, `crates/postio-gtk/data/dev.postio.Postio.Focus.desktop` is added, and release.yml's `flatpak` job carries both apps. Test first: the packaging test pattern of `crates/postio-tui/tests/packaging.rs`. Focus is a second launcher in the one Flatpak, so the classic metainfo names it rather than a second component, and the release job checks that the build carries both apps
- [X] T145 [P] Documentation:
  - `docs/PRODUCT.md` §2 and §23;
  - `docs/ARCHITECTURE.md`: the shape diagram gains the six crates (widgets, focus, classify, calendar, ai and vault), and §9 their boundaries;
  - a note in ADR 0032 on Focus's one-message dialog (FR-037)
  - `docs/config.md`: the `[focus]` section, `[[focus.digests]]` with its due-time rule, `[focus.filter]` and `[focus.model]`
- [X] T146 Draft the constitution's Scope amendment (MINOR): name Focus, and allow its optional, user-supplied local model. Update the Sync Impact Report. **It waits for the maintainer's approval, and does not land without it**. Drafted and approved by the maintainer 2026-09-27 ("approve as drafted"): 1.3.0, with CLAUDE.md's scope line to match
- [X] T147 [P] SC-011: the first classification pass over a 100,000-message store, as a `POSTIO-MEASUREMENT:` test in `.config/nextest.toml`'s nightly profile. Done, with an in-memory store:
  - needs-action alone: 3,000 bodies in 3.7 s, against a 60 s budget;
  - the filing catch-up alone: 61,188 messages in 177 s, against 300 s;
  - the two together: each about 184 s (T169)
- [X] T169 First open on a large backlog: the filing catch-up and the needs-action pass share one background connection. Together they take about 184 s, missing needs-action's 60 s budget, and inbox page reads reach 380 ms. Give the passes their own connections or a priority, and have both yield to interactive reads. Test first: T147's measurement with both passes running, needs-action inside its budget, and page reads within 16 ms at the median. Done. The filing catch-up held the write permit through its pause. Now it drops the permit before pausing, starts only once needs-action has caught up, and both passes yield to a read in flight. With both running: needs-action 3.9 s, filing 163.5 s, page reads 6 ms at the median
- [X] T148 `screens.md` complete for 01–20, with every difference and its reason (SC-009)
- [ ] T149 Walk quickstart.md by hand (scenarios 1–11) on a throwaway store, and record the outcomes in `screens.md`'s notes
- [X] T171 First run with no account: Focus opens the shared add-account form (`postio_widgets::present::onboarding::add_account`, moved in T165) over its window, as the classic app's first run does, and lists the inbox once the account is saved. The spec named no first-run path; the maintainer's T149 walk found none (2026-09-29). Test first: a `focus_suite` case over an empty store shows the form, and saving through a scripted client lists the inbox
- [X] T172 Compose with no account: `c` (and the top bar's compose button) says what is missing and offers to add an account, rather than doing nothing, since the composer is mounted only once an account is known. Found in the maintainer's T149 walk. Test first: with no account, `c` shows the sentence and its action
- [X] T173 The top bar's close button hovers in a rectangle; it should hover as a circle, the way `circular` promises (screens 01 and 02). Found in the maintainer's T149 walk. Test first: a CSS assertion that the close button's hover rule keeps its border-radius
- [X] T174 The add-account form over Focus: the window's close (x) still closes the app while the form is open; the form opens large enough that the server details show without scrolling; and its Back/return button matches the design's size. Found in the maintainer's T149 walk (2026-09-29). Test first where a test can see it
- [X] T175 Buttons and keycaps are sized as the design draws them (screens 01–25): a keycap is a small inline hint inside its button, not a second bordered box that doubles the button's height. Applies to the top bar, the open-message toolbar, the compose dialog, pickers and dialogs. Found in the maintainer's T149 walk (2026-09-29). Test first where a test can see it
- [X] T176 `j`/`k` work inside the open-message dialog (next and previous message, as its header's `k`/`j` hints promise). Found in the maintainer's T149 walk (2026-09-29). Test first where a test can see it
- [X] T177 The compose dialog shows its buttons one consistent way: Close, Send later, Send, Attach and Remind if no reply each as the same button with its keycap inside, as screen 05 draws. Found in the maintainer's T149 walk (2026-09-29). Test first where a test can see it
- [X] T178 `x` and the list keys during a first run's first sync. The maintainer's report was not reproduced by a test following the real path (`the_list_keys_work_while_the_first_sync_fills_the_inbox` guards it); a related bug was fixed with T182: closing the bar left focus in its hidden entry, which swallowed single-letter keys
- [X] T179 A visible Delete, in the bulk bar and the open-message toolbar, and `Delete` and `a` inside the open message; `delete` leaves NOT_YET
- [X] T180 A message opens at its top, every time
- [X] T181 In the open message, arrows, Page keys, space and Home/End scroll the body, not the list
- [X] T182 The command bar per spec C24: `ctrl+k` opens it in command mode with `>` filled in, `/` opens it for search, and it opens in place in the top bar's field with results below it rather than as a popup. Update `contracts/keymap.md` and `docs/keybindings.md` to match. Test first: `ctrl+k` shows `>` and only commands, and `/` shows the search row
- [X] T183 The open message is one scrolling column (screen 04): header, marker card, body, attachments and the fold line scroll together, and a plain-text body is drawn without a frame, at the reading measure and size screen 04 draws. An HTML body that sets its own background keeps a quiet frame. This reverses research R2's separate body scroller, which screen 04 always contradicted. From the design review of 2026-09-29 (screen 04). Test first where a test can see it; render and compare in light and dark
- [X] T184 The message header card (screen 04): a tinted card with small dim From/To/Cc labels, the sender's name bold and the address dim mono, a Cc line when there is one, and a relative date for today ("Today, 15:22"), absolute beyond. From the design review of 2026-09-29 (screen 04). Test first where a test can see it; render and compare in light and dark
- [X] T185 Attachments as cards (screen 04): a file icon, the name in mono and the size beneath, in a row under the body, not pills over it. From the design review of 2026-09-29 (screen 04). Test first where a test can see it; render and compare in light and dark
- [X] T186 The message's label pill carries its colour dot from `postio_ui::label_colour`, as the list's pills and screen 04 do. From the design review of 2026-09-29 (screen 04). Test first where a test can see it; render and compare in light and dark
- [X] T187 The open-message dialog's Close button is the same compact pill as its toolbar buttons (T175 did not reach it). From the design review of 2026-09-29 (screen 04). Test first where a test can see it; render and compare in light and dark
- [X] T188 The open message's column shows a stray horizontal line mid-message while scrolling, gone once scrolling stops: a tiling seam in `BodyView`'s flow mode (T183). Test first if a tile-boundary assertion can see it. From the maintainer's walk (2026-09-30). Not reproduced in a test; the inferred cause, tile edges on fractional device pixels at a fractional scale, is fixed by snapping every tile edge to a device pixel (`tiles::placements`). The maintainer confirms on a real display
- [X] T189 The open message's header: Close becomes an X icon button, and it sits where the up/down (k/j) buttons are now, with those moved to the left, per the maintainer. From the maintainer's walk (2026-09-30)
- [X] T190 Deleting (or archiving) the open message moves the dialog to the next message in that folder, as the list's cursor does, instead of leaving the deleted message on screen; at the end, the previous; with none left, the dialog closes. From the maintainer's walk (2026-09-30)
- [X] T191 The list's cursor ring moves instantly: no transition on the focused row's border, so `j`/`k` through the list are not slowed (the <=100 ms motion budget, and none on cursor moves). From the maintainer's walk (2026-09-30)
- [X] T192 One rule for every close button in Focus: an X icon button at the top right, the same widget and style everywhere (the open message's header has it since T189; the composer still closes from the left). A test walks every surface with a close control and asserts the same placement and widget, so the next surface cannot drift. From the maintainer's walk (2026-09-30) Done: `widgets::close_button()` is the one constructor (X icon, right end of the header, no keycap); the composer, digest, raw source and key map moved to it; `close_buttons::*` walks every surface. Cause: each surface hand-built its own close, worded and at the left.
- [X] T193 The window's title-bar buttons (minimise, maximise, close) hover as a square; they hover in the shape of the button, as stock GNOME chrome does. Find which Focus or widgets rule is squaring them. From the maintainer's walk (2026-09-30) Done. Cause: not a stylesheet rule -- the top bar's icon buttons (compose, menu, close) were laid out at the default `valign` inside a 46px bar, so each was 26x46 and hovered as a tall pill whatever its `border-radius`; they are centred at their own size now (`window_controls::*`). The app draws no minimise/maximise of its own, so the report may be of these.
- [X] T194 Double-clicking a message in the list opens it, as Enter does. Then audit every list and dialog interaction for the keyboard and mouse pair a person expects (click selects, double-click or Enter opens, Esc closes, a selection checkbox toggles, right-click where a menu exists), fix what is missing, and record the table in `screens.md`. From the maintainer's walk (2026-09-30) Done: a double-click (GTK's `activate` on the list) goes to the row and runs the one `OpenMessage` command; a gutter press toggles selection. Audit table in `screens.md`; Ctrl/Shift-click is T198 and the row menu T199 (new).
- [X] T195 `j`/`k` in the open message do nothing. Earlier tests passed, so the test drives a real key press through the window, with focus wherever opening a message actually leaves it (inside the column, on a link, in find), and sees the dialog move to the next or previous message. From the maintainer's walk (2026-09-30). Cause: GTK runs a key press only from the focus up to the innermost dialog over the window, so the window's capture controller -- the only one that knows the keymap -- never saw a key while any dialog was up; the tests called `handle_key` directly. Every dialog but the composer's now gets a controller that asks the window (`keys_under_dialogs`), and `support::deliver` presses keys through the controllers GTK would run, stopping at the dialog as GTK does (measured with real key presses injected through mutter's RemoteDesktop)
- [X] T196 The open message scrolls in steps: Up/Down (and the scroll keys) move a line or a page, never all the way to the top or bottom. Today one press jumps to an end. From the maintainer's walk (2026-09-30). Cause: T195's -- the arrows never reached the window's `scroll_reading`, so they fell through to GTK's focus moves, and the column's viewport scrolled to whichever control took the focus; fixed with T195, pinned by `open_keys::the_arrows_and_paging_keys_scroll_the_open_message_in_steps`
- [X] T197 The open message's column has too much padding at the sides: the reading width is reconsidered with `/gtk-design` and the canvas, against a screenshot, in light and dark. From the maintainer's walk (2026-09-30). Cause: the reference's 860px clamp inside the 980px dialog, and a 700px body measure inside that, left 60px a side and a dead margin right of the text. Decided: the column is the dialog's width less a `--postio-space-6` gutter a side, the toolbar's inset, and the body fills it, so the toolbar's words, the subject, the cards and the body share one left edge; pinned by `open_reading::the_column_fills_the_dialog_less_a_gutter`
- [X] T198 Pointer selection on the list: Ctrl-click toggles a row's selection and Shift-click extends it to a range, the mouse pair of `x` and `Shift`+`j`/`k`. Found by the T194 audit. Test first, driving the click with its modifier Done: the row's own `GestureClick` reads Ctrl or Shift and the window runs `ToggleSelection` or walks `ExtendSelection*` from the anchor; tests `pointer_pairs::ctrl_click_toggles_a_rows_selection` and `shift_click_extends_the_selection_to_a_range`.
- [X] T199 A right-click menu on a list row: the row's verbs (archive, snooze, label, move, mark read) as the toolbar offers them, each running its one command with its key shown. Found by the T194 audit; the verbs to include are a `/ux-architect` call Done: `row_menu::RowMenu` offers Open, the replies, Archive, Snooze, Remind, Mark read, Label, Move, Digest and Delete with their keys, each through the window's one `act`; the cursor goes to the row, inside the selection it acts on the selection, outside it on the row (the selection let go only when a verb runs); Escape closes it. Decision in `screens.md`; `row_menu::*`.
- [X] T200 Every Focus test that presses a key or clicks goes through what GTK would run: keys through `support::deliver` (T195), clicks through a pointer counterpart (a real `GestureClick` press/release with n_press, double-click included), never by calling a handler or emitting a signal directly. Audit `tests/focus_suite`, move each case, and fix whatever turns red: two bugs (T195, the first j/k report) got past cases that skipped delivery. Done 2026-10-01: `support::press`/`keys` now deliver through GTK's controllers (`deliver_with`), `support::click`/`click_in`/`click_at`/`click_row_saying` are the pointer counterpart (pick, then `GestureClick` pressed/released with n_press along GTK's path; GTK cannot build a button event in-process, so the signals are driven, and a real RemoteDesktop injection could not be made to reach a private compositor from this lane); every handler call and emitted signal moved (double-click, gutter, row actions, bulk Delete, pills, banner/toast buttons, alert-dialog responses, popover rows). Four real bugs surfaced: the composer dialog never got the window's keys (Esc, Ctrl+Return, mod+h dead in the composer), and three focus-orphan cases where a redraw or removal left the window's focus on a widget with no parents so no key reached it (rules list, Filtered list, any focused row removed). Kept as dispatcher units: `keymap`, `one_keymap` (resolver), `one_composer` (stub host)
- [X] T201 A click inside an open dialog reaches what the window does with clicks (`click_through_dialog`): verify with real delivery whether the dialog stops it as it stopped keys (T195), and fix it the same way if so. Done 2026-10-01: the click on the covered close button lands on the dialog host's scrim, which is outside `AdwDialog`, so the window's capture gesture is reached and `click_through_dialog` works (test now picks and clicks for real, and asserts the scrim is what is under the button); a click inside the form is the form's and closes nothing (new case). Whether the dialog stops a click inside it, as it stops keys, is not measured, and nothing needs it: the gesture acts only on the close button's place, which the form does not cover
- [X] T202 One rule for icon buttons, as T192 is for close: every icon button in Focus is centred at its own size and hovers in its own shape; a test walks every surface's icon buttons (the way `close_buttons` walks closes) so a stretched one cannot come back (T193) Done: `icon_button()` centres itself at its own size, with `icon_menu_button()` and `dress_icon()` beside it; the main menu, the composer's detach, label and recipient removes, its formatting toolbar and the notice menu moved to them; `icon_buttons::*` walks every surface.
- [X] T203 The open message's reading leftovers, decided with `/gtk-design`: plain-text paragraph gaps too wide; the body face (Barlow) reconsidered for long reading; opening find must not scroll the column to the top; the column's ground is a token, not a hard-coded colour; and the measure capped near 75 characters now the column fills the dialog (T197) Done: a 32em measure at the column's left edge (~75 characters), a blank line parts paragraphs with a 0.8em gap, Barlow kept (reasons in `screens.md`), `mod+f` opens find above the column without moving it, and the ground is `--postio-surface` read through a probe; `open_measure::*`.
- [ ] T204 `rule_query::digest_mail_like_this_present_with_a_message_says_so_with_no_model` fails about one full run in three under load and passes alone: find what it waits on by time rather than by condition and make it wait on what a person would see Investigated, not fixed (2026-09-30): the test waits on its condition already (`settle_until`, no fixed wait) and the host answers `Ok(None)` at once with no model; 24 parallel copies and 3 full runs pass here, so the ~43s failure is the 40s patience (10s x POSTIO_TEST_PATIENCE=4) running out under load. Three suites run at once failed other cases the same way (`keymap::every_key_the_key_map_shows_runs_its_command`, `open_keys::j_and_k_step_the_open_message_from_where_the_keyboard_is`, `open_keys::the_arrows_and_paging_keys_scroll_the_open_message_in_steps`: 'body never drawn'), so it is machine contention on the shared compositor, not this case. One src hazard seen: `RuleDialog` has one error label that `read_preview` and `like_this` both write, so a preview error would replace 'nothing alike'. `pickers::h_then_3_reminds_at_the_end_of_the_week` also failed once in 3 full runs at 1.3s and passed alone.

**The message dialog redesign** (the maintainer's Claude Design handoff, 2026-10-01). Its spec and reference renders stay in the maintainer's local, untracked `Design/focus-message-dialog/` (`SPEC.md`, `screens/`), never copied into the repository; the numbers each task needs are below. It supersedes T197's full-width column and T203's left-edge measure.

- [X] T205 The dialog's size comes from the window only: `clamp(640, W - 2*max(96, 0.18*W), 820)` wide (1024 -> 656, 1280 -> 819, 1440/1920 -> 820), window height - 80 tall, centred, radius 12, the list dimmed behind (black at 20% light, 45% dark). Recomputed on resize, never on `j`/`k`. A pure function, tested at those widths Done: `postio_ui::focus_dialog::dialog_width`/`dialog_height`, refitted from the window's own size on its surface's `layout` (the surface's includes a restored window's shadow); 1024 gives 655, the formula's (and the mockup's `Math.round`), not the prose's 656, which no rounding reconciles with 1280's 819. Dimming and the 12px corner are `floating-sheet` rules, so every Focus dialog keeps one pattern (FR-092); the dark scrim needed the provider's `prefers-color-scheme` to follow AdwStyleManager (`style::install`). `open_layout::the_dialog_is_sized_by_the_window_and_never_by_the_message`, `the_list_behind_is_dimmed_by_black_at_20_and_45_percent`
- [X] T206 The chrome at full dialog width: a 52px header bar (k/j steps left, subject Barlow 600 14.5px centred over "Message 5 of 60 · thread of 6" in Plex Mono 11px, close right) and a 44px action row between hairlines (Barlow 500 13.5px buttons with keycaps). Below 760px, Label, Move and Delete collapse into More `.`, a registry command rather than a local menu Done: More is `more_actions` on `.` (Reader context; registry, golden, contract), its menu in the row menu's dress; it folds below 760px or whenever the full row would not fit, because Adwaita Sans sets the nine verbs wider than Barlow (verbs padded 6px a side, not 8, so they fit 819/820). `open_layout::a_narrow_dialog_folds_label_move_and_delete_into_more`
- [X] T207 One centred content column for everything inside the message, sharing both edges: `min(480, dialog - 96)` for `Treatment::AppColours`, `min(640, dialog - 48)` for `Treatment::Paper`. A paper layout wider than its column is zoomed to fit, with sideways scroll only below 0.85. A pure function for the width, and a test that thread marker, subject, labels, sender block, action card, body and attachments share both edges Done: the column is an `adw::Clamp` at `focus_dialog::column_width(dialog, reader.treatment())`, following the treatment the renderer decided (`Reader::connect_treatment_changed`, after `O` too); edges pinned at 1280 (the suite compositor's monitor) and 1024 by `open_layout::every_block_shares_both_edges_of_one_centred_column`. Paper zoom end to end (2026-10-01, integration): a 640px newsletter in the 607px paper column at 1024 is fit at 0.95 with nothing sideways; a 900px page stops at `PAPER_FIT_FLOOR` (0.85) and scrolls sideways -- a flowing body now configures its own sideways adjustment and takes a sideways scroll (a touchpad's, or Shift with the wheel), which it did not, so such a page was cut off. `open_layout::a_page_on_paper_is_zoomed_to_its_column_and_scrolls_sideways_below_the_floor`
- [X] T208 The vertical rhythm, by explicit spacing: action row -> thread marker 28; marker -> subject 12; subject (Barlow Condensed 600 30/34) -> labels 10; labels (24px chips) -> sender block 16; sender block padding 12/12, rows 22; sender -> action card 12; action card -> render-mode line or first body line 24; render-mode line -> body 12; paragraphs 12; list items 4 apart, 20 indent; sign-off -> attribution line 20; that line -> the 28px quote toggle 4; body -> attachments 24, a hairline, 16, chips 8 apart; bottom padding 32. An absent block takes its gap with it. Tests pin each gap Done: the gaps between blocks are `focus_dialog::rhythm`, set in code (the reader's attachments row in `focus.css`), and the reader's empty notice slot no longer reserves a notice's height in flow mode; pinned by `open_layout::the_blocks_keep_the_handoffs_rhythm` and `an_absent_block_takes_its_gap_with_it`. Over an HTML body the render-mode line takes the card's 24 and keeps 12 to the body (`open_layout::the_render_mode_line_sits_24_under_the_card_and_12_over_the_body`). Inside the body the rhythm has one home, `postio-ui/data/treatment.css` (the column's flow sheet only drops the body's frame): paragraphs 12, lists 4/20, no gap under the last block (`open_measure::the_body_ends_at_its_last_line`). The attribution line is marked by the body pipeline (`postio_body::quote::ATTRIBUTION_CLASS`: plain text's line before a fold ending in a colon, and HTML's block holding it) because the renderer does not parse `:has()`; it sits 20 under the sign-off, muted at 13.5/20, the 28px toggle 4 under it (`open_measure::the_attribution_sits_between_the_sign_off_and_the_toggle`). Plain-text lists are real lists: a paragraph's run of two or more `- `/`* `/`• ` items, or consecutive numbers, with indented wrapped lines joining their item; a single dashed line, `-- `, `-5` and numbers out of order stay text (`open_measure::a_plain_list_draws_as_a_list_with_the_handoffs_rhythm`)
- [X] T209 The components: the sender block is hairlines above and below with no box, a 44px label column (13px muted), name Barlow 600 with address in Plex Mono 12.5, date right in Plex Mono 12; the action card is the only filled element (radius 8, accent at 8% light / 12% dark, min height 48, padding 8/8/8/12; outlined tag, Plex Mono date, italic quote, Snooze `s`, Dismiss `-`); keycaps Plex Mono 10.5, 16px tall, 1px inset hairline, radius 4, muted, 6px after the label; label pills 24px with a hairline and an 8px dot; attachment chips 40px, hairline, radius 8, name Barlow 13.5, size Plex Mono 11.5. Colours as tokens: surface #fdfdfb / #1c2120, behind #f5f5f2 / #141817, ink #1c2321 / #e7ecea, ink 2 #4a5350 / #b7c0bc, muted #68716d / #909a96, hairline rgba(24,34,31,.09) / rgba(255,255,255,.08), strong .16 / .14, accent #0d7068 / #5fc4b5 Done: the palette is the dialog's values of the existing roles plus `--postio-accent-soft` and `--postio-scrim` (`focus-colours.css`, scoped to `.focus-open`, light and dark by media query), the components in `focus.css`. Faces stay Adwaita Sans and Adwaita Mono (FR-093), at the handoff's sizes; the subject is Adwaita Sans 600 at 30/34, not Barlow Condensed. `open_layout::the_dialog_wears_its_palette_in_light_and_dark`
- [X] T210 Sanitise every HTML body before it reaches the view (scripts, forms, event handlers, remote content), and classify it with a pure function in `postio_body::treatment`: `Paper` if, after sanitising, the HTML paints a page background (on body, a wrapper table, or most of the content), has a fixed-width layout table >= 480px, or an image wider than 300px; otherwise `AppColours`. Unit tests over sample mail added to the corpus (`/add-fixture`): plain text, Outlook-style work mail with black text, a Gmail reply chain, a newsletter, a receipt Done: `postio_body::treatment::classify(&Sanitized)` and `paper_trigger` (the trigger, for the line's words) -- `&Sanitized` rather than the HTML alone, because the page's background is lifted off `<body>` into `canvas` and a `body` rule into `styles`; a near-white page (relative luminance >= 0.95) is correspondence, not paper. The sanitiser already stripped scripts, forms, handlers and remote content; `body_suite::treatment` holds it to that on the hostile corpus. Fixtures added: `html-work-black-text`, `html-gmail-reply-chain`, `html-newsletter-own-page`, `html-receipt-fixed-width` (plain text is `plain-text-simple`); unit cases for each trigger.
- [X] T211 App colours: strip `color`, `background`, `background-color`, `bgcolor`, `font-family`, `font-size`, `line-height` and `<font>` attributes; keep bold, italic, underline, headings, lists, blockquotes, tables, links, inline images; inject the app's stylesheet (Barlow 15/24 in ink, links in accent, tables with strong hairlines and 6/10 cell padding, images max-width 100%). An inline colour deliberately kept survives only at 4.5:1 against the current surface, else ink Done: `treatment::app_colours` and `app_colours_css` strip the listed properties and attributes (and `background-*`, the `font` shorthand, paragraph margins and `&nbsp;` spacer paragraphs), mark a data table `postio-grid`, and keep a chromatic colour on an inline element for the guard; `postio-ui/data/treatment.css` draws Barlow 15/24 in ink, links in accent, grids in strong hairlines with 6/10 padding, and the body rhythm (12, 4/20, 20, 4, toggle 28) from `var(--r-*)`, which the open message's column supplies from its own tokens through probes (`BodyView::add_palette_probe`, `reader::view::FLOW_PALETTE`). The guard is `postio_render::theme::guard`, applied by the render plan to every run of an app-colours body: kept at the floor (4.5, 7 in high contrast), else the container's ink.
- [X] T212 Paper: rendered exactly as sent on a white sheet with `color-scheme: light` and a white view ground, radius 6 and a hairline edge; in dark mode the sheet is dimmed with `brightness(0.92)`; never inverted or recoloured. The same decision in light mode Done: paper is drawn as sent on a white sheet, radius 6, a hairline edge as an outer box-shadow (so it takes no width), the light palette inside, the sender's `prefers-color-scheme: dark` rules dropped (`treatment::light_only`); the render plan never touches a paper body. Blitz equivalents: `brightness(0.92)` is an 8% black veil over the sheet (`::after`, rgba(0,0,0,0.08)) -- the same per-pixel arithmetic with no filter pass; `color-scheme: light` is the light palette and a white ground on the sheet. A paper layout wider than its sheet is zoomed to fit down to 0.85 (`postio_render::render::PAPER_FIT_FLOOR`, a second layout at the scaled zoom; `RenderedDocument::fit`, `Reader::paper_fit()`), and keeps its tables and images at their own width so the fit, not a squeeze, makes it fit.
- [X] T213 A quiet render-mode line above an HTML body (12.5px, muted), e.g. "App colours · sender colours and fonts removed · Show original ⇧O"; `⇧O` switches the open message between the two (a registry command); "Always for this sender" is stored with the other per-sender settings Done: `switch_treatment`, bound to `O` in the reader, Focus only (golden table and `docs/keybindings.md` updated); `Reader::use_treatments()` (Focus's open message), `switch_treatment()`, `remember_treatment()`, `treatment()`, `treated()` and `connect_treatment_changed()`; the line is `reader::render_mode::RenderModeLine`, its words `postio_ui::reader::document::render_mode_words`. "Always for this sender" is stored in the remote-image allow list's key file under `[Treatment]` (address = app or paper): view preference, not the store's, so no migration. Logs say which treatment, never whose mail. `focus_suite::treatments` drives it through real key delivery.
- [ ] T214 Screenshots at 1440x900 and 1024x768, light and dark, compared with the handoff's screens 01-14; differences listed in the task's note Integrated (2026-10-01, both lanes and the maintainer's decisions C25 and C26): `shot` screens 04 (plain) and 27 (newsletter on paper) at 1440x900 and 1024x768, light and dark, against the handoff's 01-04, 07, 08 and 10-13 (the rendering lane's 28 and 29 against 11-13 earlier). Matching now: the dialog 820 wide (655 at 1024) with the list dimmed; one centred column -- 480 for 04, 640 for paper at 1440 and 607 at 1024, where the 640px newsletter is fit at 0.95 edge to edge; the 52px header and 44px action row between hairlines, More `.` at 1024, Delete's cap `Del`; the gaps of 05 including card -> render-mode line 24 and line -> body 12; the action card's sentence wrapping uncut (three lines in 04's narrow card, as 01); the plain list as bullets under a 20px indent 4 apart; 'On Monday, ... wrote:' muted and smaller, 20 under the sign-off; the paper sheet's radius, hairline edge and 92% dimming in dark; the handoff's surface, ink and rules in both schemes; `O` leaving the column where it was. Accepted differences: (1) the system font, Adwaita Sans and Mono, in the chrome (C25), so the subject is Adwaita Sans 600 30/34 rather than Barlow Condensed and the verbs are padded 6px a side to fit 820; (2) the system accent (C26) for the card's fill, its tag and the body's links (the suite compositor's default accent is a teal close to the handoff's, so the shots look alike). Differences that remain, none of them the two accepted: (a) at a 900px window the dialog is 810 tall and 45px down, not 820 and 40: libadwaita's floating sheet keeps 5% of the window above and below (exact at 768: 688); (b) keys as the keymap spells them, `O` not `⇧O` and `E` not `⇧E` (C22); (c) the k/j steps are icon buttons with their caps beside them, not one button each; (d) dates follow T184, 'Today, 15:22' not 'Sat 26 Sep, 15:22'; (e) the bulk bar at the window's foot is not dimmed by the dialog, being outside the dialog host (pre-existing), and keeps its own `Delete` cap; (f) the render-mode line's mark is the U+25D1 glyph, not the handoff's drawn half-circle; (g) office mail on paper (screen 29) falls back from Calibri, which is not installed, to the renderer's sans; (h) the shot refiles screen 04's row for 27, so the newsletter keeps a thread marker, the Harbor label and a to-do card the handoff's 03 does not have. Not ticked: (a) to (f) are differences from the drawing that are not the two the maintainer accepted; (b) and (d) follow recorded decisions, and (a), (c), (e) and (f) want a decision on whether to chase the drawing.
- [X] T218 A body that falls back to plain text ("Shown as plain text: this message took too long to lay out") draws in a narrower, inset column and another face than every other body: the fallback takes the same column, face and rhythm as an app-colours body. And find why a ordinary newsletter hit the layout time limit at all; fix the cause if it is ours. From the maintainer's walk (2026-10-01) Done: the reader composes the fallback as a plain-text body (`RenderRequest::fallback`), laid out glyph for glyph where the same words sent as plain text are (`widgets_suite::body_view_fallback`); in Focus the render-mode line says "Plain text · this message took too long to lay out" and the body carries no notice, in the classic reader the notice is a quiet line above the body. Three causes of ours fixed: a request queued behind a superseded render (Focus asks twice as a message opens), app colours laid out twice in dark for a mark per `<img>` (now `treatment.css`), and a finished render replaced when the main loop came back after the deadline. Fixture `html-newsletter-many-tables`; shot screens 30 and 31.
- [X] T219 The open message's step controls (up/down arrows each with a k/j keycap) take too much of the header bar: a compact form that still teaches the keys. From the maintainer's walk (2026-10-01) Done: one linked pair, each step a quiet button with its chevron and its cap inside (Focus's rule: a key is taught inside the control it runs), about 87px where it was about 138; the cap is now the button's shortcut for a screen reader (`focus_suite::open_reading::the_steps_carry_their_keys_inside_and_stay_compact`).
- [x] T220 While the first sync is filling the inbox the list says the inbox is empty: during a sync that has not finished its first pass, the empty state says it is syncing (and how far), never "empty". From the maintainer's walk (2026-10-01) Done: `inbox_saying` (postio-ui) decides; the engine now says a pass finished.
- [X] T221 The composer redesigned with `/gtk-design` and `/ux-architect`, the header's controls first (title, detach, Send later with its long keycap, the menu arrow, Send), consistent with the message dialog's chrome (T206, T209). From the maintainer's walk (2026-10-01). Done 2026-10-01: the message dialog's size, header and action row with Send first, short caps, one 480 column with the body on the dialog's surface, no footer (screens.md, "The composer"; `compose_layout`)
- [X] T217 The Postio icon on Focus: a running Focus is drawn with the package's one icon (`postio_gtk::app::ICON_NAME`, `dev.postio.Postio`, which the desktop entry's `Icon=` names and the Flatpak installs) Done: `postio_widgets::style::install_icons` puts the icon on the display's theme from `postio-widgets`' GResource, built from `postio-gtk`'s own committed SVGs (one file, no copy), and `FocusWindow` sets it as the default icon name; the desktop Flatpak already installs the icons and Focus's entry, so it needed no change. `scripts/run-isolated.sh --focus --install-desktop` installs the entry (Exec= on the isolated binary) and the icon under `~/.local/share` for a plain cargo run, opt-in. `focus_suite::desktop::focus_shows_the_postio_icon` and `packaging::the_entry_is_named_after_the_id_and_its_icon_ships_in_the_package`.
- [X] T222 Responsive newsletters lose content: a `@media (max-width:600px)` rule that makes table cells `display:block` applies in the 480px app-colours column, and Blitz then drops every such row (text and images). Fix it in the engine patch queue (`patches/blitz`, never upstream), with an invented fixture and a render test that the stacked cells' text is drawn. Found by T218 Done: `patches/blitz/0002` wraps a row's non-cell children in one anonymous cell (CSS 2.1 §17.2.1), where they stack; `engine_patches::a_cell_made_display_block_is_drawn_and_stacks` and `treatment::app_colours_draws_the_stacked_cells_of_a_responsive_newsletter` on `html-responsive-stacked-cells`.
- [x] T223 Focus sanitises, classifies and treats every body on the main thread (`render_open` -> `body_html_treated`, 70-200ms per newsletter in dev): move it off the main thread as the classic reader's prepared path does, with a test that opening a message does no body preparation on the main thread. Found by T218 Done: `open.rs` prepares on `gio::spawn_blocking` (`prepare_treated`: sanitised once, drawn in both treatments), superseded by `j`/`k`; `focus_suite::treatments::opening_and_stepping_prepare_no_body_on_the_interface_thread`
- [X] T215 A store whose schema changed under it opens: migrated in place when a recorded step reaches its stamp, otherwise refused with a way forward that is not "Try again" -- a fresh store that sets the old one aside and keeps the accounts and config.toml -- in the window and as a CLI Done: `postio_storage::schema::MIGRATIONS` chained by fingerprint (first step: the rebase's three indexes, 3f95ddb1 -> d8c1e5df), replaced HEADs kept in `tests/schemas/`; `postio_session::{Refusal, Remedy}` and `start_over`; Focus's "Start a fresh store"; `postio-store status|reset`, `scripts/run-isolated.sh --reset-store`; `storage_suite::migrations`, `schema::tests`, `focus_suite::store_unavailable`; decision in `docs/notes/2026-10-01-store-migrations-and-starting-over.md`.
- [X] T216 The window before the inbox can be closed: its close button, `Ctrl+Q` and `Ctrl+W`, on every page that says why there is no mail Done: a top bar with the window's close button over the opening and refusal pages, removed once the inbox's own takes over; keys before the inbox resolve Quit from the keymap; `Ctrl+W` is Quit's alternate in the registry; `focus_suite::store_unavailable` clicks and presses with real delivery.

**Classification quality** (the maintainer's review, 2026-10-02: on a sample of their own inbox most to-do markers were boilerplate, real actions were missed, and nothing was filed. Fixtures are invented look-alikes of the patterns; nothing from the maintainer's mail enters the repository.)

- [ ] T224 A counts-only measurement over a real store: a local command that runs the classifier over a store (a copy, read-only) and prints counts and message ids only -- marked by kind and source, filed by reason and layer, held by rule, considered vs not by the needs-action guard -- never subjects, senders or text. The before/after for T225-T228 is measured with it
- [ ] T225 Automated senders without List-Id: transactional mail (banks, utilities, billing, confirmations) carries none of List-Id, List-Unsubscribe or Auto-Submitted, so the needs-action guard treats it as a person writing. Add header and address signals -- `Feedback-ID` and other ESP headers, no-reply style local parts, bulk sending subdomains -- so such mail is not considered for person-style markers. Test with invented transactional fixtures
- [ ] T226 The detector ignores boilerplate: text below a disclaimer, signature or footer marker (confidentiality notices, "if you are not the intended recipient"), conditional instructions ("If you set up...", "If you would like..."), and statement or portal pointers ("please view your statement", "please visit the ... page"). Precision tests for each, held to SC-013
- [ ] T227 An "action required" marker for automated mail, from a few strong patterns -- an invoice or payment due with an amount, an overdue notice, "complete your ..." before an appointment, a reminder to update something -- shown as a to-do with its source, distinct from a person's ask. Needs a `/ux-architect` call on how it reads beside a person's to-do
- [ ] T228 Filing and digests: find why mail already in the inbox was not filed, then file machine reports (DMARC aggregate reports and the like) as notifications, List-Id newsletters into digests, and unsubscribe-offering marketing as promotions; repeated identical posts from one sender collapse. Measured with T224

**One app: Focus, and the classic app retired** (the maintainer, 2026-10-02: "start the process of retiring the old gtk app", after weighing one app with two modes against two apps. Focus is the app; the classic three-pane GTK app is retired rather than kept as a mode.)

- [X] T229 Record the decision: an ADR (it outlives this feature and other work must obey it -- no new work in `postio-gtk` or the `postio` binary beyond keeping them building until removal), and a row in `spec.md`'s decisions. Say what was rejected (two apps; one app with two modes) and why, from the session's evidence: fixes landing twice or only once, shared widgets rippling, two app ids Done: ADR 0043, C27
- [X] T230 A parity audit: every user-facing capability of the classic app (`postio-gtk`, `postio-app`) -- commands in the registry offered only to the classic frontend, surfaces, settings, accounts and folders management, search, threading views, printing, attachments, offline states -- checked against Focus, written as a table in `specs/007-postio-focus/` with each gap either a new task, "covered by", or "dropped, because". Nothing is removed before this table exists Done: `classic-parity.md`, T233-T248
- [X] T231 The retirement plan as ordered tasks from T230: what moves to shared crates first, the point at which the flatpak and desktop entry switch to Focus as the one app (its id, the launcher, the icon), the CI and test suites that go with the classic app, the docs (keybindings frontends, README, PRODUCT.md), and the final removal of `postio-gtk` and the `postio` binary. Removal itself waits for the maintainer Done: `classic-parity.md`, T249-T256
- [X] T232 The open message beside the list: a layout where Enter shows the message in a pane next to the inbox list instead of the dialog, the list keeping its cursor and `j`/`k` stepping the pane. A setting and a command switch between dialog and pane; a window too narrow for both falls back to the dialog. Designed with `/ux-architect` and `/gtk-design` against the message dialog's spec (the column, rhythm, components, treatments carry over), test-first Done: screens.md, "Reading beside the list"; `F8`, `[focus] reading`; `reading_pane`
- [X] T233 Settings, part one (`classic-parity.md` rows 42, 44, 49): move the settings window (`postio-gtk::settings` and the widgets it alone draws: `checkrow`, `settings_group`, `nav_row`) and its presenters (`postio-app`'s `settings_accounts`, `settings_credential`, `settings_egress`, `settings_privacy`, `sidebar_backfill`) into `postio-widgets`, the presenters as `present::settings` over `postio-client` (ADR 0031: the model stays in `postio_ui::settings`). The classic app opens the moved window, and its behaviour does not change. Proof: the classic app's `settings_*_wiring` cases stay green unchanged, and `check-crate-boundaries.py` passes Done: `postio_widgets::settings` and `present::settings`; classic suites green
- [X] T234 Settings, part two (rows 42, 44, 49, 50): Focus opens the settings window from the main menu's Settings and `mod+comma`, which `FocusWindow::act` does not answer today, in Focus's frame (C25, C26; `/gtk-design`). Sync and storage gains a per-folder backfill exclusion (ADR 0016), because Focus has no sidebar to carry it. Appearance shows no `[ui]` key Focus does not honour (rows 18, 19). Test first: `focus_suite` cases for the menu item and the key, and ports of `settings_accounts_wiring`, `settings_credential_wiring`, `settings_reindex_wiring`, `signature_default_wiring`, `egress_wiring`, `read_receipt_wiring` and `sidebar_backfill_wiring` Done: a dialog in the message dialog's frame (screens.md, "Settings"); screens 40-43
- [X] T235 Edit configuration and live config (rows 51, 52): `EditConfig` (`mod+e`) opens `config.toml` in the person's editor, as `postio-gtk::config` does. Move the launcher to `postio-widgets` and answer the command in Focus. `Session::follow_config` also applies `[storage]` (the ceiling, `postio_host::maintenance::enforce_ceiling`), `[compose]` and `[reader]`, which today wait for a restart. Test first: a `reload` case per section, porting `storage_ceiling_wiring` Done: `postio_widgets::editor`; `[compose]`, `[reader]`, `[storage]` follow live
- [X] T236 Every command Focus offers is answered (row 5): `g s` (Sent), `g r` (Archive), `g z` (Snoozed) and `g *` (Flagged) draw their caps in the folders popover and do nothing, because `FocusWindow::act` has no arm for them. Answer them as `go_to_drafts` does, and add to `registry_parity` the assertion that every offered command not in `NOT_YET` reaches a handler, so the "no Focus surface answers this command yet" fallthrough cannot hide a gap again. Test first: the four keys through real delivery Done: `g s`, `g r`, `g z`, `g *` answered (folders by role, Snoozed and Flagged as view scopes); `A` answered too; `registry_parity` runs every offered command outside `NOT_YET` and fails on any `act` fallthrough
- [X] T237 Reading a message marks it read (row 33): Focus never sends `MarkReadOnDwell`, so a message opened and read stays unread. With `/ux-architect`, decide when it counts as read: at open, or after `[reader]`'s dwell delay in the dialog and in T232's pane. Wire that, keeping FR-016 (moving the cursor marks nothing). Test first: open, wait, and the row is no longer bold; `j`/`k` past a message faster than the dwell leaves it unread Done: open for `DWELL_TO_READ`, in the dialog or the pane, marks it (`MarkReadOnDwell`); stepping on, closing or `r` stops the clock; `r` in the open message is the way back (screens.md, "Reading marks it read")
- [X] T238 Unsnooze (row 16): `B` and the row menu on a snoozed conversation, in the Snoozed mailbox and in the open message, with an undo toast. Focus offers `Unsnooze` (in `NOT_YET`), but nothing answers it. Test first, and take it out of `NOT_YET` Done: `B` and the row menu (Snoozed list only), in the list and the open message, with the Unsnooze toast and Undo; a conversation row wakes its sleeping messages
- [X] T239 Sending states (row 45; ADR 0021 Decision 3): Focus's rows in Outbox and Drafts draw a message's draft state (queued, stopped, not confirmed), and the open message offers Cancel send, Retry send and Mark as sent where they apply. Today `open.rs` builds its reader with `Verbs::NONE`. Test first: port `unconfirmed_send` and `resume_queued_draft` to `focus_suite` Done: the row and subtitle say the state; a draft not being written opens to be read, its action row `focus_dialog::send_verbs`, in the dialog or the pane; `g o` lists the Outbox while it holds anything (screens.md, "Sending states"; `sending_states`)
- [x] T240 Saving attachments (row 37; `docs/PRODUCT.md` §11 "save as"): the `o` chooser offers Save and Save all through the file-chooser portal, and activating an attachment chip in the open message opens the same chooser at that part (wire `Reader::connect_attachment`). Test first: a part saved byte-identical to a chosen path, and a click on a chip shows the chooser. Done: Save and Save all in the `o` chooser through the portal (`set_file_picker` seam), the chip opens the chooser at its part; `save_attachments`
- [X] T241 Search, what the bar drops (row 40): show `SearchResults::instead`, "Search instead for …" (ADR 0037), as a row the bar can run, and let the results switch between relevance and date (`ToggleResultOrder`, `O` while results are listed), where `bar.rs` asks for relevance only. Test first: port `search_instead`, and a case where `O` reorders the rows
- [x] T242 Zoom (row 28; spec 006 FR-021; `docs/PRODUCT.md` §20): `mod+plus`, `mod+minus` and `mod+0` in the open message (and T232's pane), kept in `[reader] zoom` and applied to the next message. The paper treatment's fit-to-column multiplies under it. Test first: port `zoom_persists`. Done: `act` answers the three keys in dialog and pane; `postio_config::save_zoom` (shared with the classic app) keeps the level; `reader_zoom`, `open_layout::paper_fit_multiplies_under_the_zoom`
- [x] T243 The sync-window step (row 47; #876): after an account is saved, Focus's first run asks how much history to sync, as the classic app's does (`Status::SyncWindow`, `write_sync_window`). Move the step from `postio-app::onboarding` into `postio_widgets::present::onboarding`, so both apps run the same one. Test first: port `sync_window` to `focus_suite::first_run`. Done: `Presenter::ask_sync_window` (widgets) is the step both first runs use; Focus asks it when it has no account (`add_account_asking_history`); the classic test still covers its own `install`
- [x] T244 `mailto:` (row 46): Focus opens a composer from a `mailto:` URI, through its existing `HANDLES_OPEN` (`postio_model::mailto`, as `postio-app` does). The desktop entry's `MimeType` moves with T253, not before, so the two launchers never claim it at once. Test first: port `mailto_uri`. Done: `FocusWindow::open_link` routes `mailto:` to `Compose::open_mailto`, held until the composer is mounted on a cold launch
- [x] T245 Dragging messages out (row 22): Focus's rows offer selected messages to a file manager as `.eml` files, lazily. Move `postio-gtk::drag_out` into `postio-widgets` and `postio-app::export` into a presenter over `postio-client`. Test first: port `drag_out_wiring`. Done: `postio_widgets::drag_out` and `postio_widgets::present::export` are the shared halves (the classic list uses them); Focus's rows drag through `FocusWindow::drag_offer`, a select-all offers nothing
- [x] T246 Window size remembered (row 10): Focus keeps its size and maximised state across a restart, best-effort, in `$XDG_STATE_HOME/postio` as `postio-gtk::state` does. A missing or unreadable file opens at the default. Test first. Done: `postio_widgets::state::Geometry` is the shared half (`postio-gtk::state` reads and writes its size through it); Focus saves on close
- [x] T247 Remote images allowed, proven in Focus (row 31): in the open message, the shared banner's Show and Always (`ShowImages`, `AlwaysShowImages`) fetch through the app, and "always" holds when the sender's next message opens. Nothing is requested before the click. Test first, extending `focus_suite::remote_images`, and take both commands out of `NOT_YET`. Done: `i i` and `i a` reach the shared banner through `act`, both out of `NOT_YET`; `remote_images` proves Show once and Always holds
- [X] T248 A failing account is named (row 24; ADR 0005 Q10): when one account's sync is failing for a reason other than its password, Focus's banner names the account and the reason and offers Retry, and the inbox of the accounts that work stays usable. Today the sync label says "Sync failed" and nothing else. Test first in `postio_ui::focus_state`, then a `focus_suite::state` case porting `degraded_unified`
- [X] T249 Move first, the build data (the retirement, step 2): the design-token generation (`postio-gtk/build.rs` -> `postio-widgets/data/metrics.css` and `tokens.css`), the icons that `postio-widgets/build.rs` reads from `../postio-gtk/data/icons`, and the desktop entry and metainfo the Flatpak installs from `crates/postio-gtk/data/` move to `postio-widgets` or `postio-gtk`. The classic app reads them from their new home. Proof: the Flatpak manifest and `postio-gtk/tests/packaging.rs` name no path under `crates/postio-gtk`, and Focus's icon test still passes Done: `postio-widgets/build.rs` generates `metrics.css`, `space.rs` and `reader-tokens.css` (`postio-gtk/build.rs` keeps only its own `tokens.css`); the icons are `postio-widgets/data/icons`, the desktop entry and metainfo `postio-gtk/data/`; `packaging::nothing_focus_ships_is_read_from_the_classic_crate`
- [X] T250 Move first, the instruments (step 2): the startup timeline (`postio-gtk::startup`, `POSTIO_STARTUP_TRACE`) and the jank detector (`postio-gtk::jank`) move to `postio-widgets`, and Focus records its startup phases against the 500 ms budget. `postio-bench`'s `list_scroll` and `composer_open` move to Focus's row and composer frame; `conversation_rows` goes with the classic app. Test first: a `focus_suite` case that the timeline reaches its last phase Done: `postio_widgets::{startup, jank}` (postio-gtk re-exports them); Focus marks window, shell, store, account, feeds and first frame (`startup::time`, `startup_timeline`), installs the jank probe and notes each command; `list_scroll` and `composer_open` time Focus's row and composer; `gtk_jank` is `widgets_suite::jank`
- [X] T251 What only the classic app exercises (step 3): list every `pub` item in `postio-ui` and `postio-widgets` whose only callers are `postio-gtk` or `postio-app` -- the reader's conversation and rail seams, `Verbs::STANDARD`, `list_state`'s aggregate rule, the `ComposerHost` of `postio-gtk::composer`, among others. Decide for each whether Focus or the terminal will use it (it stays, with a test from that caller) or whether it goes with the classic app. Write the list into `classic-parity.md`'s risks Done: `classic-parity.md`, "Shared code only the classic app calls"; `ComposerHost`, the detach seam and `postio_ui::settings` stay (Focus calls them)
- [X] T252 What only the classic suites prove (step 3): for every case in `postio-app/tests/app_suite`, `e2e.rs`, `oauth_signin.rs`, `backend_choice.rs` and `postio-gtk/tests` (`gtk_suite`, `logic_suite`, `gtk_reader`, `gtk_composer`, `gtk_accessibility`, `drag_out`), decide whether to port it to `focus_suite`, the `postio-widgets` suite or the host's suite, or delete it with the classic app because it proves a classic surface. Host and renderer behaviour (`reader_spawns_no_web_process`, `hostile_mail`, `reclaim_wiring`, `startup_repair`, `event_fanout`, `notify_off_the_main_thread`, `e2e`) is ported. Record the decision per case in `classic-parity.md`, and port the cases Done: `classic-parity.md`, "What only the classic suites proved"; host behaviour in `postio-host/tests` (`e2e`, `oauth_signin`, `backend_choice`, `reclaim`, `search_index`), the composition root in `focus_suite`, shared-crate cases moved to their crates' suites; four Focus gaps held out as T261-T264
- [X] T253 The package switches to Focus (step 4; after T233, T234, T236, T237, T239, T243, T244 and T249): Focus takes the app id `dev.postio.Postio` (`postio_gtk::app`), its desktop entry becomes `dev.postio.Postio.desktop` with `MimeType=x-scheme-handler/mailto;x-scheme-handler/postio;`, and the metainfo describes Focus. `flatpak/dev.postio.Postio.json` builds and installs Focus only, with `command` set to it. The classic `postio` binary leaves the package and still builds from source until T256. Rewrite `packaging.rs` for one launcher. The name and the binary name follow the maintainer's answer in `classic-parity.md` (question 2). Test first: `packaging.rs` The app is named "Postio", its binary `postio` (ADR 0043) Done: `postio-gtk` builds `postio` under `dev.postio.Postio`, titled Postio; one entry (`mailto:`, `postio://`), the metainfo describes Focus (the classic screenshots dropped until the site has Focus renders); the Flatpak and release build and check it alone. The classic app builds as `postio-classic` under `dev.postio.Postio.Classic`, installed by nothing
- [X] T254 CI and scripts follow Focus (step 5; after T253): Focus's and `postio-widgets`' GUI suites get the CI job `test-gtk` and `test-app` have, under the nightly and the release gate, with `full-suite-crates.sh`'s `SLOW` list and `.config/nextest.toml`'s GUI profiles (which name `gtk_suite` and `app_suite`) measured and updated. `scripts/run-isolated.sh` (Focus by default), `install-local.sh`, `screens.sh`, `appearance.sh`, `ci-changes.sh` and the release workflow's size comparison (`postio-tui` against `postio`) follow. The classic jobs stay, building and testing, until T256. Coverage floors are unchanged: GUI crates have none (`scripts/coverage-floors.json`) Done: `Tests (GTK widgets)` runs `postio-widgets` in two shards and `Tests (application)` runs `postio-gtk`, on every pull request; `SLOW` and the nextest profiles name them; `appearance.sh` retired (it shot the classic window), `screens.sh` was already `storyboards.sh screen`
- [X] T255 The docs say one app (step 6; after T253): README (install, run, `cargo run -p` Focus), `docs/PRODUCT.md` (§2 Platforms, §5 Threading, §9 Layout, §10 Compose, §23 v1, with the flag answer), `docs/ARCHITECTURE.md`, `docs/config.md`'s `[ui]`, `docs/keybindings.md`'s Where column (through the registry, not by hand), the spec's Packaging assumption, US11 and Out of Scope, and `CLAUDE.md`'s build and test guidance, skills and hook (the list in `classic-parity.md`, "Notes for T255"). The constitution's Scope and boundary paragraph change in the amendment that lands with the branch, which needs the maintainer Done: CLAUDE.md, README, PRODUCT, ARCHITECTURE, the user guide, `config.md`'s `[ui]` and `keybindings.md`'s Where column (through their generators), the skills and the hook say one app; the spec's Packaging and US11 are current; the constitution is amended to 1.4.0 (approved 2026-10-06: one product, several interfaces, each separate from the engine)
- [X] T256 Remove the classic app (step 7; approved by the maintainer 2026-10-02), after T233-T255, T261-T265: delete `postio-gtk`, the `postio` binary and `postio-app`'s GUI with its suites and examples, and the workspace entries. `Frontend::Classic` is removed, or the classic default in `Availability::open` is replaced; `Requirement::ThreePane` is narrowed to macOS (the terminal left it in T300); `[ui]` keys no app honours are removed; and `check-crate-boundaries.py` and every check naming `postio-gtk` are updated. ADR 0043's rules for the time before removal go with this. With `postio-app` go its storyboard runner (`demo::storyboard`, `examples/storyboard.rs`, five `app_suite` cases, their `.config/nextest.toml` filters), `postio_gtk`'s `storyboard` re-export, `storyboards/gaps/classic.toml`, and `postio_storyboard::apply::App::Classic` with the tests that name it Done: both classic crates, `Frontend::Classic`, `App::Classic` and the T251 list deleted; `postio-focus` renamed `postio-gtk`; every `[ui]` key is still honoured by macOS or the terminal, so none went; ADR 0043 updated
- [X] T257 Flag in Focus (C13): Flag and Unflag on `*`, in the row menu and the command bar, local-first with Undo; no flag mark on rows; `g *` lists flagged mail. Test-first with real key delivery Done: Flag offered to Focus, `*` toggles, row menu says Flag or Unflag, in the command bar, Undo clears it; no flag mark on rows; `g *` lists flagged mail
- [ ] T258 The account verbs by keyboard in Focus (`classic-parity.md` row 49): remove, rebuild the index, set the default, enable or disable, and map a mailbox role are reached only from an account row's menu and its detail view in Settings. Their registry entries (`RemoveAccount`, `RebuildAccountIndex`, `SetDefaultAccount`, `ToggleAccountEnabled`, `MapMailboxRole`, `Context::Accounts`) are the classic app's alone (`THREE_PANE_MAIL`). Offer them to Focus with keys that act on the account row Settings has focused. Test first: each key through real delivery in Settings' Accounts section
- [ ] T259 The default signature says what is chosen: when an account has signatures and no default, Settings' "Default signature" picker shows the first signature as if it were the default. Show "None" until one is picked, and choosing one is the only thing that sets it. Test first: an account with two signatures and no default opens on "None", and nothing is written until a choice
- [X] T260 A failing account's banner says the sync's own reason: `Event::Error` names no account, so Focus's `Trackers::apply(event, None)` never puts the sync's words on that account's status and T248's banner falls back to the failure kind. Carry the account on the event (`postio-host`/`postio-runtime`), test first Done: `Event::Error` carries `account`; `Trackers` files it there
- [X] T261 Unsubscribe in the open message (`classic-parity.md` row 32): the open message offers the reader's unsubscribe notice (`Reader::set_unsubscribe`, as the classic reading pane did) and `act` answers `Unsubscribe` there, logging the activation under the message's own account. The notice shows only for list mail (a `List-Id` or a `List-Unsubscribe`, `postio_ui::unsubscribe::banner`); personal mail opens clean and `U` still leaves its list (`focus_suite::unsubscribe::personal_mail_has_no_unsubscribe_band_but_u_still_works`). Test first: take `focus_suite::unsubscribe::the_open_message_s_unsubscribe_notice_logs_it_and_the_privacy_section_lists_it` out of `IGNORED`
- [X] T262 A missing credential offers the credential form (row 48): an account with no password in the keyring (`SecretError::NotFound`, which the engine deliberately does not count as a refused password) shows the sign-in banner's "Update password…", or opens the form as the classic app's startup repair did (`postio_host::startup::route`), not "can't sync" with Retry now. Test first: take `focus_suite::startup_repair::an_account_with_no_credential_offers_the_repair` out of `IGNORED`
- [X] T263 Rows grow with the text scale (`docs/PRODUCT.md` §20): Focus's rows are a fixed 40 and 72 px (`list::row::ONE_LINE`, `TWO_LINES`), so at 200% text the type outgrows them. Scale the row heights with the text, keeping 40 and 72 at 100%. Test first: take `focus_suite::a11y_sweep::at_200_percent_text_rows_grow_with_the_type` out of `IGNORED`
- [X] T264 A destroyed window frees its composer: Focus's window and list are freed when it is destroyed, but a mounted composer, even one never opened, outlives it (suspected: `Composer` and the `DialogHost` it is mounted on hold each other). Test first: take `focus_suite::window_teardown::a_destroyed_window_releases_its_composer` out of `IGNORED`
- [X] T265 The storyboard runner (spec 008) runs over Focus: move it from `postio_gtk::storyboard`, `postio_app::demo` and `postio-storyboard` into `postio-widgets` (or its own crate over Focus's window), keep `postio_ui::observe`'s parts it needs, and re-record its 19 `gtk_suite` and 5 `app_suite` cases against Focus's surfaces, so design review keeps working for the one app (the maintainer, 2026-10-02). T256 waits for it Done: the GTK half is `postio_widgets::storyboard`; the runner is `postio_gtk::demo::storyboard` over the one demo (store halves in `postio_storage::seed`); the catalogue is re-expressed, classic-only storyboards dropped (02b8557b); the 19 `gtk_suite` cases are in `widgets_suite` and the 5 `app_suite` ones in `focus_suite`, none dropped. `scripts/storyboards.sh` and `issue-land.sh` play and key Postio only, with no `--app`
- [X] T150 Rebase onto `main`, and run the full suites the diff touches (quickstart, "Automated"). **Do not land**. Done 2026-09-29: `main` had not moved, so the rebase was a no-op. The run covered fmt, the workspace check, 4,328 engine and terminal tests, config (serial), host and app libs, 844 GTK, widget and Focus tests, `app_suite`, the two nightly measurements and `scripts/check.sh`. All green but the known load flakes (#1677, #1703, and an a11y timeout that passes alone in 1 s), plus one real fix: the settings test's "free" key had become `capture_note`

---

## Phase 17: Milestone 2: the user's own local model (P3)

- [X] T151 `crates/postio-ai`, the client of research R16. Test first, against a fake transport (no network in the default suite):
  - OpenAI-compatible chat completions with `json_schema`;
  - loopback-only endpoints;
  - client-side validation;
  - the `model` egress subsystem, a schema change;
  - the boundary rule
- [X] T152 `[focus.model]` and its validation: endpoints on this machine only, per-feature switches, and no probing. Test first: SC-016, with no section and no connection attempt
- [X] T153 [US12] The model answers the needs-action question in place of the built-in detector, and the detector answers when the model is down (FR-107, FR-170). Test first: scenario 6
- [X] T154 [US13] The digest summariser: statements with references resolved by excerpt, plain text only, the Summary tab (22), and the email from a reference (23), falling back to the list. Test first: scenarios 1–6 and SC-014. The engine half is done: `client.digest_summary(delivery)`, and `Surfaced::Digest.summary_line` re-read on `SurfacedChanged`. The Summary tab and dialog remain
- [X] T155 [US14] `list:` and query rules, and "Match a list or a search instead…". "More like this" goes through the model. Test first: scenarios 1 and 2. The engine half is done: list and query rules; `client.digest_like_this(message)`, present only when `model_for(ModelFeature::LikeThis)` is set. The command needs a registry id, and the dialog remains
- [X] T156 Compare screens 22 and 23, and record them

## Phase 18: Milestone 3: Obsidian and `postio://` (P3)

- [X] T157 [US15] `crates/postio-vault`: the Tasks line with the link before the date (spec C21), notes appended, project suggestion, and finished tasks read back. Test first: against a temporary vault, scenario 1's bytes, and nothing else changed
- [X] T158 [US15] The capture sheet (screen 25): `t` and `n`, and `Context::Capture`'s keys. Test first: a `focus_suite` case. `postio_vault::Vault` has `append_task`, `append_note`, `projects`, `suggest` and `tasks`. Wire them through a host request, and delete their baseline lines
- [X] T159 [US15] `postio://`: `x-scheme-handler/postio` in Focus's desktop file, and `open` only navigates. Test first: scenario 2, including an unknown id refused with a message. The desktop entry registers `x-scheme-handler/postio` (T144). Still to do: `HANDLES_OPEN`, and navigating from the URI. Done: `HANDLES_OPEN`, and `postio://message/<id>`, where the id is the local message id, opens that message and does nothing else. An unknown id or link is refused with a sentence
- [X] T160 Compare screen 25, and record it

---

## Phase 19: User Story 16: Focus in the terminal (P1)

**Goal**: `postio-tui` is Focus drawn in character cells ([terminal.md](terminal.md), C29). **Independent test**: User Story 16's scenarios, on rendered screens, against a fixture store with the network absent. Ids from T300 so they never collide with the desktop's (T001-T299). Each task removes the three-pane code it replaces in the same commit; nothing is kept behind a flag.

### Foundation

- [X] T300 [US16] The registry: the terminal meets `Requirement::Focus` and not `ThreePane` (`crates/postio-core/src/registry.rs` `met_by`); `SwitchTreatment` also needs `Graphical`; `postio_ui::keymap_sheet::key_map` takes the frontend. The terminal's handlers for commands it no longer offers (sidebar, panes, parts panel, folder walking, saved-search rename and reorder) go. `registry_parity`'s palette surfaces follow (Picker, Digest, Filtered, Capture in; Sidebar and Parts out), and every Focus command the terminal cannot answer yet is listed in `app.rs`'s `GAPS`, which each later task empties. Test first: a Focus row is reachable for the terminal and a three-pane row is not Done: `back_to_words` and `capture_write` gain terminal-deliverable alternates (`alt+BackSpace`, `alt+Return`); the sidebar drawing and its click handlers stay for T303
- [X] T301 [US16] One test harness for the terminal (`crates/postio-tui/src/test_support.rs`, `#[cfg(test)]` plus the `test-support` feature for `examples/shot.rs`): seed accounts, places and pages, serve fetches, open a message, answer reads, `screen(w, h)` and `hits_of`. `app.rs`'s, `view/mod.rs`'s and `shot.rs`'s copies of the seeding go as their tests move onto it
- [X] T302 [US16] Focus's engine in the terminal (FR-186): `run.rs` calls `Host::enable_focus` with `FocusSetup` after the store opens and before `start_syncing`, and again when `[focus]` changes; the shared setup and saved-search reading move out of `postio-gtk/src/startup.rs` to `postio-session` so both apps call one. Test first: the host's probe counts the call under the terminal (it asserted the opposite for T034), and `startup_budget` follows Done: the shared setup is `FocusSetup::from_config` in `postio-host` (session cannot see the host); `saved_searches` is in `postio-session`
- [X] T303 [US16] The window's layout (FR-188, FR-190, FR-191): `layout.rs` becomes the top bar, strip, banner slot, list and bottom line of terminal.md, with the overlay and pane geometry as functions of `W × H`; `view/sidebar.rs`, `sidebar.rs`'s line model, `state.rs` (reader width, folded folders), the divider and the pane cycle go. The sidebar's account and folder feed becomes the places list the command bar and `g o` read. Test first: the screen at 120×36 and 50×12 has the rows terminal.md draws and no sidebar Done: the places feed is `places::Places`; `g o` opens the finder's `#` until T319; the open message fills the body until T315; "rest then read" is gone, the body is read on open (FR-195)

### Shared policy, out of the GTK crate (FR-189)

Each moves toolkit-free code from `crates/postio-gtk` to `postio-ui` (or `postio-session` where it needs the client), with its tests, and Focus calls it from there; then the terminal is written against it.

- [ ] T304 [P] [US16] Rows: `list/item.rs` (`FocusRow`, `Digest`, `Conversation`, `surfaced`, ids, `threads`, `two_lines`), `list/model.rs`'s `day_of` and the single-heading rule, and the label merge and digest splice of `list/feed.rs` as pure functions
- [ ] T305 [P] [US16] Targets and verbs: `window.rs`'s `aims`, `reach`, `aimed_message`, `aimed_senders`, `capture_source`, `picker_target`; the `act` table's command-to-`Command` construction; and the window's sentences (no saved search pinned, a digest rule saved, removed or missing, stop digesting, unsubscribed, messages open beside the list, nothing to open, not being sent)
- [ ] T306 [P] [US16] Places and the bar: `places.rs`'s sections, ranks, `go_to`, `place_name` and the Filtered and Outbox rules; `bar.rs`'s routing (`in:`, `>`, chips only when filters), hit de-duplication and limit, headings, order and held-place words; `names.rs`
- [ ] T307 [P] [US16] Pickers and frames: the label picker's applied rule and create flow, the move picker's destinations, Archive-first order and Recent, the pickers' typing rule; `open.rs`'s `TOOLBAR`, `SEND_TOOLBAR`, `FOLDED`, position and thread-chip words and thread stepping; `bulk.rs`'s `ACTIONS`; `chrome.rs`'s strip words; `compose/frame.rs`'s words
- [ ] T308 [P] [US16] Digests, Filtered and capture: `digest.rs`'s page states, title, rule line, topic grouping and reference stepping; `rule_dialog.rs`'s defaults, queries and validation; `rules.rs`'s row words; `filtered.rs`'s paging and tabs; `capture.rs`'s modes, quick picks, project filter and words; `keymap_dialog.rs`'s title and footer

### Surfaces

- [X] T309 [US16] The inbox (scenario 1): open `FocusScope::Inbox`, spliced digest and reminder rows, labels and `focus_counts`, re-read on `SurfacedChanged`; one- and two-line rows, marks, pills, trailing column and day headings as terminal.md draws them. Test first on the rendered screen Done: `Effect::Fetch` carries the surfaced rows' `Placement`, so the executor places them and reads each page's labels once; `Enter` on a digest waits for T323
- [X] T310 [US16] Markers and their actions: the second line, `y`/`Y` answering, `e`, `s`, `t` with a vault, `-` dismiss, an answered or past invitation's status (scenario 3) Done: `y`, `Y` and a drawn answer send the row's message to the host; `t` waits for the vault (T325), `s` for the picker (T320)
- [X] T311 [US16] The strip and top bar: place name, counts, `!` with its accent and "Showing", filtered and rule counts, the sync label and `? keys` Done: every strip and top-bar item is a `Target::Command` click; the sync label folds the tracked accounts' events and the folders' last sync
- [X] T312 [US16] Selection and the bulk bar: `x`, `J`/`K`, `X` as a predicate (C19), `Esc`, the bar's verbs (scenario 4)
- [X] T313 [US16] The bottom line's toast with Undo, 8 s, beside the bulk bar
- [X] T314 [US16] States: the banner (`focus_state::banner`) with its action, first sync's progress, the empty inbox
- [X] T315 [US16] The open message frame (scenario 2, FR-195): header, steps, position, action row and More, the column and its rhythm, the action card and the quote's highlight, attachments, the fold line, `j`/`k`, `[`/`]`, `o`, `v`, read on dwell, `Esc` back to the same row; the list dimmed behind Done: `find_*` stay in GAPS (spec 006's find has no terminal index yet); the column shows one message at a time, so `toggle_fold` says so
- [X] T316 [US16] Reading beside the list, `F8` and `[focus] reading`, from 128 columns
- [X] T317 [US16] The composer in the frame; detach to the whole screen (FR-196); reply, reply all and forward; Send later; Remind `ctrl+h` Done: Remind `ctrl+h` opens the schedule picker's numbered list with the reminder presets until the pickers (T320) replace it; Escape from a detached draft brings the frame back
- [X] T318 [US16] The command bar: opening in place, saved searches, chips, `in:`, commands with keys, results, the footer
- [X] T319 [US16] Folders and labels, `g o`, and the `g` go-to keys
- [X] T320 [US16] Pickers: snooze, remind, label and move, anchored at the row
- [X] T321 [US16] The key map, `?`, from `keymap_sheet` for the terminal
- [X] T322 [US16] Filtered, `g f`: the view, tabs, reasons, `R`, Sweep `F` Done: the strip, tab line and day-headed rows take the window; `R`, `1`-`7`, `F` (a framed question first) and the mouse all work; rows are read fifty at a time as they come into view; Return does not open a filtered message here, since the open message is aimed at the inbox's rows
- [X] T323 [US16] The digest window: summary and list, references, the email from a reference, `A`, `D`, `U` Done: Enter on a digest row opens the message frame as the window; it opens on the summary only when one is written; `]`/`[`, `Tab`, `A`, `D` (asked first), `U`, `ctrl+z` and every part by the mouse; `d` waits for T324
- [X] T324 [US16] Digest rules, `g d`, and the rule dialog, `d` Done: `g d` lists each rule with its match, when, next delivery and what it holds, Return edits and Delete asks before it removes; `d` opens the 64-wide dialog from a row, a selection, an open message or a digest, with the 90-day preview, "Match a list or a search instead…", and `L` (a rule from the model) only while `[focus.model]` has like_this on
- [X] T325 [US16] Capture, `t` and `n` Done: a frame 72 wide with the Task/Note pair, the sentence or the subject, due quick picks, the suggested project with its reason and `ctrl+p` to change it, and the exact line as the preview; `ctrl+Return` writes through the host and the toast says where; with no vault the toast says to name one; a marked to-do's row offers `Task t` when a vault exists
- [ ] T326 [US16] The mouse on every new surface (scenario 7, FR-192), and `NO_COLOR` on every new surface (scenario 6, FR-193)

### Closing

- [ ] T327 [US16] `examples/shot.rs` renders every surface in terminal.md, and each is compared with its drawing there, with differences recorded (FR-199)
- [ ] T328 [US16] Spec 005 says what is true now: `contracts/tui-surface.md`'s Layout, Mouse and Colour tables point at terminal.md, FR-002's sidebar goes, and the README, `docs/keybindings.md` (through the registry) and the book describe the terminal as Focus
- [ ] T329 [US16] Budgets (FR-197, SC-017): the startup, keystroke and rows-read counts hold for the new inbox and frame; `GAPS` and `NOT_YET` are empty

## Dependencies and order

- **Phase 1**, then **Phase 2**, which blocks everything.
  - Within Phase 2, each spike settles a later task:
    - T007: T107 (the calendar adapter)
    - T008: T104 (the promoted headers)
    - T009: T041 and T095 (the list and its spliced rows)
    - T010: T116 (the detector)
    - T011: T065 and T066 (highlighting)
    - T012: T093 (`sort_at`)
  - The shared-crate moves (T013–T025) come before any Focus surface.
  - The one keymap (T026–T032) comes before any Focus key is drawn.
- **P1 stories**, in the order of Phases 3–10.
  - US1 comes first: every other story draws in its window.
  - US6, US7 and US11 depend only on US1.
  - US2 depends on US1 and T065–T066.
  - US3 depends on US1 and T022–T024.
  - US4 depends on US1.
  - US5 depends on US1, T036 and T037.
  - US3's reminder field (T096) waits for US5's reminders (T094).
- **Phase 11** comes before the P2 stories. US8, US12, US9 and US10 can then run in parallel. US9 and US10 each touch the filing pass: they are separate rules, and the pass is written in T102.
- **Phase 16** closes milestone 1. Phases 17 and 18 follow in order, and each depends on milestone 1.

## Parallel examples

- **After T024:** T026–T028 (registry, contexts, alias) can run beside T033 and T035–T037 (engine seams).
- **Within US1:** T040 (colours) and T042 (label colours) run beside T041 (list model).
- **Within US4:** T084 (natural lowering, postio-search) and T085 (finder, postio-ui) touch different crates.
- **Within US10:** T131 (matcher) and T132 (config) run beside each other, before T133.

## Implementation strategy

- **Milestone 1** is Phases 1–16, and within it:
  - The P1 phases make Focus a complete, keyboard-first mail client on the one keymap. Each phase ends with its screens compared.
  - The P2 phases add the four differentiators that need no model.
- **Milestones 2 and 3** follow on the same branch.
- **Nothing lands until the maintainer says so.**
  - The branch is rebased onto `main` throughout.
  - At the maintainer's word it lands once, as one pull request reviewed against the spec, with the constitution's amendment beside it.

## Requirement coverage

Each requirement and success criterion in the spec, and the tasks that
satisfy it. A requirement with no task here is a gap. `/speckit-analyze` checks
this table against the spec.

| Requirement | Tasks |
|---|---|
| FR-001, FR-002 | T001, T034, T038 |
| FR-003 | T063 |
| FR-004 | T064 |
| FR-005 | rule 1; T013–T025, T029 (their suites unchanged but for paths and keys) |
| FR-006 | T018, T031, T036, T037, T042, T066, T076, T084, T085 |
| FR-007 | T001, T002, T004, T013, T016, T019, T021–T024 |
| FR-008 | rule 6; T013–T025 |
| FR-009 | T029, T106, T145, T146 |
| FR-010, FR-011 | T041, T043 |
| FR-012 | T042, T043, T044 |
| FR-013 | T009, T043, T044 |
| FR-014 | T021, T041 |
| FR-015 | T045 |
| FR-016 | T047 |
| FR-017 | T048 |
| FR-018 | T039, T126, T140 |
| FR-020 | T033, T041, T109 |
| FR-021 | T109, T116, T117 |
| FR-022 | T033, T048, T089, T095, T102, T109, T124, T134 |
| FR-030 | T069 |
| FR-031, FR-032 | T070 |
| FR-033 | T068, T072 |
| FR-034 | T067, T071 |
| FR-035 | T065, T066, T073 |
| FR-036 | T069 |
| FR-037 | T069, T145 |
| FR-040 | T029, T045, T046, T092, T097, T098 |
| FR-041 | T029, T046 |
| FR-042 | T091, T097, T098, T099 |
| FR-043 | T037 |
| FR-044, FR-045 | T094, T096 |
| FR-050 | T023, T024, T078 |
| FR-051 | T080 |
| FR-052 | T075, T076, T079 |
| FR-053 | T077, T078 |
| FR-054 | T078 |
| FR-055 | T082 |
| FR-060 | T085, T086 |
| FR-061 | T085 |
| FR-062 | T084, T086 |
| FR-063 | T032, T086, T087 |
| FR-064 | T086 |
| FR-065 | T088 |
| FR-070 | T056, T140 |
| FR-071 | T052 |
| FR-072 | T053, T054 |
| FR-080 | T029, T058, T059 |
| FR-081 | T029, T060 |
| FR-082 | T029, T030 |
| FR-083 | T059, T128 |
| FR-084 | T031, T058 |
| FR-085 | T030 |
| FR-090, FR-093 | T040 |
| FR-091 | T040, T042 |
| FR-092 | T016, T069, T091 |
| FR-094 | T143 |
| FR-095 | T051, T057, T062, T074, T083, T090, T100, T114, T120, T130, T141, T148 |
| FR-096 | T142 |
| FR-100 | T108, T110 |
| FR-101 | T007 (the survey is research R9) |
| FR-102 | T111, T112, T113 |
| FR-103 | T110 |
| FR-104 | T115, T116, T117 |
| FR-105 | T116 |
| FR-106 | T010, T116 |
| FR-107 | T153 |
| FR-108 | T118 |
| FR-110, FR-112 | T122 |
| FR-111 | T101, T122 |
| FR-113 | T121 |
| FR-114 | T105 |
| FR-115, FR-117 | T124 |
| FR-116 | T124, T125 |
| FR-118 | T102, T128 |
| FR-119 | T123 |
| FR-120 | T138 |
| FR-121 | T133, T139, T140 |
| FR-122 | T133 |
| FR-123 | T135, T136 |
| FR-124 | T136, T154 |
| FR-125 | T137 |
| FR-126 | T139 |
| FR-127 | T131, T138 |
| FR-130 | T035, T101, T102, T116 |
| FR-131 | T102, T103 |
| FR-132 | T002, T035, T119 |
| FR-133 | T035, T152 |
| FR-134 | T034, T127 |
| FR-140 | T033, T069, T089, T147 |
| FR-141 | T103, T147 |
| FR-142 | T151, T154 |
| FR-150 | T072, T112, T149, T152 |
| FR-151 | T102, T149 |
| FR-152 | rule 5; T007, T010 |
| FR-153 | T129 |
| FR-160 | T060, T123 |
| FR-161 | T123, T125, T132 |
| FR-165 | T002, T151 |
| FR-166, FR-167 | T117, T152 |
| FR-168, FR-169 | T151, T152 |
| FR-170 | T153 |
| FR-171 | T155 |
| FR-172–FR-175 | T154 |
| FR-180, FR-181 | T157 |
| FR-185 | T159 |
| FR-186 | T302 |
| FR-187 | T300, T329 |
| FR-188 | T303, T309-T325 |
| FR-189 | T304-T308 |
| FR-190 | T300, T303 |
| FR-191 | T303 |
| FR-192 | T326 |
| FR-193 | T326 |
| FR-194 | T309-T325 |
| FR-195 | T315, T316 |
| FR-196 | T317 |
| FR-197 | T329 |
| FR-198 | T318, T319, T315 |
| FR-199 | T327 |
| SC-001 | T033, T089, T147 |
| SC-002 | T041, T109 |
| SC-003 | T059 |
| SC-004 | T149 |
| SC-005 | T069, T070 |
| SC-006 | T122 |
| SC-007 | T133, T135 |
| SC-008 | T112 |
| SC-009 | T148 |
| SC-010 | T013–T025, T029 |
| SC-011 | T147 |
| SC-012 | Measured on the maintainer's mailbox for a month after milestone 1, from the decisions and restores the store keeps (T121, T125) |
| SC-013 | T010, T116 |
| SC-014 | T154 |
| SC-015 | T029 |
| SC-016 | T117, T152 |
| SC-017 | T309-T329 |
