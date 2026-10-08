---
description: "Task list for spec 009, Postio Focus on macOS"
---

# Tasks: Postio Focus on macOS

**Input**: `specs/009-focus-macos/` — [spec.md](spec.md), [plan.md](plan.md),
[research.md](research.md), [data-model.md](data-model.md),
[contracts/](contracts), [quickstart.md](quickstart.md)

**Tests**: Required. The constitution (IV) makes test-first non-negotiable.
Every implementation task is preceded by a task that writes its test and
watches it fail. Interactions are written as storyboards before they are
built (FR-063).

## Conventions

- `[P]` means the task can run in parallel: different files, and no
  dependency on an unfinished task.
- `[USn]` is the user story in spec.md the task serves.
- **Where a task lands:**
  - **`main·Sn`**: in controller slice *n*'s own pull request to `main`, from
    a branch `focus-controller/sn-<slug>` cut from `origin/main`. In that PR,
    GTK adopts the slice without changing behaviour. Verify locally with
    `cargo test -p postio-focus` and `cargo check -p postio-ffi`; CI proves
    `postio-gtk`, which does not build on the Mac. Grep for orphaned imports
    first (`docs/notes/2026-09-06-moving-code-out-of-a-crate-you-cannot-compile.md`).
    Land with `scripts/issue-land.sh --detach`, then rebase
    `feature/focus-macos` onto `main`.
  - **`main·fix`**: its own small PR to `main`, a behaviour change with its
    own failing test.
  - **Unmarked**: a commit on `feature/focus-macos`.
- **Commits** end `Refs: specs/009-focus-macos` and the task id (constitution,
  Development Workflow). Never write a closing keyword.
- **Every Mac phase ends with its PNG comparison** (FR-061): capture with
  `scripts/macos-shot.sh <nn>` at 1440×900 in light and dark, and record
  every difference in `docs/notes/<date>-focus-macos-phase-<n>.md`, fixed or
  explained by a decision (C*, M*).
- **Before any cargo command on this Mac**, `unset RUSTUP_TOOLCHAIN`.
- **Uncalled public functions.** Slice 1 lists `digest_size`, `for_platform`
  and `refresh_counts` in `scripts/checks/uncalled-pub-fn-baseline.txt`, each
  naming the task that first calls it. That task removes the line.

---

## Phase 1: Setup (documents and tooling)

- [x] T001 Write ADR 0045 "Focus's behaviour lives in `postio-focus`; frontends draw its intents" in `docs/decisions/0045-focus-behaviour-lives-in-postio-focus.md`: the rule, the crate boundary from contracts/focus-controller.md, and "no Focus rule in a window" (plan.md, ADR). Add its row to `docs/decisions/README.md`
- [x] T002 [P] Amend `docs/decisions/0019-macos-frontend.md`: set the status to "partly superseded by spec 009". Its boundary, SwiftPM, packaging and Linux-stays-green rules hold; its three-pane surface and read-only framing go
- [x] T003 [P] Fold `specs/007-postio-focus/macos.md` into spec 009: anything still true and not already in spec.md or research.md goes into spec.md's Context. Delete the file, and update every link to it (`grep -rn "007-postio-focus/macos.md"`), including `crates/postio-ffi/tests/ffi_suite/command_coverage.rs` KNOWN_ORPHANS comments
- [ ] T004 [P] Add `scripts/macos-shot.sh <screen> [--dark|--light|--both] [--width W]`:
  - copy the seeded demo store (`postio_storage::seed`) to a scratch directory;
  - launch `macos/build/Postio.app/Contents/MacOS/Postio` with `POSTIO_STORE`/`POSTIO_CONFIG`, with `-AppleInterfaceStyle Dark` for dark;
  - size the main window to 1440×900, or the given width;
  - capture with `screencapture -l<windowid>` into `Design/review/focus-macos/<screen>-<appearance>.png`.

  Document it in quickstart.md
- [x] T005 [P] Add `crates/postio-focus/` to the workspace (root `Cargo.toml` `members` and `default-members`), with an empty `lib.rs` and `Cargo.toml` depending on `postio-ui`, `postio-client`, `postio-core`, `postio-model`, `postio-config`, `postio-search`, `chrono`, `tracing`. Land it in the slice-1 PR (T008)
- [x] T006 Add the `postio-focus` rule to `scripts/checks/check-crate-boundaries.py`:
  - **banned:** gtk4/gdk4/libadwaita/webkit6 (+`-sys`), `turso*`, `rusqlite`, `libsqlite3-sys`, `io-imap`, `postio-host`/`-session`/`-storage`/`-runtime`/`-widgets`/`-gtk`, `uniffi`;
  - **not allowed as direct dependencies:** `tokio`, `glib`, `async-std`.

  First write a fixture in that script's test proving a GTK dependency fails it. **main·S1**

---

## Phase 2: Foundational (blocks every story)

**Purpose**: the controller skeleton, platform geometry, the FFI as a Focus
frontend with the engine's Focus pass on, and the Swift package split.

### Controller slice 1: skeleton and geometry by platform (**main·S1**)

- [x] T007 Write failing tests in `crates/postio-ui/src/focus_dialog.rs` for `Platform::Apple` geometry (data-model.md table):
  - W 1024→656, 1280→720, 1440→720, 1920→720;
  - text column `min(560, w−80)`; paper `min(640, w−80)`; More below 700;
  - digest = the message size with a 560 column;
  - Linux values unchanged (the existing tests stay).

  Loop over both platforms (`docs/notes/2026-09-05-the-gate-that-runs-cannot-see-the-platform-that-does-not.md`)
- [x] T008 Add `Geometry` (`Geometry::for_platform`, `LINUX`, `MAC`, with `dialog_width`, `dialog_height`, `folds_into_more`, `column_width`, `digest_size`) to `crates/postio-ui/src/focus_dialog.rs`. The existing free functions delegate to `Geometry::LINUX` and keep their API, so GTK changes nothing in this slice. GTK moves onto `Geometry` with the slice that moves its call sites (S6). Land T005-T009 as slice 1
- [x] T009 Define the controller's public types in `crates/postio-focus/src/lib.rs` per contracts/focus-controller.md: `FocusController`, `Policy{platform, caps{reading_pane, stacking}}`, `Input`, `Effect`, `Intent`, `Request`, `Reply`, `Ticket`, `Press`, `trait Rows`, `FocusView`. `perform` goes in `crates/postio-focus/src/perform.rs`. Add a test asserting `FocusController: Send`. **main·S1**

### The FFI becomes a Focus frontend

