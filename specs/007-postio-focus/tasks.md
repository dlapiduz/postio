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
- Focus's integration tests (`crates/postio-focus/tests/focus_suite/`) assert
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

- **Paths** are workspace-relative, from `~/src/postio-worktrees/postio-focus`.
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
  - `crates/postio-focus/` (bin `postio-focus`)
  - `crates/postio-classify/`
  - `crates/postio-calendar/`
- [X] T002 Add the boundary rules of contracts/engine.md to `scripts/checks/check-crate-boundaries.py`, with their reasons: postio-widgets, postio-focus, postio-classify and postio-calendar, and postio-gtk's "not postio-focus". Add a workspace-wide ban on inference engines to `scripts/checks/check-dependency-policy.py`: candle, ort, tch, tract, burn, and llama.cpp bindings (FR-165). Test first: a fixture graph where `postio-focus` depends on `postio-gtk`, one where `postio-classify` depends on `postio-smtp`, and one where any crate depends on `candle-core`, each fails its check
- [X] T003 [P] Widen the eight checks that scan only `crates/postio-gtk` so they also scan `crates/postio-widgets` and `crates/postio-focus` (research R1):
  - `check-key-hints-are-derived.py`
  - `check-buttons-have-a-kind.py`
  - `check-no-dead-css.py`
  - `check-shadows-use-tokens.py`
  - `check-spacing-literals-ratchet.py`
  - `check-reader-header-has-one-home.py`
  - `check-blocking-now-sites.py`
  - `check-uncalled-pub-fn.py`'s `FRONTENDS`

  Test first: a literal key hint planted in `crates/postio-widgets/src` fails `check-key-hints-are-derived.py`
- [X] T004 [P] Write `docs/decisions/0043-gtk-both-desktop-apps-share-lives-in-postio-widgets.md`, kept to the rule: what may live there, what may not, and who depends on it. List it in `docs/decisions/README.md`
- [X] T005 [P] Create `crates/postio-focus/tests/focus_suite/main.rs` on the `app_suite` custom harness (`CASES`, `IGNORED`, the `--list` contract of `list_contract.rs`) on the headless compositor, and `crates/postio-widgets/tests/widgets_suite/main.rs` the same way. Test first: an empty case is listed and runs. `widgets_suite` is done. `focus_suite` comes with T038, when there is a Focus to run
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
- [X] T009 [P] **S3.** In a throwaway `crates/postio-focus/examples/list_spike.rs` (deleted after), put 100,000 synthetic rows of two fixed heights, plus 50 spliced rows, in a `gtk::ListView`. Measure rows built per frame while scrolling and jumping, and record them in research R3
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

- [X] T038 [US1] `crates/postio-focus/src/{main,app,startup}.rs`: an `AdwApplication` with id `dev.postio.Postio.Focus`. The store opens on a thread behind a window that says what it waits for. `enable_focus` is called. Test first, in `focus_suite/starts_offline.rs`: a fixture store, no network, and the inbox is listed (scenario 1). Call `Host::enable_focus` before `start_syncing`, and connect as `ClientKind::Focus` (T034). Build the resolver with `Resolver::from_commands_for(.., Frontend::Focus)`, and set `Availability.frontend = Focus`. Call `host.enable_focus(FocusSetup::default().with_config(config.focus))`, and again on every `[focus]` change (T060). Pass the config path (`FocusSetup::with_config_path`), because Focus's config writes refuse without it, and call `Host::stop` on quit, which keeps the filing mark (T164)
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
- [X] T050 [US1] `crates/postio-focus/examples/shot.rs`: a seeded demo store (the storage seed, plus markers written through the host) that renders a named screen, light or dark, at a given size. Test first: `shot 01` writes a PNG, and an unknown screen exits non-zero with `NO IMAGE WAS WRITTEN`
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
- [X] T144 [P] Packaging: `flatpak/dev.postio.Postio.json` builds `postio-focus`, `crates/postio-focus/data/dev.postio.Postio.Focus.desktop` is added, and release.yml's `flatpak` job carries both apps. Test first: the packaging test pattern of `crates/postio-tui/tests/packaging.rs`. Focus is a second launcher in the one Flatpak, so the classic metainfo names it rather than a second component, and the release job checks that the build carries both apps
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
- [ ] T198 Pointer selection on the list: Ctrl-click toggles a row's selection and Shift-click extends it to a range, the mouse pair of `x` and `Shift`+`j`/`k`. Found by the T194 audit. Test first, driving the click with its modifier
- [ ] T199 A right-click menu on a list row: the row's verbs (archive, snooze, label, move, mark read) as the toolbar offers them, each running its one command with its key shown. Found by the T194 audit; the verbs to include are a `/ux-architect` call
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