- [x] T010 Write a failing test in `crates/postio-ffi/tests/ffi_suite/command_coverage.rs`: `offered_on_the_mac` means `Frontend::Focus` on `Platform::Apple`; Focus commands are owed and no `ThreePane` command is offered
- [x] T011 Write a failing test in `crates/postio-core/tests/core_suite/one_keymap.rs` that resolves the whole Focus keymap for both `Platform::Freedesktop` and `Platform::Apple`, with no unparseable binding. Delete Macos from `APPS`
- [ ] T012 (Done with T028 and T034 in US1, not here: while the three-pane Swift shell still exists, removing its commands leaves it drawing keys that do nothing.) Remove `Frontend::Macos` and `Requirement::ThreePane` from `crates/postio-core/src/registry.rs` (lines ~242-299, ~375-377), with the 19 ThreePane-only commands (ToggleRail, ToggleSidebar, CyclePane, CyclePaneBack, NextFolder, PrevFolder, ToggleFolder, RenameSavedSearch, MoveSavedSearchUp/Down, DeleteSavedSearch, OpenParts, NextPart, PrevPart, OpenPart, SavePart, SaveAllParts, OpenPartExternally, RenderPartOnce) and their `CommandId` variants. Fix:
  - `postio-ui/src/{keymap_sheet.rs,settings.rs:214,palette.rs:370}`;
  - `postio-core/tests/core_suite/command_registry.rs`;
  - `postio-ui/tests/ui_suite/{keymap_api.rs,keybindings_doc.rs}`;
  - the regenerated `docs/keybindings.md`.

  It lands on `feature/focus-macos`, **not** `main`: until that branch
  lands, `main`'s Mac app is still the three-pane app and uses these
  commands every day
- [x] T013 Point the FFI at `Frontend::Focus` (settings sections stay the Mac's until T131 draws Focus's Filtering pane) in `crates/postio-ffi/src/session.rs:422,4365` and `crates/postio-ffi/src/settings.rs:316`. Update `INTERCEPTED` (`crates/postio-ffi/src/registry.rs:275-345`) and its Swift mirror `Intercepted` (`macos/Sources/PostioKit/Accessibility.swift`) to drop the three-pane entries. Make T010 green
- [x] T014 Write a failing ffi_suite test: a session opened with `[focus]` enabled files a seeded promotion and marks a seeded question (the markers appear in `focus_counts().has_action`)
- [x] T015 (`feature/focus-macos` only: on `main` it would start filing the three-pane Mac app's mail away) Call `Host::enable_focus(FocusSetup::from_config(config.focus, Some(config_path)))` in `crates/postio-ffi/src/session.rs` after the host starts and before `start_syncing`, as `crates/postio-gtk/src/startup.rs:116,174` does (R8). Make T014 green
- [x] T016 Write a failing ffi_suite test: rewriting `[keys]` in the session's `config.toml` changes what `key()` resolves within 1 s, and emits `UiEvent::KeymapChanged`
- [x] T017 Start `ConfigService::watch` in `crates/postio-ffi/src/session.rs`:
  - on `ConfigChanged.keys`, rebuild the resolver and emit `UiEvent::KeymapChanged`;
  - on `.focus`, re-run `enable_focus`;
  - on `.filters`, refresh saved searches.

  Mirror GTK's `follow_config` (`crates/postio-gtk/src/startup.rs:95-150`). Append `KeymapChanged` and `SurfacedChanged` at the end of `UiEvent` (the typed `BackfillProgress` comes with its banner, T099) in `crates/postio-ffi/src/event.rs`. Make T016 green
- [ ] T018 [P] Write a failing test in `crates/postio-core` that Delete resolves from `BackSpace` on `Platform::Apple` in the List and Reader contexts, without colliding with the bar's `mod+BackSpace`. Then add `"BackSpace"` to Delete's `alternate_bindings` (`crates/postio-core/src/registry.rs:903-913`) if it is free in those contexts (M6, R6). **main·fix**

### The Swift package split

- [x] T019 Add a `PostioAppKit` target to `macos/Package.swift` (it depends on `PostioKit` and `PostioFFI`), and a `PostioAppKitTests` test target. `Postio` depends on both
- [x] T020 Move the AppKit-importing files from `macos/Sources/PostioKit/` to `macos/Sources/PostioAppKit/`, with their tests to `macos/Tests/PostioAppKitTests/`:
  - `KeyEvent`, `KeyWindowTracker`, `TypingResponder`, `ViewTreeFocus`;
  - `MenuBar`, `ComposeEditor`, `ComposeHandoff`, `ComposeView`;
  - `ReaderView`, `ReaderPolicy`.

  Split `Accessibility.swift`: `Intercepted` stays in PostioKit, and `Motion` (NSWorkspace) moves. Run `scripts/macos-test.sh` green
- [x] T021 Write a Swift test in `macos/Tests/PostioKitTests/NoAppKitTests.swift` that fails if any file under `macos/Sources/PostioKit/` contains `import AppKit` or `import Cocoa` (#1264)
- [x] T022 [P] Write a Swift test in `macos/Tests/PostioKitTests/SemanticColourTests.swift` that fails on a hex colour literal or `Color(red:` / `NSColor(red:` under `macos/Sources/` (contracts/mac-surfaces.md, FR-017)

**Checkpoint**: the FFI is a Focus frontend, Focus's engine pass runs on the
Mac, the controller crate exists on `main`, and the Swift package is split.

---

## Phase 3: User Story 1 — the Mac opens on the Focus inbox (P1) 🎯 MVP

**Goal**: screens 01 and 02. The toolbar, the header strip, and the Focus
list with day headers and markers. The three-pane shell is gone.

**Independent test**: launch over the demo store. 01 and 02 match, with
differences listed. A 10k-conversation store scrolls without dropped frames.

### Controller slice 2: the feed (**main·S2**)

- [x] T023 [US1] Write failing tests in `crates/postio-focus/tests/feed.rs`:
  - opening the inbox asks for accounts, the count, surfaced and page 0;
  - a reply for an old generation is dropped;
  - a scope switch during the count discards it;
  - a page retry gives up after the limit;
  - `NewMail` at the top inserts, and `MessageListChanged` reloads (the `postio_ui::paging::Paging::plan` table);
  - `SurfacedChanged` re-reads surfaced rows and splices them at their `position`.
- [x] T024 [US1] Move the feed logic from `crates/postio-gtk/src/list/feed.rs` (`Inner`, lines 37-60; the open-scope sequence 136-196; pages 264-336; the event reaction 217) into `crates/postio-focus/src/feed.rs`. Implement `perform` for `OpenScope`, `Page`, `FocusCounts` and `Surfaced` in `crates/postio-focus/src/perform.rs`. Make T023 green
- [x] T025 [US1] Reduce `crates/postio-gtk/src/list/feed.rs` to a driver: it runs `perform` with `glib::spawn_future_local`, feeds back `Input::Reply`, and applies `DeliverPage`/`RefreshList` to `WindowedModel`. The guards are `focus_suite` cases `list_contract`, `list_reload`, `reload`, `surfaced`, `rows` and `startup_reads` (CI). Land slice 2

### FFI: Focus rows and the controller driver

- [x] T026 [US1] Write failing ffi_suite tests:
  - `open(FocusScope::Inbox)` over the seed yields `FocusRowFfi`s with `day_heading` on the first row of each day;
  - a marked row carries `MarkerLineFfi{kind, date, quote, action_id, action_label}`;
  - a digest row carries `DigestRowFfi`;
  - `row_count` equals the inbox count;
  - controller intents arrive as `UiEvent::Intents`.
- [x] T027 [US1] Create `crates/postio-ffi/src/focus.rs`:
  - `FocusRowFfi`, `MarkerLineFfi`, `LabelPillFfi`, `DigestRowFfi`, `IntentFfi`, `OriginFfi`, `FactFfi`, `FocusCountsFfi`;
  - conversions from `postio_ui::focus_row` and `focus_list::FocusRow`;
  - a `Focus` arm in `ScopeFfi` (`crates/postio-ffi/src/list.rs`).

  In `session.rs`:
  - hold `Mutex<FocusController>` and `Mutex<ListWindow<FocusRow>>`;
  - run `perform` on the session runtime and feed replies back;
  - forward reply- and event-driven intents as `UiEvent::Intents`;
  - export `command(id, origin)` and `ui_fact(fact)`.

  Never call Swift while holding a lock. Make T026 green
- [ ] T028 [US1] (After T034, when the Swift Focus list has replaced the classic one.) Delete the classic list, cursor and selection machinery from `crates/postio-ffi/src/session.rs`: `HANDLED_HERE` (:133), the `selection`/`cursor`/`cursor_row`/`anchor` fields (:624-646), `handle_locally` (:4236), `RailFfi` and `rail_presentation` (`crates/postio-ffi/src/rail.rs`), `next_pane`, and the sidebar and parts exports. Delete their ffi_suite tests; keep `cargo nextest run -p postio-ffi` green

### Mac: the inbox

- [ ] T029 [P] [US1] Write a storyboard `storyboards/list/inbox-opens-on-the-first-row.toml` (`apps = ["focus"]`): launch, the cursor on index 0, and the keyboard region the list (C30). Run it on Linux with `scripts/storyboards.sh run` to see it pass there
- [ ] T030 [US1] Write failing Swift tests in `macos/Tests/PostioKitTests/FocusRowModelTests.swift` for the row view model built from `FocusRowFfi`:
  - the row kind is header, one-line or two-line;
  - at most two pills;
  - the marker's action keycap text comes from the FFI's spelling, not a literal;
  - unread is bold.
- [ ] T031 [US1] Implement `FocusRowModel` and `FocusListModel` (the row count, `row(at:)` via the FFI, applying `DeliverPage`/`RefreshList`/`Cursor`/`Selection` intents) in `macos/Sources/PostioKit/FocusList.swift`. Make T030 green
- [ ] T032 [US1] Implement `FocusListTable` (`NSTableView`, view-based) in `macos/Sources/PostioAppKit/FocusListTable.swift`:
  - three row views (day header, one-line, two-line) with fixed heights per kind;
  - table selection disabled;
  - the accent focus ring on the cursor row only;
  - a checked-box gutter for selected rows;
  - semantic colours only.

  Reuse what fits from `MessageTable.swift`/`MessageRowView.swift`, then delete those files
- [ ] T033 [US1] Implement the main window in `macos/Sources/Postio/MainWindow.swift`:
  - an `NSToolbar` (unified): compose button, flexible space, sync label, `NSSearchToolbarItem` with a ⌘K keycap;
  - the SwiftUI header strip in `macos/Sources/PostioKit/HeaderStrip.swift`: Inbox ▾ `g o` with its count, Has action `!` with its count, and the filtered and digest counts only while in use (C10);
  - `FocusListTable` below.
- [ ] T034 [US1] Delete the three-pane shell:
  - **in `macos/Sources/Postio/`:** `Shell.swift`, `FolderRow.swift`, and the three-pane parts of `Engine.swift` (`collapsedFolders`, `sidebar*`, `folderCursor`, `pick(SidebarRowId)`, `stepSidebar`, `children(of:)`, `specialFolders`, `folderRoots`, `open(mailbox:)`, `pane`/`focus(_:)`, `conversation`, `railHidden`, `showingThread`, `parts`, `readerPages`, `bodyHeights`);
  - **in `macos/Sources/PostioKit/`:** `Sidebar*.swift`, `SavedSearchRows.swift`, `ConversationRail.swift`, `ConversationView.swift`, `ConversationModel.swift`, `ThreadDocumentView.swift`, `ToolbarPlan.swift`, `ReaderActionPlan.swift`, `SearchScopeRail.swift`, `SearchRefineBar.swift`, `PartsPanel.swift`, `PartPreview.swift`, `BodyHeight*.swift`;
  - **their tests.**

  Wire `PostioApp.swift`'s main scene to `MainWindow`. Keep `scripts/macos-test.sh` green
- [ ] T035 [US1] Write a performance check in `macos/Tests/PostioAppKitTests/FocusListScrollTests.swift`: over a 10k-conversation seeded store, row views per scroll page stay bounded and `row(at:)` FFI calls per frame stay ≤ the visible rows plus the prefetch page (counted, not timed; constitution V)
- [ ] T036 [US1] Compare screens 01 and 02 (FR-061): `scripts/macos-shot.sh 01 --both`, against `Design/focus-macos-design/screens/01-inbox-light.png` and `02-inbox-dark.png`. Record the differences in `docs/notes/<date>-focus-macos-phase-1.md`, then fix or explain each one

**Checkpoint**: the Mac launches into the Focus inbox. This is the MVP.

---

## Phase 4: User Story 2 — triage from the keyboard (P1)

**Goal**: screen 03 and the triage keys. Selection, the action bar, Has
action, and undo through ⌘Z and Edit › Undo.

**Independent test**: select three rows, archive them, choose Edit › Undo
"Archive 3 messages", and the rows return with the cursor on them.

### Controller slice 3: cursor, selection, Has action, strip counts (**main·S3**)

- [x] T037 [US2] Write failing tests in `crates/postio-ui/src/selection.rs` for a plain `Selector {selection, anchor: MessageId, reach}`: toggle, extend up and down from the anchor, select all as a predicate (C19), clear, and `changed: bool` returned
- [x] T038 [US2] Extract `Selector` from `SelectionState` in `crates/postio-ui/src/selection.rs`. `SelectionState` stays as the `Rc`/observer wrapper over it for the terminal. Make T037 green
- [x] T039 [US2] Write failing tests in `crates/postio-focus/tests/cursor.rs` for contract invariants 1-3 and 9:
  - `x` never moves the cursor;
  - ⇧J/⇧K extend, skipping digest rows (`window.rs:1952`);
  - `!` clears the selection and keeps the cursor on the same message, else on row 0;
  - every list opens on row 0;
  - a printable key with `in_text_entry` is not handled;
  - strip counts show only while in use (`window.rs:2801-2858`).
- [ ] T040 [US2] Move `move_cursor`, `cursor_to`, `extend`, `take_cursor_row`, `pick`, ToggleSelection/SelectAll/Back's selection arm, `toggle_has_action`, `keep`, `list_landed`, `cursor_to_first`, `update_counts`, `show_counts`, `show_empty_or_list` and `selection_moved` from `crates/postio-gtk/src/window.rs` (~1902-2046, 2582-2914) into `crates/postio-focus/src/cursor.rs`. GTK applies the `Cursor`/`Selection`/`Strip`/`EmptyOrList` intents; its `SelectionState` becomes a mirror written only by the intent applier. The guards are focus_suite `cursor`, `selection`, `has_action`, `place_strip`, `empty`, `marked_rows`, `pointer_pairs` and `keyboard_home`. Make T039 green; land slice 3

### Controller slice 4: verbs, aim, removal, undo cursor (**main·S4**)

- [x] T041 [US2] Write failing tests in `crates/postio-focus/tests/verbs.rs`:
  - `a` sends Archive aimed at the selection, else the cursor's row;
  - after removal the cursor goes to the survivor below;
  - `ActionCompleted` yields `Toast{text, undoable}`;
  - `UndoPerformed` places the cursor on the restored row;
  - `Aim::Everything` is resolved once in `perform`;
  - `DismissMarker`, `AcceptInvite`/`DeclineInvite` and `ArchiveDigest` send the right commands.
- [ ] T042 [US2] Move `aims`, `dispatch`, `send`, `post`, `note_removed`, `cursor_past_removed`, `restore_cursor`, `place_restored`, `archive_digest`, `answer`, `dismiss_marker`, and `hear`'s toast arms from `crates/postio-gtk/src/window.rs` (~2046-2241, 2964-2999, 3102-3126) into `crates/postio-focus/src/verbs.rs`. Implement `perform` for `Send`/`Post`. The guards are focus_suite `undo`, `selection` (`archive_hands_the_cursor_to_the_row_below`), `invitations`, `offline_send` and `marker_card`. Make T041 green; land slice 4

### Engine: the undo top (**main·fix**)

- [ ] T043 [US2] Write a failing test in `crates/postio-session` that `Actions::peek_description()` returns "Archived 3 messages" after archiving three, `None` after undo, and `None` once the entry's window has closed
- [ ] T044 [US2] Add `Actions::peek_description` (`crates/postio-session/src/actions.rs`, over `UndoStack::peek`, `crates/postio-core/src/undo.rs:397`), `Req::UndoTop`/`Resp::UndoTop(Option<String>)` (`crates/postio-client/src/protocol.rs`), its host handler, and `Client::undo_top()` (`crates/postio-client/src/api.rs`). Make T043 green; land it as its own PR

### FFI and Mac: keys, selection, the action bar, undo

- [ ] T045 [US2] Write failing ffi_suite tests:
  - `key("x", …)` returns `handled` with a `Selection` intent;
  - `key("a", …)` with a selection returns a `Toast` intent through `UiEvent::Intents`;
  - `undo_description()` returns the toast's words;
  - `key("j", …, in_text_entry: true)` is not handled.
- [ ] T046 [US2] Replace `Session::key -> KeyOutcomeFfi` with `key(char, name, ModifiersFfi, in_text_entry) -> KeyPressFfi{handled, pending, intents}` driven by the controller's `press`. Export `undo_description()` in `crates/postio-ffi/src/session.rs`. Make T045 green
- [ ] T047 [P] [US2] Write storyboards `storyboards/list/x-selects-without-moving.toml`, `storyboards/list/has-action-keeps-the-cursor.toml` and `storyboards/list/undo-after-the-pill-is-gone.toml` (`apps = ["focus"]`). Run them on Linux
- [ ] T048 [US2] Write failing Swift tests:
  - `macos/Tests/PostioKitTests/IntentApplierTests.swift`: applying `Cursor`, `Selection`, `Toast` and `KeyboardHome` to a fake main-window model changes exactly what the intent says;
  - `macos/Tests/PostioAppKitTests/UndoManagerTests.swift`: `PostioUndoManager.canUndo` is false with no description; with a description `undoMenuItemTitle` is "Undo <description>"; `undo()` invokes `undo`; `canRedo` is always false.
- [ ] T049 [US2] Implement `IntentApplier` in `macos/Sources/PostioKit/IntentApplier.swift`: the one switch over `IntentFfi`, on the main actor. The unknown-intent arm logs the name and does nothing
- [ ] T050 [US2] Rework `macos/Sources/Postio/KeyMonitor.swift` to call the new `key(…)`: swallow when `handled`, show `pending` as the chord hint, and keep the IME and `TypingResponder` guards. Remove the `UiContext` argument, because the context is the controller's
- [ ] T051 [US2] Implement `PostioUndoManager` (an `NSUndoManager` subclass) in `macos/Sources/PostioAppKit/PostioUndoManager.swift`:
  - it caches `undo_description()`, refreshed on each `Toast`/`Notice` intent;
  - the main window's delegate returns it from `windowWillReturnUndoManager`;
  - text fields keep the field editor's own manager.

  Make T048 green
- [ ] T052 [US2] Implement the action bar (SwiftUI) in `macos/Sources/PostioKit/ActionBar.swift`: the count, the actions with keycaps (Archive `a`, Snooze `s`, Mark read `r`, Digest these… `d`, Task `t` only with a vault (C9), Label `l`, Move `m`), and the hints on the right. Add a minimal undo notice line (the pill proper is T093)
- [ ] T053 [US2] Compare screen 03 (FR-061): `scripts/macos-shot.sh 03 --both` against `03-inbox-has-action-filter.png`. Record in `docs/notes/<date>-focus-macos-phase-2.md`

---

## Phase 5: User Story 3 — one message in its own window (P1)

**Goal**: screen 04 and the message window's design: geometry (M1), the
chrome, one column, the rhythm, and both treatments (M5).

**Independent test**: open a plain message, a newsletter and a work message
in light and dark, with main widths 1440 and 1024. Each matches its
`message-window/screens/` PNG.

### Controller slice 5: key routing and the surface stack (**main·S5**)

- [ ] T054 [US3] Write failing tests in `crates/postio-focus/tests/keys.rs`:
  - the key context comes from the top surface;
  - Reader context routes Back to `CloseMore`, then `CloseFind`, then close;
  - `j`/`k` in Reader step the list;
  - `[`/`]` step the thread;
  - with `stacking = false`, opening a Digest while a Message is open replaces it (contract invariant 6);
  - Back on the list follows the ladder: places, then the bar, then Filtered, then clear (R2).
- [ ] T055 [US3] Move `handle_key` (`crates/postio-gtk/src/window.rs:803-946`), `key_context` (990-1016), `reading_key` (1117-1231), `digest_key` (~3201), `capture_key`, `settings_key` (`settings.rs:486`) and the dialog close rule (896-907) into `crates/postio-focus/src/keys.rs` and `surfaces.rs`, as tables over the surface stack. Keep the `_ => Proceed` arm (behaviour-neutral). GTK reports `SurfaceOpened`/`SurfaceClosed`; `FocusWindow::handle_key` stays as a shim. The guards are focus_suite `open_keys`, `keymap`, `one_keymap`, `registry_parity`, `every_command`, `settings` and `row_menu`. Make T054 green; land slice 5
- [ ] T056 [US3] Write a failing test in `crates/postio-focus/tests/keys.rs` and a failing focus_suite case `crates/postio-gtk/tests/focus_suite/open_keys.rs::t_in_the_open_message_opens_capture`: `t`, `n` and `d` in the open-message dialog open capture, note and the digest rule (#1754)
- [ ] T057 [US3] Make Reader-context commands the surface does not own fall through to the list's command table, in `crates/postio-focus/src/keys.rs` (contract invariant 5). Make T056 green. Land as **main·fix**, and comment the fix on #1754 with `Refs` (not a closing keyword unless #1754's acceptance is fully met)

### Controller slice 6: the email window (**main·S6**)

- [ ] T058 [US3] Write failing tests in `crates/postio-focus/tests/reader.rs`:
  - ↩ emits `OpenMessage{message, row, position, origin: List}`;
  - `j` in the message emits `OpenMessage` for the next row, without a `CloseMessage`;
  - Archive in the message moves on to the next row (T190 behaviour);
  - a message opened from Found hits steps through the hits;
  - Esc emits `CloseMessage` + `KeyboardHome(CursorRow)` with the selection kept;
  - `step_past` gives up after the timer rather than polling.
- [ ] T059 [US3] Move `open_message`, the position line, `follow_cursor`, `reading_changed`, `place_reading`, found-hits stepping and `step_past` (`crates/postio-gtk/src/window.rs:1237-1280, 4230-4259`) into `crates/postio-focus/src/reader.rs`. Replace the 20 ms polling with an event plus a `Timer` give-up. The guards are focus_suite `open_message`, `open_reading`, `reading_pane`, `read_on_dwell` and `view_source`. Make T058 green; land slice 6

### Shared: the treated document and the contrast guard (**main·fix**)

- [ ] T060 [P] [US3] Write failing tests in `crates/postio-body/src/treatment.rs` for `guard_kept_colours(html, surfaces: [light, dark]) -> String`: a kept inline red on white passes 4.5:1 and stays; a kept `#777` fails in dark and is dropped or scoped under `prefers-color-scheme: light`; nothing changes in paper. Also a shared-corpus test asserting the same kept or dropped decision as `postio_render::theme::guard` for the app-colours fixtures in `crates/postio-model/tests/corpus/` (it runs where postio-render builds)
- [ ] T061 [US3] Implement `guard_kept_colours` beside `app_colours` in `crates/postio-body/src/treatment.rs`, using the surface tokens from `postio-ui/data/treatment.css`. Make T060 green

### FFI: the reader document

- [ ] T062 [US3] Write failing ffi_suite tests:
  - `reader_document(newsletter)` returns `treatment_shown = Paper` and `classified = Paper`, with `render_mode_words` naming it;
  - `switch_treatment` flips it;
  - `always_treatment(sender, AppColours)` persists to the allowlist file and applies to the next message from that sender;
  - `raw_source(id)` returns the bytes;
  - `column_width` follows M1 for the given window width.
- [ ] T063 [US3] Move the FFI reader (`crates/postio-ffi/src/session.rs` ~5420-5544, `reader_answers`) to `postio_ui::reader::document::prepare_treated` / `document_for_treated`, with `guard_kept_colours` applied in app colours. Extend `ReaderDocumentFfi` (treatment_shown, treatment_classified, render_mode_words, sender_choice, column_width, paper_floor). Export `switch_treatment`, `always_treatment`, `treatment_css`, `raw_source`. Delete `thread_document`. Make T062 green

### Mac: the message window

- [ ] T064 [P] [US3] Write storyboards `storyboards/reader/esc-returns-to-the-same-row.toml`, `storyboards/reader/j-steps-the-list-without-resizing.toml` and `storyboards/reader/shift-o-switches-treatment.toml` (`apps = ["focus"]`). Run them on Linux
- [ ] T065 [US3] Write failing Swift tests in `macos/Tests/PostioAppKitTests/SecondaryWindowTests.swift`:
  - opening a second kind closes the first (M4);
  - the frame is centred on the main window with the width from the FFI's geometry;
  - close emits the `SurfaceClosed` fact;
  - closing the main window closes the secondary window.
- [ ] T066 [US3] Implement `SecondaryWindowController` in `macos/Sources/PostioAppKit/SecondaryWindowController.swift`: a titled child `NSWindow` of the main window, one at a time, centred, sized from the geometry. ⌘W and Esc close it; on close it sends `ui_fact(SurfaceClosed)` and the main window makes the table first responder. Make T065 green
- [ ] T067 [US3] Implement the message window's chrome in `macos/Sources/PostioKit/MessageWindowView.swift` (SwiftUI) per `message-window/SPEC.md` §2:
  - a 52 pt title area: subject 13.5 bold, the position line in tertiary, then on the right the `k j` keycap and the joined ↑/↓ pair (30×28, radius 6, 1 px divider);
  - a 44 pt action row: Reply `e`, Reply all `E`, Forward `f`, Archive `a`, Snooze `s`, Remind `h`, Label `l`, Move `m`, Delete `⌫`. Label, Move and Delete fold into More `.` below 700.
- [ ] T068 [US3] Implement the content column in `macos/Sources/PostioKit/MessageContentView.swift` per `message-window/SPEC.md` §3-5:
  - the thread marker; the subject at 26/32 bold; labels; the sender block grid (44 pt label column, monospaced address and date);
  - the action card (accent 8% light, 12% dark, C26) with Snooze `s` and Dismiss `-`;
  - the render-mode line; attachments;
  - the §4 rhythm table as constants in one place.
- [ ] T069 [US3] Rework the body web view in `macos/Sources/PostioAppKit/ReaderView.swift` and `macos/Sources/PostioKit/ReaderConfiguration.swift`:
  - JS off, a non-persistent store, `loadHTMLString(_, baseURL: nil)`;
  - a `WKContentRuleList` blocking every load not on `postio-cid:`/`postio-reader:`;
  - app colours via `treatment_css()`;
  - paper with light appearance forced, white `underPageBackgroundColor`, a radius 6 sheet with a hairline, `brightness(0.92)` in dark, and fit zoom ≥ `paper_floor`, measured with `evaluateJavaScript` on the main actor only;
  - the action sentence highlighted (accent 8%, 2 pt underline).

  Add a test in `macos/Tests/PostioAppKitTests/ReaderEgressTests.swift` that a remote `<img>` makes no request
- [ ] T070 [US3] Wire `OpenMessage`, `CloseMessage` and `Reader(verb)` in `IntentApplier`, and `v` raw source as an in-place replacement of the content (M4)
- [ ] T071 [US3] Compare screen 04 and all nine `message-window/screens/*.png` (FR-061): capture at main width 1440 and 1024, in light and dark. Check the 1024 numbers: the window 656, the plain column 560, a paper newsletter at 0.9. Record in `docs/notes/<date>-focus-macos-phase-3.md`

---

## Phase 6: User Story 4 — write and reply (P2)

**Goal**: screens 05 and 06.

**Independent test**: reply to a question row, press Esc, reopen the draft,
send it. It goes through the outbox, and the marker clears.

### Controller slice 7: compose (**main·S7**)

- [ ] T072 [US4] Write failing tests in `crates/postio-focus/tests/compose.rs`:
  - `c`, `e`, `E` and `f` emit `Composer(New|Reply|ReplyAll|Forward)` (on the Mac replacing any open secondary surface);
  - `refuses_reply` cases;
  - the autosave debounce: edits within 1500 ms coalesce into one save, and Esc saves now and emits `Toast("Draft saved locally …")`;
  - `settle_send` cancel, retry and mark sent.
- [ ] T073 [US4] Move `refuses_reply`, `OpenDraft`, `offered_on_open_draft`, `settle_send` and the add-account offer (`crates/postio-gtk/src/window.rs` ~1677-1800), plus the autosave and generation state machine from `crates/postio-widgets/src/composer/mod.rs:101-563, 722-724`, into `crates/postio-focus/src/compose.rs`, as `Input::ComposerEdited`/`Effect::Timer`. The guards are focus_suite `compose`, `drafts`, `one_composer`, `sending_states` and `remind_on_send`. Make T072 green; land slice 7

### FFI and Mac: compose

- [ ] T074 [US4] Write failing ffi_suite tests for `recipient_suggestions(account, "ad", 8, extra: [ExternalContactFfi{"Ada Example","ada@example.com"}])`: a correspondent the person wrote to ranks above the extra; a group completes by name; suppressed contacts never appear
- [ ] T075 [US4] Export `recipient_suggestions` in `crates/postio-ffi/src/contacts.rs` over `postio_ui::recipients::suggest`, with the directory cached as `FinderSources` is. Make T074 green
- [ ] T076 [P] [US4] Write the storyboard `storyboards/compose/esc-keeps-the-draft.toml` (`apps = ["focus"]`), and run it on Linux
- [ ] T077 [US4] Write failing Swift tests in `macos/Tests/PostioKitTests/ContactsSourceTests.swift`: with authorization `.notDetermined` the first recipient keystroke requests access once; with `.denied` it never asks again and suggestions still come from mail. Use a fake `CNContactStore` protocol
- [ ] T078 [US4] Implement `ContactsSource` in `macos/Sources/PostioKit/ContactsSource.swift`: `CNContactStore` read, after one prompt, of names and email addresses only; never stored or logged. Add `NSContactsUsageDescription` to `macos/Resources/Info.plist`. Make T077 green
- [ ] T079 [US4] Rebuild the composer's frame in its secondary window in `macos/Sources/PostioAppKit/ComposeView.swift` and `macos/Sources/PostioKit/ComposeModel.swift`:
  - fields From (account picker), To, Cc, Bcc, Subject and Labels;
  - a footer with Attach ⌘⇧A, Remind if no reply ⌘H (shows the date when on), the word count, Send later ▾, and Send ⌘↩ (a `labelColor`-filled default button);
  - the reply: pre-filled, quoted text folded, and no contact list until typing.

  Keep the existing `ComposeEditor` WKWebView; `editor.js` stays byte-identical to GTK's (drift test)
- [ ] T080 [US4] Compare screens 05 and 06 (FR-061), recorded in `docs/notes/<date>-focus-macos-phase-4.md`. ⌘T, ⌘⇧B and ⌘⇧Q are M3 (Phase 12)

---

## Phase 7: User Story 5 — find and go anywhere (P2)

**Goal**: screens 07 to 10.

**Independent test**: type "from ada last week with attachments", see the
chips, open a result, go to `in:Receipts`, and pick a label from the popover.

### Controller slice 8: the bar, search, go-to and places (**main·S8**)

- [ ] T081 [US5] Write failing tests in `crates/postio-focus/tests/bar.rs`:
  - `/` emits `OpenBar(Search)`, and ⌘K `OpenBar(Commands)` with `>` (C24);
  - Tab into the chips, and ⌘⌫ back to words (from `bar.rs:527-590`);
  - a command chosen in the bar acts on the aim captured at open;
  - `in:Receipts` opens that folder as a list with the cursor on row 0;
  - `g i`, `g t`, `g s`, `g z`, `g r` and `g f` go to their places;
  - ⌥1–4 open saved searches and ⌘S saves;
  - the places read is one `Request::Places`, not N×M calls.
- [ ] T082 [US5] Move `go_to`, `go_to_inbox`, `go_to_role`, `go_to_view`, the saved searches, `bar_action`, reopen-after-hit, the chip editor (`crates/postio-gtk/src/bar.rs:527-590`) and the places reads (`crates/postio-gtk/src/places.rs:400-470`) into `crates/postio-focus/src/bar.rs`. Implement `perform` for `Places` once. The guards are focus_suite `bar`, `places` and `cursor` (`a_folder_from_the_popover…`). Make T081 green, and clear `GoToArchive`/`GoToSnoozed`/`SavedSearch1-4` from `KNOWN_ORPHANS`. Land slice 8

### Mac: the bar and the folders popover

- [ ] T083 [P] [US5] Write storyboards `storyboards/search/slash-opens-search-cmd-k-opens-commands.toml` and `storyboards/search/tab-enters-the-chips.toml` (`apps = ["focus"]`). Run them on Linux
- [ ] T084 [US5] Write failing Swift tests in `macos/Tests/PostioKitTests/CommandBarModelTests.swift`: results from `BarText` intents become one-line rows with keycaps; typing forwards `Typed{Bar}` facts; Esc emits `CloseBar`
- [ ] T085 [US5] Implement `CommandBarPanel` in `macos/Sources/PostioAppKit/CommandBarPanel.swift`: a borderless, non-activating child `NSPanel` under the `NSSearchToolbarItem`, as wide as the field or 640, whichever is wider, with no dimming. The keyboard stays in the field. Its SwiftUI content goes in `macos/Sources/PostioKit/CommandBarView.swift`, rendering the chips. Make T084 green
- [ ] T086 [US5] Implement the folders and labels popover in `macos/Sources/PostioAppKit/PlacesPopover.swift`: an `NSPopover` anchored to Inbox ▾, with SwiftUI content listing mailboxes, folders and labels with counts, filtered as you type; ↩ opens the place
- [ ] T087 [US5] Compare screens 07 to 10 (FR-061), recorded in `docs/notes/<date>-focus-macos-phase-5.md`

---

## Phase 8: User Story 6 — pickers at the row, and the undo pill (P2)

**Goal**: screens 11 to 15.

**Independent test**: snooze to a typed "tue 9am", label two rows, move one
to a folder, and undo each with the pill and ⌘Z.

### Controller slice 9: pickers, the row menu, toast policy (**main·S9**)

- [ ] T088 [US6] Write failing tests in `crates/postio-focus/tests/pickers.rs`:
  - `s`, `h`, `l` and `m` emit `OpenPicker{kind, anchor: Row(pos), aim}`, or `anchor: OpenMessage` from the email window;
  - a number key picks a preset;
  - Tab plus typed "tue 9am" resolves through `postio_search::date::parse_when` to the same instant on both platforms;
  - Label toggles with Space and creates a label;
  - Move offers recent folders as `1` and `2`;
  - toast policy: a new toast replaces the old, the timeout is 8 s, and the undo of a queued send runs first.
- [ ] T089 [US6] Move `open_when`, `open_labels`, `open_moves`, the picker-chosen → `Send` path and `row_menu_alone` (`crates/postio-gtk/src/window.rs:4821-5077`, `move_picker.rs`, `label_picker.rs`), plus the toast policy from `crates/postio-widgets/src/widgets/toast.rs` (`activate_undo` 244, `show_action_completed_for` 155, `rehome` 378), into `crates/postio-focus/src/pickers.rs`. The guards are focus_suite `pickers`, `row_menu` and `undo`. Make T088 green; land slice 9

### Mac: the popovers and the pill

- [ ] T090 [P] [US6] Write storyboards `storyboards/list/snooze-typed-date.toml` and `storyboards/list/label-two-with-space.toml` (`apps = ["focus"]`). Run them on Linux
- [ ] T091 [US6] Write failing Swift tests in `macos/Tests/PostioKitTests/PickerModelTests.swift` for the four picker models driven by `OpenPicker` and `Typed{Picker}`: presets numbered 1–4, the typed field's parsed preview from the FFI, the Space toggle, ↩ confirms, Esc closes
- [ ] T092 [US6] Implement the picker popovers in `macos/Sources/PostioAppKit/PickerPopover.swift` (`.transient`, anchored to the cursor row's rect in `FocusListTable`, or to the action button in the message window), with SwiftUI content in `macos/Sources/PostioKit/PickerViews.swift`. Make T091 green
- [ ] T093 [US6] Implement the undo pill in `macos/Sources/PostioKit/UndoPill.swift`: a SwiftUI overlay at the bottom centre with the text, an Undo button and a ⌘Z keycap, fading after the toast's seconds. It replaces T052's minimal line
- [ ] T094 [US6] Compare screens 11 to 15 (FR-061), recorded in `docs/notes/<date>-focus-macos-phase-6.md`

---

## Phase 9: User Story 7 — the app says what state it is in (P2)

**Goal**: screens 16 to 19.

**Independent test**: an empty store, the network off, a revoked password.

### Controller slice 10: states (**main·S10**)

- [ ] T095 [US7] Write failing tests in `crates/postio-focus/tests/states.rs`:
  - `ConnectionChanged`/`SyncProgress`/`BackfillProgress` produce `Banner` and `SyncLabel` intents per `postio_ui::focus_state::banner`;
  - an empty inbox produces `EmptyOrList(Some(page))` with only the shortcuts that exist;
  - a sign-in failure produces the banner with `UpdateCredential`.
- [ ] T096 [US7] Move `hear_sync`, `note_account`, `show_state` and `announce` (`crates/postio-gtk/src/window.rs` ~3748-3803, 3048) into `crates/postio-focus/src/states.rs`. The guards are focus_suite `state`, `starts_offline`, `idle_passes` and `empty`. Make T095 green; land slice 10

### FFI and Mac: states

- [ ] T097 [US7] Write a failing ffi_suite test: `start_over(store_path)` on a store marked from another build leaves a fresh store that `Session::open_at` opens
- [ ] T098 [US7] Export `start_over(store_path)` over `postio_session::start_over_at` in `crates/postio-ffi/src/lib.rs`, with the store key from the keyring. Make T097 green
- [ ] T099 [US7] Append a typed `UiEvent::BackfillProgress{account, done, total}` (`crates/postio-ffi/src/event.rs`), then implement the banner strip and the empty state in `macos/Sources/PostioKit/BannerStrip.swift` and `EmptyInbox.swift`:
  - full width under the header strip; the error strip in `systemRed` at low opacity;
  - Retry, and "Update password…" opening a sheet that stores through the engine's credential store (Keychain), reusing `AccountRepair.swift`.
- [ ] T100 [US7] Show the store refusal on launch with "Start over" calling `start_over`, in `macos/Sources/Postio/StoreRefusal.swift`
- [ ] T101 [US7] Compare screens 16 to 19 (FR-061), recorded in `docs/notes/<date>-focus-macos-phase-7.md`

---

## Phase 10: User Story 8 — the key map teaches the keys (P3)

**Goal**: screen 20 and the menu bar.

**Independent test**: rebind one command in `[keys]`; `?` and the menu bar
show the new key without a restart.

### Controller slice 11: keymap reload and the key map (**main·S11**)

- [ ] T102 [US8] Write failing tests in `crates/postio-focus/tests/keymap.rs`: an `Input::Keymap` rebuilds the resolver; the new binding resolves; `?` toggles `OpenKeyMap`/`CloseTop`
- [ ] T103 [US8] Move `set_keymap` (`crates/postio-gtk/src/window.rs:716`) and the CheatSheet toggle into `crates/postio-focus/src/keys.rs`. The guards are focus_suite `keycaps` and `keymap`. Make T102 green; land slice 11

### Mac: the key map and the menu bar

- [ ] T104 [US8] Write failing Swift tests in `macos/Tests/PostioKitTests/MenuPlanTests.swift`: every command offered on Apple for Focus has a menu item with the key `bindingsFor` returns; after `KeymapChanged` the plan is rebuilt with the new key
- [ ] T105 [US8] Rebuild `macos/Sources/PostioKit/MenuPlan.swift` and `macos/Sources/PostioAppKit/MenuBar.swift` for Focus's menus (`menus()` for Focus on Apple). Add the standard App/File/Edit/Window items (Settings ⌘,, Quit ⌘Q, New Message ⌘N, Close ⌘W); Edit › Undo backed by `PostioUndoManager`; rebuild on `KeymapChanged`. Fix the stale `ctrl+…` comment at `MenuBar.swift:97`. Make T104 green
- [ ] T106 [US8] Implement the key map sheet in `macos/Sources/PostioKit/KeyMapSheet.swift`: groups from `postio_ui::keymap_sheet` via the FFI's `cheat_sheet_sections`, and a footer naming `~/Library/Application Support/Postio/config.toml`, `[keys]` (C3). Delete `macos/Sources/Postio/Palette.swift`'s `CheatSheet` once replaced
- [ ] T107 [US8] Compare screen 20 (FR-061), recorded in `docs/notes/<date>-focus-macos-phase-8.md`

---

## Phase 11: User Story 9 — filtered, digests and capture (P3)

**Goal**: screens 21 to 25, and `postio://`.

**Independent test**: walk each surface over the demo store, with a model
stub and a test vault. Opening a captured line's `postio://` link from
another app brings Postio forward on that message.

### Controller slice 12: the filtered, digest, rules and capture sub-states (**main·S12**)

- [ ] T108 [US9] Write failing tests in `crates/postio-focus/tests/digest.rs`, `filtered.rs` and `capture.rs`:
  - **filtered:** `g f` shows Filtered with tabs `1`–`7`; `R` sends `RestoreFiltered`.
  - **digest:** ↩ on a digest row opens the digest on its summary when a model is configured, else on its list (C6); `]`/`[` step the references; ↩ opens the reference's email with `host: DigestWindow`; Esc returns to the same reference; ⇧A archives all; `D` stops digesting with a `Confirm`.
  - **rules and capture:** `d` opens the rule dialog pre-filled; `t`/`n` open capture only with a vault (C9); ⌥S swaps in the subject.
  - **On the Mac (`stacking = false`):** a message from the digest's list opens in the digest window (M4).
- [ ] T109 [US9] Move the per-surface state of `crates/postio-gtk/src/digest.rs` (83-112), `filtered.rs` (52-67), `capture.rs` (52-84), `rules.rs` and `rule_dialog.rs`, and `window.rs`'s digest, filtered and rules handlers (3133-3722), into `crates/postio-focus/src/{digest,filtered,capture}.rs`. The guards are focus_suite `filtered`, `digest`, `digest_summary`, `capture`, `rule_query` and `unsubscribe`. Make T108 green; land slice 12

### FFI: digest, vault, capture and links

- [ ] T110 [US9] Write failing ffi_suite tests:
  - `digest_summary(delivery)` over the seed with a model stub returns statements with numbered references;
  - `vault(subject)` with no `[focus.vault]` fails with the configured sentence;
  - `capture_task` writes the exact line with the `postio://` link before the date (C21);
  - `parse_message_link("postio://message/42/")` is 42; `"postio://message/0"` is None.
- [ ] T111 [US9] Export the digest, rule, vault and capture reads Swift draws directly (contracts/ffi-focus.md "Reads"), with `*Ffi` mirrors in `crates/postio-ffi/src/focus.rs`. Add `message_link`, `parse_message_link`, `link_unknown` and `link_gone` in `crates/postio-ffi/src/links.rs` over `postio_ui::links`. Make T110 green

### Mac: the surfaces

- [ ] T112 [P] [US9] Write storyboards `storyboards/flows/digest-reference-and-back.toml`, `storyboards/flows/restore-from-filtered.toml` and `storyboards/flows/capture-task-from-message.toml` (`apps = ["focus"]`). Run them on Linux
- [ ] T113 [US9] Implement the Filtered view in `macos/Sources/PostioKit/FilteredView.swift`: it replaces the list in the main window, with the reason per row, tabs `1`–`7` with counts, `R` restore, and the header "Nothing here is deleted automatically" (C4)
- [ ] T114 [US9] Implement the digest window in `macos/Sources/PostioKit/DigestView.swift`, hosted by `SecondaryWindowController` at the message window's size with a 560 column (M1):
  - the summary by topic with numbered references; the list page on Tab;
  - the reference's email opens in place with "‹ Summary" `Esc`, the citation banner and the highlighted passage;
  - `U` unsubscribes and `D` stops digesting.
- [ ] T115 [US9] Implement the digest-this-sender sheet in `macos/Sources/PostioKit/DigestRuleSheet.swift`: pre-filled sender; daily, weekly or monthly with day and time; a preview of what it would have caught; "Match a list or a search instead…"
- [ ] T116 [US9] Implement the capture window in `macos/Sources/PostioKit/CaptureView.swift`:
  - the task text verbatim, with ⌥S for the subject;
  - the due date with quick picks;
  - the suggested project with its reason, and ⌘P for the picker;
  - the exact line as a preview;
  - ⌘↩ writes it (Add task is a `labelColor`-filled default button).
- [ ] T117 [US9] Register `postio` in `macos/Resources/Info.plist` (a second `CFBundleURLTypes` dict). Route it in `macos/Sources/Postio/URLHandling.swift`: `parse_message_link` leads to `command("open_message_by_id")`, or a pill with `link_unknown`/`link_gone`. Add a Swift test in `macos/Tests/PostioKitTests/LinkRoutingTests.swift`
- [ ] T131 [US9] Give the Mac's settings window Focus's nav: export `[focus]` reads and patches over `postio_ui::settings` (as GTK's Filtering pane uses), draw the Filtering pane in `macos/Sources/PostioKit/SettingsPaneView.swift`, drop Appearance, and point `settings_sections` (`crates/postio-ffi/src/settings.rs`) at `crate::FRONTEND`. Update `SettingsStoreTests` and `ffi_suite/settings.rs` (Filtering in, Appearance out)
- [ ] T118 [US9] Compare screens 21 to 25 (FR-061), recorded in `docs/notes/<date>-focus-macos-phase-9.md`

---

## Phase 12: User Story 10 — both apps gain what the Mac design adds (P4)

**Goal**: M3, built once in the shared layers and drawn by both apps. Each
lands on `main` with its GTK half (**main·fix** each), then the Mac half on
the branch.

- [ ] T119 [US10] Write a failing test in `crates/postio-ui/src/focus_row.rs` for the row's "Task in <project> · due <day>" line from an open vault task linked to the message (spec 007 FR-181). Implement it, and draw it in `crates/postio-gtk/src/list/row.rs` and `FocusListTable`
- [ ] T120 [P] [US10] Write a failing controller test, then add Task after sending (⌘T) as a registry command and a composer toggle: after send, capture opens on the sent message. Add it to GTK's composer footer and the Mac's
- [ ] T121 [P] [US10] Split `copy_fields` into Cc (`mod+shift+c`) and Bcc (`mod+shift+b`) in `crates/postio-core/src/registry.rs`, with tests in `one_keymap.rs` for both platforms. Wire both composers
- [ ] T122 [P] [US10] Add a "show quoted text" command (`mod+shift+q`) to the registry and both composers, test first
- [ ] T123 [US10] Show the focused reference's email under its paragraph in the digest summary: controller test, `crates/postio-ui/src/digest.rs`, GTK `digest.rs`, Mac `DigestView.swift`
- [ ] T124 [US10] Give the email opened from a digest reference its action row (Reply, Forward, Archive, Note, Label, Unsubscribe, Stop digesting): controller test, both apps

---

## Phase 13: Polish and landing

- [ ] T125 [P] Regenerate `docs/keybindings.md` and check the "(not macOS)" annotations are gone (`crates/postio-ui/tests/ui_suite/keybindings_doc.rs`)
- [ ] T126 [P] Update `macos/CLAUDE.md` for the Focus app: the targets, the intent applier, `macos-shot.sh`, and the no-AppKit rule in PostioKit
- [ ] T127 [P] Add a dated note `docs/notes/<date>-focus-on-the-mac.md` listing the constraints future sessions must respect: one secondary window, the intents not the widgets, the treatment from the shared document. Add it to `docs/archive/engineering-notes.md`
- [ ] T128 Run quickstart.md's scenarios table end to end on the bundle over the demo store, then on real mail (the maintainer). Record the results on the PR
- [ ] T129 Check `specs/009-focus-macos/spec.md` success criteria SC-001 to SC-007 one by one, and write the evidence for each into the PR body
- [ ] T130 Land `feature/focus-macos` once: rebase onto `main`, then `scripts/issue-land.sh --detach --full-suite`, with the PR labelled `interactions-unreviewed` (FR-063). The PR body says what this spec closes and that the Mac storyboard runner is spec 008's later phase

---

## Dependencies and execution order

```text
Setup (T001–T006) ─▶ Foundational (T007–T022) ─▶ US1 (T023–T036) ─▶ US2 (T037–T053) ─▶ US3 (T054–T071)
                                                                                        │
            ┌──────────────────────────────┬────────────────────┬───────────────────────┤
            ▼                              ▼                    ▼                       ▼
     US4 (T072–T080)                US5 (T081–T087)      US6 (T088–T094)        US7 (T095–T101)
            └──────────────┬───────────────┴────────────────────┴───────────────────────┘
                           ▼
                    US8 (T102–T107) ─▶ US9 (T108–T118) ─▶ US10 (T119–T124) ─▶ Polish (T125–T130)
```

- **Slices before the Mac.** Controller slice *n* lands on `main` before the
  Mac tasks of its story. The branch is rebased onto `main` after each slice.
- **Slices 1–6 must go in order:** the feed, then cursor, verbs, keys and the
  reader all build on the earlier ones. Slices 7–12 depend only on 5, and
  can go in any order, or in parallel by separate sessions on separate
  `focus-controller/*` branches.
- **US4–US7** depend on US3's `SecondaryWindowController` (US4) and on US2's
  intent applier. They are otherwise independent of each other.
- **US1 is the MVP.** The Mac shows the Focus inbox over the real engine,
  with the engine's Focus pass on.

## Parallel examples

- **Setup:** T002, T003 and T004 together; T005 and T006 go in slice 1's PR.
- **Foundational:** T018 (Delete on ⌫) beside T010–T017; T021 and T022
  beside T019 and T020.
- **US1:** T029 (storyboard) beside T030–T031 (row model).
- **US3:** T060–T061 (contrast guard, `postio-body`) beside T054–T059
  (controller slices 5–6).
- **US10:** T120, T121 and T122 together.

## Implementation strategy

1. **MVP first.** Setup, Foundational and US1 make a Mac app that opens on
   the Focus inbox with markers and digests from the engine. Show it to the
   maintainer before going further.
2. **Then the triage loop**, US2 and US3: keyboard, undo and reading. That is
   the product.
3. **Then breadth**, US4–US9, in the brief's order. Each phase ends with its
   PNG comparison note.
4. **M3 extras** (US10) only after parity, on both apps.
5. **Landing.** Throughout, the controller slices keep `main` and the branch
   close. The branch lands once (T130).
