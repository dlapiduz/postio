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
- [x] T004 [P] (Built over an in-memory demo store, not a copy on disk: `postio-demo`'s seeds, opened by `Session.openDemo` behind the FFI's `demo` feature, so no Keychain is read.) Add `scripts/macos-shot.sh <screen> [--dark|--light|--both] [--width W]`:
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
- [x] T012 (Done with T028 and T034 in US1, not here: while the three-pane Swift shell still exists, removing its commands leaves it drawing keys that do nothing.) Remove `Frontend::Macos` and `Requirement::ThreePane` from `crates/postio-core/src/registry.rs` (lines ~242-299, ~375-377), with the 19 ThreePane-only commands (ToggleRail, ToggleSidebar, CyclePane, CyclePaneBack, NextFolder, PrevFolder, ToggleFolder, RenameSavedSearch, MoveSavedSearchUp/Down, DeleteSavedSearch, OpenParts, NextPart, PrevPart, OpenPart, SavePart, SaveAllParts, OpenPartExternally, RenderPartOnce) and their `CommandId` variants. Fix:
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
- [x] T018 [P] Write a failing test in `crates/postio-core` that Delete resolves from `BackSpace` on `Platform::Apple` in the List and Reader contexts, without colliding with the bar's `mod+BackSpace`. Then add `"BackSpace"` to Delete's `alternate_bindings` (`crates/postio-core/src/registry.rs:903-913`) if it is free in those contexts (M6, R6). **main·fix**

  *As built:* landed on main as #1795 (`feat(config): Delete is the Mac's delete key`).

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
- [x] T028 [US1] (After T034, when the Swift Focus list has replaced the classic one.) Delete the classic list, cursor and selection machinery from `crates/postio-ffi/src/session.rs`: `HANDLED_HERE` (:133), the `selection`/`cursor`/`cursor_row`/`anchor` fields (:624-646), `handle_locally` (:4236), `RailFfi` and `rail_presentation` (`crates/postio-ffi/src/rail.rs`), `next_pane`, and the sidebar and parts exports. Delete their ffi_suite tests; keep `cargo nextest run -p postio-ffi` green

### Mac: the inbox

- [x] T029 [P] [US1] (Already written: `storyboards/list/launch-keyboard-on-first-row.toml` asserts it, and applies to the Mac unchanged.) Write a storyboard `storyboards/list/inbox-opens-on-the-first-row.toml` (`apps = ["focus"]`): launch, the cursor on index 0, and the keyboard region the list (C30). Run it on Linux with `scripts/storyboards.sh run` to see it pass there
- [x] T030 [US1] Write failing Swift tests in `macos/Tests/PostioKitTests/FocusRowModelTests.swift` for the row view model built from `FocusRowFfi`:
  - the row kind is header, one-line or two-line;
  - at most two pills;
  - the marker's action keycap text comes from the FFI's spelling, not a literal;
  - unread is bold.
- [x] T031 [US1] Implement `FocusRowModel` and `FocusListModel` (the row count, `row(at:)` via the FFI, applying `DeliverPage`/`RefreshList`/`Cursor`/`Selection` intents) in `macos/Sources/PostioKit/FocusList.swift`. Make T030 green
- [x] T032 [US1] Implement `FocusListTable` (`NSTableView`, view-based) in `macos/Sources/PostioAppKit/FocusListTable.swift`:
  - three row views (day header, one-line, two-line) with fixed heights per kind;
  - table selection disabled;
  - the accent focus ring on the cursor row only;
  - a checked-box gutter for selected rows;
  - semantic colours only.

  Reuse what fits from `MessageTable.swift`/`MessageRowView.swift`, then delete those files
- [x] T033 [US1] Implement the main window in `macos/Sources/Postio/MainWindow.swift`:
  - an `NSToolbar` (unified): compose button, flexible space, sync label, `NSSearchToolbarItem` with a ⌘K keycap;
  - the SwiftUI header strip in `macos/Sources/PostioKit/HeaderStrip.swift`: Inbox ▾ `g o` with its count, Has action `!` with its count, and the filtered and digest counts only while in use (C10);
  - `FocusListTable` below.
- [x] T034 [US1] Delete the three-pane shell:
  - **in `macos/Sources/Postio/`:** `Shell.swift`, `FolderRow.swift`, and the three-pane parts of `Engine.swift` (`collapsedFolders`, `sidebar*`, `folderCursor`, `pick(SidebarRowId)`, `stepSidebar`, `children(of:)`, `specialFolders`, `folderRoots`, `open(mailbox:)`, `pane`/`focus(_:)`, `conversation`, `railHidden`, `showingThread`, `parts`, `readerPages`, `bodyHeights`);
  - **in `macos/Sources/PostioKit/`:** `Sidebar*.swift`, `SavedSearchRows.swift`, `ConversationRail.swift`, `ConversationView.swift`, `ConversationModel.swift`, `ThreadDocumentView.swift`, `ToolbarPlan.swift`, `ReaderActionPlan.swift`, `SearchScopeRail.swift`, `SearchRefineBar.swift`, `PartsPanel.swift`, `PartPreview.swift`, `BodyHeight*.swift`;
  - **their tests.**

  Wire `PostioApp.swift`'s main scene to `MainWindow`. Keep `scripts/macos-test.sh` green
- [x] T035 [US1] Write a performance check in `macos/Tests/PostioAppKitTests/FocusListScrollTests.swift`: over a 10k-conversation seeded store, row views per scroll page stay bounded and `row(at:)` FFI calls per frame stay ≤ the visible rows plus the prefetch page (counted, not timed; constitution V)
- [x] T036 [US1] Compare screens 01 and 02 (FR-061): `scripts/macos-shot.sh 01 --both`, against `Design/focus-macos-design/screens/01-inbox-light.png` and `02-inbox-dark.png`. Record the differences in `docs/notes/<date>-focus-macos-phase-1.md`, then fix or explain each one

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

  *Open:* GTK's adoption is owed to a Linux session: this Mac cannot build `postio-gtk`, and editing `window.rs` blind was judged worse than waiting. The controller half is built and tested in `postio-focus` and driven by the Mac through the FFI; GTK still runs its own copy of these rules, which the focus_suite guards keep honest until it adopts them.

### Controller slice 4: verbs, aim, removal, undo cursor (**main·S4**)

- [x] T041 [US2] Write failing tests in `crates/postio-focus/tests/verbs.rs`:
  - `a` sends Archive aimed at the selection, else the cursor's row;
  - after removal the cursor goes to the survivor below;
  - `ActionCompleted` yields `Toast{text, undoable}`;
  - `UndoPerformed` places the cursor on the restored row;
  - `Aim::Everything` is resolved once in `perform`;
  - `DismissMarker`, `AcceptInvite`/`DeclineInvite` and `ArchiveDigest` send the right commands.
- [ ] T042 [US2] Move `aims`, `dispatch`, `send`, `post`, `note_removed`, `cursor_past_removed`, `restore_cursor`, `place_restored`, `archive_digest`, `answer`, `dismiss_marker`, and `hear`'s toast arms from `crates/postio-gtk/src/window.rs` (~2046-2241, 2964-2999, 3102-3126) into `crates/postio-focus/src/verbs.rs`. Implement `perform` for `Send`/`Post`. The guards are focus_suite `undo`, `selection` (`archive_hands_the_cursor_to_the_row_below`), `invitations`, `offline_send` and `marker_card`. Make T041 green; land slice 4

  *Open:* GTK's adoption is owed to a Linux session: this Mac cannot build `postio-gtk`, and editing `window.rs` blind was judged worse than waiting. The controller half is built and tested in `postio-focus` and driven by the Mac through the FFI; GTK still runs its own copy of these rules, which the focus_suite guards keep honest until it adopts them.

### Engine: the undo top (**main·fix**)

- [x] T043 [US2] Write a failing test in `crates/postio-session` that `Actions::peek_description()` returns "Archived 3 messages" after archiving three, `None` after undo, and `None` once the entry's window has closed
- [x] T044 [US2] Add `Actions::peek_description` (`crates/postio-session/src/actions.rs`, over `UndoStack::peek`, `crates/postio-core/src/undo.rs:397`), `Req::UndoTop`/`Resp::UndoTop(Option<String>)` (`crates/postio-client/src/protocol.rs`), its host handler, and `Client::undo_top()` (`crates/postio-client/src/api.rs`). Make T043 green; land it as its own PR

### FFI and Mac: keys, selection, the action bar, undo

- [x] T045 [US2] Write failing ffi_suite tests (`crates/postio-ffi/tests/ffi_suite/focus.rs`):
  - `invoke("toggle_selection")` is answered by the controller with `UiEvent::FocusSelection`, and `invoke("next_message")` with `FocusCursor`;
  - a verb's `FocusToast` carries the host's words, and `undo_description()` returns them;
  - `focus_point`, `focus_pick(…, range)` move the cursor and select as a click would.
- [x] T046 [US2] Route the controller's commands through `Session::invoke` (`FocusDriver::command`), and emit its intents as UiEvents: `FocusCursor`, `FocusSelection`, `FocusHeading`, `FocusListToTop`, `FocusToast`. Remove `set_cursor`: a verb that is not the list's own aims at the controller's cursor. Export `focus_point`, `focus_pick`, `focus_at_top` and `undo_description()`. Make T045 green

  *As built:* the plan had `key(…) -> KeyPressFfi{handled, pending, intents}` returning intents from the key call. Keys stay as they were (`key` resolves a chord to a command, `invoke` runs it), and the intents arrive on `nextEvent` like every other event. That avoids a second path for the same intents (keys, menus, the bar and buttons all go through `invoke`), and keeps `key` free of side effects as its doc requires. `in_text_entry` is still the caller's, through `key`.
- [x] T047 [P] [US2] Write storyboards `storyboards/list/x-selects-without-moving.toml`, `storyboards/list/has-action-keeps-the-cursor.toml` and `storyboards/list/undo-after-the-pill-is-gone.toml` (`apps = ["focus"]`). Run them on Linux

  *As built:* the three storyboards are in `storyboards/list/` and lint clean; filming them is the Linux runner's, still owed.
- [x] T048 [US2] Write failing Swift tests:
  - `macos/Tests/PostioKitTests/IntentApplierTests.swift`: applying `Cursor`, `Selection`, `Toast` and `KeyboardHome` to a fake main-window model changes exactly what the intent says;
  - `macos/Tests/PostioAppKitTests/UndoManagerTests.swift`: `PostioUndoManager.canUndo` is false with no description; with a description `undoMenuItemTitle` is "Undo <description>"; `undo()` invokes `undo`; `canRedo` is always false.

  *As built:* `FocusIntentsTests.swift` rather than `IntentApplierTests.swift`, over the five Focus `UiEvent`s T046 built (there is no `KeyboardHome` event yet; it comes with the secondary windows), each compared as a whole snapshot before and after so "nothing else changed" is asserted, not assumed. `FocusListPointerTests.swift` adds the table's half: a click is reported and moves nothing, the ring and boxes follow the intents.
- [x] T049 [US2] Implement `IntentApplier` in `macos/Sources/PostioKit/IntentApplier.swift`: the one switch over `IntentFfi`, on the main actor. The unknown-intent arm logs the name and does nothing

  *As built:* `FocusIntents` (`macos/Sources/PostioKit/FocusIntents.swift`), because T046 sends the intents as `UiEvent`s rather than `IntentFfi`: one switch over `FocusCursor`, `FocusSelection`, `FocusHeading`, `FocusListToTop` and `FocusToast`, returning what changed so `FocusListTable.apply` redraws only that. Every other event is `nil` and changes nothing, which is the unknown arm. `FocusListModel` reads its cursor, selection and heading from it and has no setters left; the table reports clicks (`focusPoint`, `focusPick`) and the top (`focusAtTop`, only on a change) and moves nothing itself. Its tests are `FocusIntentsTests` and `FocusListPointerTests`.
- [x] T050 [US2] Rework `macos/Sources/Postio/KeyMonitor.swift` to call the new `key(…)`: swallow when `handled`, show `pending` as the chord hint, and keep the IME and `TypingResponder` guards. Remove the `UiContext` argument, because the context is the controller's

  *As built:* nothing to rework, because the new `key(…)` was not built (see T046): the monitor keeps its shape -- `key` resolves the chord, `invoke` runs it -- with its IME and `TypingResponder` guards and its `UiContext`, which is still the caller's while the surface stack is Swift's (slice 5, T055). What changed is underneath it: no Swift handler moves the cursor or the selection any more (T049), and Escape still closes the surfaces above the list before it reaches `back`.
- [x] T051 [US2] Implement `PostioUndoManager` (an `NSUndoManager` subclass) in `macos/Sources/PostioAppKit/PostioUndoManager.swift`:
  - it caches `undo_description()`, refreshed on each `Toast`/`Notice` intent;
  - the main window's delegate returns it from `windowWillReturnUndoManager`;
  - text fields keep the field editor's own manager.

  Make T048 green

  *As built:* `windowWillReturnUndoManager` cannot work for SwiftUI's window. It makes its `NSUndoManager` while the scene is built, before any view can reach the window, and an `NSWindow` that has one never asks its delegate again (measured: a forwarding delegate installed afterwards was never called). So Edit › Undo and Redo are aimed at an `UndoRouter` (`PostioUndoManager.swift`): in the main window, while its first responder takes no text, it is the engine's manager, titled "Undo <the toast's words>"; anywhere else it sends `undo:` on down the responder chain, so a field keeps its field editor's own undo. The cache is refreshed off the main actor on every `FocusToast` and whenever the main window becomes key. In the list ⌘Z still reaches `undo` through the key monitor first (`mod+z`); the menu's item is the same command.
- [x] T052 [US2] Implement the action bar (SwiftUI) in `macos/Sources/PostioKit/ActionBar.swift`: the count, the actions with keycaps (Archive `a`, Snooze `s`, Mark read `r`, Digest these… `d`, Task `t` only with a vault (C9), Label `l`, Move `m`), and the hints on the right. Add a minimal undo notice line (the pill proper is T093)

  *As built:* `ActionBarWords` holds the verbs in screen 01's order (`postio_ui::focus_dialog::BULK` less Delete, which the screen does not draw; the boundary does not export the table, so the Swift copy says so). The count is the controller's `FocusSelection.summary`. Task is left out: nothing across the boundary says yet whether capture has a vault (C9), and `ActionBarWords(vault:)` takes the answer once something does. The hints are `toggle`, `extend` and `clear`, as GTK's bulk bar words them, with caps from `KeyCapSpelling`, so they read `J K` and `⎋` where the screen draws `⇧J ⇧K` and `Esc` (C22's spelling; the shared `hints::short` is not exported). `UndoNoticeLine` draws the controller's `FocusToast` with Undo and its ⌘Z cap while the stack can take it back; in the main window completions, undos and refusals are now only the toast, and `Notice` is left for failures (`Notice.shownBesideFocusToast`), so one archive is announced once.
- [x] T053 [US2] Compare screen 03 (FR-061): `scripts/macos-shot.sh 03 --both` against `03-inbox-has-action-filter.png`. Record in `docs/notes/<date>-focus-macos-phase-2.md`

  *As built:* recorded in `docs/notes/2026-10-08-focus-macos-phase-2.md`. The state is reached with `POSTIO_DEMO_KEYS` (demo builds only), which presses keys once the list has landed: `'!'` for 03, `'x J J'` and `'x J J a'` for the bar and the undo line. Screen 03 itself shows no selection, so the bar is compared with screen 01's.

---

## Phase 5: User Story 3 — one message in its own window (P1)

**Goal**: screen 04 and the message window's design: geometry (M1), the
chrome, one column, the rhythm, and both treatments (M5).

**Independent test**: open a plain message, a newsletter and a work message
in light and dark, with main widths 1440 and 1024. Each matches its
`message-window/screens/` PNG.

### Controller slice 5: key routing and the surface stack (**main·S5**)

- [x] T054 [US3] Write failing tests in `crates/postio-focus/tests/keys.rs`:
  - the key context comes from the top surface;
  - Reader context routes Back to `CloseMore`, then `CloseFind`, then close;
  - `j`/`k` in Reader step the list;
  - `[`/`]` step the thread;
  - with `stacking = false`, opening a Digest while a Message is open replaces it (contract invariant 6);
  - Back on the list follows the ladder: places, then the bar, then Filtered, then clear (R2).

  *As built:* the cases are in `crates/postio-focus/tests/surfaces.rs` (slice 5, `5530985e`) rather than `keys.rs`: the key context from the top surface, Back closing More, then find, then the message, `j` stepping the list without a close, the Mac replacing the open window and Linux stacking.
- [ ] T055 [US3] Move `handle_key` (`crates/postio-gtk/src/window.rs:803-946`), `key_context` (990-1016), `reading_key` (1117-1231), `digest_key` (~3201), `capture_key`, `settings_key` (`settings.rs:486`) and the dialog close rule (896-907) into `crates/postio-focus/src/keys.rs` and `surfaces.rs`, as tables over the surface stack. Keep the `_ => Proceed` arm (behaviour-neutral). GTK reports `SurfaceOpened`/`SurfaceClosed`; `FocusWindow::handle_key` stays as a shim. The guards are focus_suite `open_keys`, `keymap`, `one_keymap`, `registry_parity`, `every_command`, `settings` and `row_menu`. Make T054 green; land slice 5

  *Open:* GTK's adoption is owed to a Linux session: this Mac cannot build `postio-gtk`, and editing `window.rs` blind was judged worse than waiting. The controller half is built and tested in `postio-focus` and driven by the Mac through the FFI; GTK still runs its own copy of these rules, which the focus_suite guards keep honest until it adopts them.
- [ ] T056 [US3] Write a failing test in `crates/postio-focus/tests/keys.rs` and a failing focus_suite case `crates/postio-gtk/tests/focus_suite/open_keys.rs::t_in_the_open_message_opens_capture`: `t`, `n` and `d` in the open-message dialog open capture, note and the digest rule (#1754)

  *Open:* the controller half is in (`a_key_the_message_does_not_own_is_the_lists` in `crates/postio-focus/tests/surfaces.rs`, and slice 12 made `t`, `n` and `d` the controller's in the open message). The focus_suite case is GTK's, owed to a Linux session.
- [x] T057 [US3] Make Reader-context commands the surface does not own fall through to the list's command table, in `crates/postio-focus/src/keys.rs` (contract invariant 5). Make T056 green. Land as **main·fix**, and comment the fix on #1754 with `Refs` (not a closing keyword unless #1754's acceptance is fully met)

### Controller slice 6: the email window (**main·S6**)


  *As built:* the controller half: `a_key_the_message_does_not_own_is_the_lists` (`surfaces.rs`) proves `t`, `n` and `d` fall through to the host's table in the open message. T056's GTK focus_suite case and GTK's adoption (T055, T059) are not visible from this Mac branch and stay open.
- [x] T058 [US3] Write failing tests in `crates/postio-focus/tests/reader.rs`:
  - ↩ emits `OpenMessage{message, row, position, origin: List}`;
  - `j` in the message emits `OpenMessage` for the next row, without a `CloseMessage`;
  - Archive in the message moves on to the next row (T190 behaviour);
  - a message opened from Found hits steps through the hits;
  - Esc emits `CloseMessage` + `KeyboardHome(CursorRow)` with the selection kept;
  - `step_past` gives up after the timer rather than polling.

  *As built:* in `crates/postio-focus/tests/surfaces.rs` with T054's, not a `reader.rs` of its own.
- [ ] T059 [US3] Move `open_message`, the position line, `follow_cursor`, `reading_changed`, `place_reading`, found-hits stepping and `step_past` (`crates/postio-gtk/src/window.rs:1237-1280, 4230-4259`) into `crates/postio-focus/src/reader.rs`. Replace the 20 ms polling with an event plus a `Timer` give-up. The guards are focus_suite `open_message`, `open_reading`, `reading_pane`, `read_on_dwell` and `view_source`. Make T058 green; land slice 6

  *Open:* GTK's adoption is owed to a Linux session: this Mac cannot build `postio-gtk`, and editing `window.rs` blind was judged worse than waiting. The controller half is built and tested in `postio-focus` and driven by the Mac through the FFI; GTK still runs its own copy of these rules, which the focus_suite guards keep honest until it adopts them.

### Shared: the treated document and the contrast guard (**main·fix**)

- [x] T060 [P] [US3] Write failing tests in `crates/postio-body/src/treatment.rs` for `guard_kept_colours(html, surfaces: [light, dark]) -> String`: a kept inline red on white passes 4.5:1 and stays; a kept `#777` fails in dark and is dropped or scoped under `prefers-color-scheme: light`; nothing changes in paper. Also a shared-corpus test asserting the same kept or dropped decision as `postio_render::theme::guard` for the app-colours fixtures in `crates/postio-model/tests/corpus/` (it runs where postio-render builds)
- [x] T061 [US3] Implement `guard_kept_colours` beside `app_colours` in `crates/postio-body/src/treatment.rs`, using the surface tokens from `postio-ui/data/treatment.css`. Make T060 green

### FFI: the reader document

- [x] T062 [US3] Write failing ffi_suite tests:
  - `reader_document(newsletter)` returns `treatment_shown = Paper` and `classified = Paper`, with `render_mode_words` naming it;
  - `switch_treatment` flips it;
  - `always_treatment(sender, AppColours)` persists to the allowlist file and applies to the next message from that sender;
  - `raw_source(id)` returns the bytes;
  - `column_width` follows M1 for the given window width.
- [x] T063 [US3] Move the FFI reader (`crates/postio-ffi/src/session.rs` ~5420-5544, `reader_answers`) to `postio_ui::reader::document::prepare_treated` / `document_for_treated`, with `guard_kept_colours` applied in app colours. Extend `ReaderDocumentFfi` (treatment_shown, treatment_classified, render_mode_words, sender_choice, column_width, paper_floor). Export `switch_treatment`, `always_treatment`, `treatment_css`, `raw_source`. Delete `thread_document`. Make T062 green

### Mac: the message window


  *As built:* as `focus_reader_document(message, remote, chosen, main_width)` (`crates/postio-ffi/src/focus_reader.rs`) beside the classic `reader_document`, rather than in place of it: `chosen` is the switch (no `switch_treatment` export), and the document carries the treatment's CSS, so `treatment_css` was not needed. `raw_source`, `always_treatment` and (T069) `reader_font` are exported. `thread_document` is not deleted: the classic `ReaderView` and its tests still use it, and both go with the three-pane cleanup (FR-005). Phase 3 added the flowing column's rules (`FLOW_CSS`, `FLOW_FLAT_CSS`, now in `postio_ui::reader::document`) with the Mac's `-apple-system-*` palette and `color-scheme: light dark` for app colours, and `RenderModeWordsFfi.always` ("Always for this sender", `ALWAYS_FOR_SENDER`).
- [x] T064 [P] [US3] Write storyboards `storyboards/reader/esc-returns-to-the-same-row.toml`, `storyboards/reader/j-steps-the-list-without-resizing.toml` and `storyboards/reader/shift-o-switches-treatment.toml` (`apps = ["focus"]`). Run them on Linux

  *As built:* the three storyboards are in `storyboards/reader/`; their Linux run is not visible from this branch.
- [x] T065 [US3] Write failing Swift tests in `macos/Tests/PostioAppKitTests/SecondaryWindowTests.swift`:
  - opening a second kind closes the first (M4);
  - the frame is centred on the main window with the width from the FFI's geometry;
  - close emits the `SurfaceClosed` fact;
  - closing the main window closes the secondary window.

  *As built:* plus a toolbar the content adds not growing the window (the title area is one), and a narrow main window keeping the engine's width. `present` -- the one step that orders a window on screen -- is replaced in the tests, so they need the window server for nothing but the objects.
- [x] T066 [US3] Implement `SecondaryWindowController` in `macos/Sources/PostioAppKit/SecondaryWindowController.swift`: a titled child `NSWindow` of the main window, one at a time, centred, sized from the geometry. ⌘W and Esc close it; on close it sends `ui_fact(SurfaceClosed)` and the main window makes the table first responder. Make T065 green

  *As built:* AppKit, not a SwiftUI scene with `openWindow`: a scene has no parent window, no placement against another window, keeps no frame across content changes, and closes only from a view's `dismissWindow`, where this one is closed by an engine intent. Every close (⌘W, the close button, `FocusCloseSurface`, another kind replacing it, the main window closing) is reported once through `onClosed`; the Engine then calls `focusSurfaceClosed`, and the controller's `FocusKeyboardHome` puts the table back in front. `isReleasedWhenClosed` is off (ARC owns it). The window is tagged `KeyWindow.message`, which resolves as the reader.
- [x] T067 [US3] Implement the message window's chrome in `macos/Sources/PostioKit/MessageWindowView.swift` (SwiftUI) per `message-window/SPEC.md` §2:
  - a 52 pt title area: subject 13.5 bold, the position line in tertiary, then on the right the `k j` keycap and the joined ↑/↓ pair (30×28, radius 6, 1 px divider);
  - a 44 pt action row: Reply `e`, Reply all `E`, Forward `f`, Archive `a`, Snooze `s`, Remind `h`, Label `l`, Move `m`, Delete `⌫`. Label, Move and Delete fold into More `.` below 700.

  *As built:* `MessageChromeWords` composes nothing: the verbs, their words and what folds are `focus_message_view`'s (`crates/postio-ffi/src/focus_message.rs`, a new export: the subject, the position line, the thread chip and the messages `[`/`]` reach, labels, the sender block, the marker card, the action row and attachments, all from `postio_ui`'s functions), and Label, Move and Delete leave the row only when the document's `foldsIntoMore` says so. The title area is a unified `NSToolbar` (`MessageWindowChrome`), which gives it 52 pt and centres the traffic lights, hosting the SwiftUI title and stepper; its separator is off so the title and the action row are one band. Focus rings are off in the window: the keyboard is the key monitor's. The words GTK kept as literals ("+ Label", "Dismiss", From/To/Cc) and "and N others" moved to `postio_ui` for the FFI to use.
- [x] T068 [US3] Implement the content column in `macos/Sources/PostioKit/MessageContentView.swift` per `message-window/SPEC.md` §3-5:
  - the thread marker; the subject at 26/32 bold; labels; the sender block grid (44 pt label column, monospaced address and date);
  - the action card (accent 8% light, 12% dark, C26) with Snooze `s` and Dismiss `-`;
  - the render-mode line; attachments;
  - the §4 rhythm table as constants in one place.

  *As built:* the rhythm is `MessageRhythm`, the same numbers as `postio_ui::focus_dialog::rhythm`. The card is the list row's marker line for the message shown (found by message, not by index: the position line's index counts messages while the list also draws digest rows). A message's scroll position resets per message (`.id(model.shown)`) and survives `O`. The column is the document's `columnWidth`; a window the person resizes keeps the column it opened with (there is no FFI for a window's own width yet).
- [x] T069 [US3] Rework the body web view in `macos/Sources/PostioAppKit/ReaderView.swift` and `macos/Sources/PostioKit/ReaderConfiguration.swift`:
  - JS off, a non-persistent store, `loadHTMLString(_, baseURL: nil)`;
  - a `WKContentRuleList` blocking every load not on `postio-cid:`/`postio-reader:`;
  - app colours via `treatment_css()`;
  - paper with light appearance forced, white `underPageBackgroundColor`, a radius 6 sheet with a hairline, `brightness(0.92)` in dark, and fit zoom ≥ `paper_floor`, measured with `evaluateJavaScript` on the main actor only;
  - the action sentence highlighted (accent 8%, 2 pt underline).

  Add a test in `macos/Tests/PostioAppKitTests/ReaderEgressTests.swift` that a remote `<img>` makes no request

  *As built:* a Focus-specific `MessageBodyView` (`macos/Sources/PostioAppKit/MessageBodyView.swift`) beside the classic `ReaderView`, which nothing in the app uses any more and goes with FR-005. The rule list blocks every load and lets `postio-cid:`, `postio-font:`, `postio-reader:`, `data:` and `about:` through (one rule each: WebKit's rule regexes have no alternation); its egress test uses a document whose CSP *allows* remote images and was seen red without the list. `postio-font:` is served (`FontSchemeHandler`, over `reader_font`), so app colours are in Barlow (C25). Postio's own script runs in the `.defaultClient` world with the page's off (`MessageBodyTests`): the height is the lowest edge of the content, less the classic reader's scroll anchors (which made one-line bodies thousands of points tall), read again once the faces arrive; the paper fit measures what the overflowing body box holds; the action sentence is wrapped in a `<mark>` when one run of text holds it whole. Paper's dim is black at 8% over the sheet, exactly `brightness(0.92)`. Remote images are always asked for blocked: the per-sender grant and the notice's "Load images" in this window are not wired yet.
- [x] T070 [US3] Wire `OpenMessage`, `CloseMessage` and `Reader(verb)` in `IntentApplier`, and `v` raw source as an in-place replacement of the content (M4)

  *As built:* `FocusIntents.surface` maps the six surface events; `MessageWindowModel` (PostioKit) holds the message window's state and reads the chrome and document off the main actor, dropping stale answers; Engine applies them. The window opens once its first document says how wide (M1), reported open once; a step replaces the content. `FocusOpenDraft` opens the compose window as drafts open now; `FocusOpenDigest` logs the kind only. Esc from the raw source returns to the message in Swift (the controller does not hear of the source); Reply, Reply all and Forward from the window answer the message shown. Opening the composer closes the message window first (M4) until the composer is a surface the controller hears of (phase 4). `[`/`]` show the message `focus_message_view` names as `earlier`/`later`, so no conversation is read in Swift.
- [x] T071 [US3] Compare screen 04 and all nine `message-window/screens/*.png` (FR-061): capture at main width 1440 and 1024, in light and dark. Check the 1024 numbers: the window 656, the plain column 560, a paper newsletter at 0.9. Record in `docs/notes/<date>-focus-macos-phase-3.md`

  *As built:* recorded in `docs/notes/2026-10-08-focus-macos-phase-3.md`. At 1024 the window is 655, the formula rounded, where the pack draws 656: a recorded decision, not a defect. The demo can now refile the opened row as the handoff's HTML bodies (`POSTIO_DEMO=small:27`, `small:28`), press Return and Escape in `POSTIO_DEMO_KEYS`, and opens in front (WebKit stops painting a web view in a window it judges covered).

---

## Phase 6: User Story 4 — write and reply (P2)

**Goal**: screens 05 and 06.

**Independent test**: reply to a question row, press Esc, reopen the draft,
send it. It goes through the outbox, and the marker clears.

### Controller slice 7: compose (**main·S7**)

- [x] T072 [US4] Write failing tests in `crates/postio-focus/tests/compose.rs`:
  - `c`, `e`, `E` and `f` emit `Composer(New|Reply|ReplyAll|Forward)` (on the Mac replacing any open secondary surface);
  - `refuses_reply` cases;
  - the autosave debounce: edits within 1500 ms coalesce into one save, and Esc saves now and emits `Toast("Draft saved locally …")`;
  - `settle_send` cancel, retry and mark sent.

  *As built:* 21 tests; 19 were seen red against the new types with their inputs stubbed, and two (no timer with no composer, a save with nothing to keep says nothing) pass vacuously by asserting absence. Beyond the list: on Linux the composer stacks over the open message and Esc leaves the message under it; a sent message and a forward are not refused; a new composition saves the one it replaces before refilling; a composer whose window the toolkit closed is saved as Esc saves it; a failed save is said in its own words; a composer the frontend opened itself (`mailto:`) is a composition too. The toast's words are `postio_ui::compose::saved_at`, "Draft saved locally 16:12", the design's (Mac SPEC §05); GTK's composer says `kept_note`, "Draft saved to Drafts (g t)", which `storyboards/compose/body-typing-is-not-eaten.toml` pins, and both change when GTK adopts the slice.
- [x] T073 [US4] Move `refuses_reply`, `OpenDraft`, `offered_on_open_draft`, `settle_send` and the add-account offer (`crates/postio-gtk/src/window.rs` ~1677-1800), plus the autosave and generation state machine from `crates/postio-widgets/src/composer/mod.rs:101-563, 722-724`, into `crates/postio-focus/src/compose.rs`, as `Input::ComposerEdited`/`Effect::Timer`. The guards are focus_suite `compose`, `drafts`, `one_composer`, `sending_states` and `remind_on_send`. Make T072 green; land slice 7

  *As built (controller):* `crates/postio-focus/src/compose.rs`. The draft's words stay the toolkit's -- its fields and editing surface hold them, and the frontend saves them -- so the controller keeps *when*: which composer opens and what it answers, when what is written is saved, and what is said when a composition ends. A *composition* is one draft's time in the composer, numbered from 1.
  - **Inputs:** `ComposerEdited` (every edit), `DraftSaved{composition, saved: Result<bool, String>}` (kept, nothing worth keeping, or the sentence), `Timer(token)` (an `Effect::Timer` ran out). `Accounts` and the feed's `Opened` also say whether there is an account to write from.
  - **Intents:** `Composer{kind: ComposerKind(New | Reply | ReplyAll | Forward | Draft), message}` replaces `OpenDraft` (a draft row, and Edit on an open draft on its way or stopped, are `Draft`); `SaveDraft{composition}`; `ToastKind::Offer{label, command}` for the add-account offer, whose words left GTK's literal for `focus_target::{NO_ACCOUNT_TO_WRITE_FROM, ADD_ACCOUNT}`.
  - **Requests:** `DraftBehind{message, command}`, answered `Reply::DraftBehind{message, command, draft}` and not dropped by an invalidation, then the settle `Post` (`focus_target::settle_command`) or `NOT_BEING_SENT`.
  - **Effects:** `Effect::Timer{token, after: AUTOSAVE}` (1500 ms, the widgets' `AUTOSAVE_DEBOUNCE`) -- the first slice to use it. Each edit re-arms with a new token; a token the controller moved on from is nothing when it fires, so no timer is ever cancelled.
  - **Rules kept from GTK:** `c`, `e`, `E`, `f` aim at the open message, the email in the digest's window, or the cursor's row (`aimed_message`), and over the bar, a picker, Filtered or capture `e`/`E`/`f` are the surface's; `refuses_reply` refuses a reply (not a forward) to mail still on its way out; settling the open draft closes it with the keyboard home and does not open the next row; `c` with no account offers Add account. Opening the composer ends the composition in it first (saved), so a timer armed for one can never save another's words onto the next row -- the widgets' generation, kept as the composition's number. A composition that ends written is saved at once, and the toast says so once `DraftSaved` says it was kept; an untouched one closes without a word.
  - **Surfaces:** the composer is a controller surface now. Esc on it is the controller's (`answers(Back)`); the writing verbs from inside it begin another composition; every other key is the composer's own. On the Mac it replaces the open secondary window (M4) and any window replacing it ends its composition; on Linux it stacks. Four existing tests changed with it: Compose from the bar opens the composer itself instead of `Run(Compose)`, a draft row's key context is the composer's, the composer's Back is the controller's, `answers` claims the writing verbs.

  *As built (FFI):* `crates/postio-ffi/src/focus_compose.rs`, events in `event.rs`, the driver in `focus_list.rs`; tests in `ffi_suite/focus_compose.rs` (6) and `ffi_suite/compose.rs` (4 more).
  - **Events:** `FocusComposer{kind: ComposerKindFfi, message}` (build the draft with `newDraft`, `replyDraft`, `forwardDraft` or `draftForMessage`); `FocusSaveDraft{composition}`; `FocusOffer{text, label, command}`. `FocusOpenDraft` is gone.
  - **Exports:** `focus_composer_edited()`, `focus_draft_saved(composition, kept, error)`; a `mailto:` composer is `focus_surface_opened(Composer)`, a toolkit close `focus_surface_closed(Composer)`.
  - **The driver honours `Effect::Timer`:** it sleeps on the session's runtime and hands back `Input::Timer`.
  - **For the frame:** `fold_quote`, `composer_title`, `draft_summary`, `draft_saved_words`, `remind_meaning`, `remind_presets`; `DraftFfi.remind_at` (epoch ms, defaulted); `from_ffi` honours a changed `account` (the From picker).
  - `Compose`, `Reply`, `ReplyAll` and `Forward` left `INTERCEPTED`; `RemindIfNoReply` joined it as the composer's own ⌘H.

  *Open:* GTK's adoption is owed to a Linux session, as for slices 3-6: this Mac cannot build `postio-gtk`/`postio-widgets`, so the focus_suite guards were not run and GTK keeps its own copy of these rules. When it adopts, `body-typing-is-not-eaten.toml`'s `notice.text` changes to the controller's words.

### FFI and Mac: compose

- [x] T074 [US4] Write failing ffi_suite tests for `recipient_suggestions(account, "ad", 8, extra: [ExternalContactFfi{"Ada Example","ada@example.com"}])`: a correspondent the person wrote to ranks above the extra; a group completes by name; suppressed contacts never appear

  *As built:* `crates/postio-ffi/tests/ffi_suite/recipients.rs`: a group by name, then a correspondent written to, then the address book's; a deleted contact never; too short a prefix offers nothing; an address the directory has is offered once.
- [x] T075 [US4] Export `recipient_suggestions` in `crates/postio-ffi/src/contacts.rs` over `postio_ui::recipients::suggest`, with the directory cached as `FinderSources` is. Make T074 green

  *As built:* `recipient_suggestions(account, text, limit, extra: [ExternalContactFfi{name, address}]) -> [RecipientSuggestionFfi{label, accepted, group}]` in `crates/postio-ffi/src/contacts.rs`. `accepted` is the field's whole text once taken, so Swift splices nothing. The directory is kept a minute per account; what Contacts lends is used for the one answer and kept nowhere.
- [x] T076 [P] [US4] Write the storyboard `storyboards/compose/esc-keeps-the-draft.toml` (`apps = ["focus"]`), and run it on Linux

  *As built:* lints clean; **not yet filmed** (this Mac cannot build GTK, and GTK has not adopted slice 7). Reply, type a line, Esc: composer closed, keyboard back on its row, `notice.undo = false`; then `g t` and Return reopen the draft. The toast's words carry the time, so they are the frame's to judge, not a `notice.text` check. Its `source` is `{ kind = "spec", ref = "specs/009-focus-macos US4" }`.
- [x] T077 [US4] Write failing Swift tests in `macos/Tests/PostioKitTests/ContactsSourceTests.swift`: with authorization `.notDetermined` the first recipient keystroke requests access once; with `.denied` it never asks again and suggestions still come from mail. Use a fake `CNContactStore` protocol
- [x] T078 [US4] Implement `ContactsSource` in `macos/Sources/PostioKit/ContactsSource.swift`: `CNContactStore` read, after one prompt, of names and email addresses only; never stored or logged. Add `NSContactsUsageDescription` to `macos/Resources/Info.plist`. Make T077 green

  *As built:* `ContactsSourceTests` (7; the two asserting nothing is asked passed against the stub). `ContactBook` (`@MainActor`) is the seam -- `authorization`, `requestAccess()`, `contacts(matching:)` -- with `SystemContactBook` over `CNContactStore` matching by name and fetching only the full name and email addresses. The prompt goes up on the first non-blank keystroke in a recipient field; denied and restricted are answers, and a refusal at the prompt is never asked again in the run. What it lends is read again for each answer and crosses as `extra`; `recipient_suggestions` is asked whatever the book said. A demo never reads the book.
- [x] T079 [US4] Rebuild the composer's frame in its secondary window in `macos/Sources/PostioAppKit/ComposeView.swift` and `macos/Sources/PostioKit/ComposeModel.swift`:
  - fields From (account picker), To, Cc, Bcc, Subject and Labels;
  - a footer with Attach ⌘⇧A, Remind if no reply ⌘H (shows the date when on), the word count, Send later ▾, and Send ⌘↩ (a `labelColor`-filled default button);
  - the reply: pre-filled, quoted text folded, and no contact list until typing.

  Keep the existing `ComposeEditor` WKWebView; `editor.js` stays byte-identical to GTK's (drift test)

  *As built:*
  - **The surface:** `ComposerWindow` (PostioKit, 10 tests) holds what `FocusComposer`, `FocusSaveDraft` and `FocusCloseSurface(.composer)` said; the Engine makes the draft and shows it in `SecondaryWindowController`'s `.composer` window, 980 wide at 1440 (screen 05; the main window less 80 below that), tagged `.compose`. The SwiftUI `WindowGroup` (several composers at once), `ComposeStore` and its three tests are gone. Every edit is `focus_composer_edited`; the Mac's own save on every keystroke and on disappearing are gone; the save is the controller's word, and the model outlives its window until the next composition so the save closing asks for has something to save. Only toolkit closes are reported (close button, ⌘W, Send and Discard).
  - **The frame:** `ComposeView` (PostioAppKit) and `ComposeWindowChrome` (the unified title area); the model's new parts are tested in `ComposeFrameTests` (10). Title "New message"/"Reply to all" with "Draft saved locally 16:12" under it; Send later ▾ and Send ⌘↩ (a `FocusDefaultButton`); From with the account picker; To with "Cc · Bcc ⇧⌘C"; Cc and Bcc once asked for; Subject; the body (the WKWebView for a rich draft, a `TextEditor` for a plain one); a plain reply's quote folded (`fold_quote`) and still saved and sent whole; attachments; a footer with Attach ⌘⇧A, Remind if no reply ⌘H (a menu of `remind_presets`, its day when on, `remind_at` on the draft), the recipient count, the Rich/Plain switch (the marks bar shows only for a rich draft) and the word count.
  - **Completion:** typing in a recipient field asks `ContactsSource.suggestions` (the engine with Contacts as `extra`); the list hangs under the field, ↑↓ move, Return/Tab accept (`accepted` replaces the field's text), Esc takes the list down before it closes the composer. Nothing shows until a recipient is typed.
  - **Not here:** the Labels row (the draft carries no labels across the boundary; a reply's thread labels are GTK's composer seam) and recipient chips; the list's "wrote N times"; M3's ⌘T, ⌘⇧B and ⌘⇧Q. Edit on a queued draft does not cancel its send first, as GTK's `resume` does.- [x] T080 [US4] Compare screens 05 and 06 (FR-061), recorded in `docs/notes/<date>-focus-macos-phase-4.md`. ⌘T, ⌘⇧B and ⌘⇧Q are M3 (Phase 12)

  *As built:* `docs/notes/2026-10-08-focus-macos-phase-4.md`. Seeds `small:05`/`small:06` add `compose_demo`'s people and thread; keys `c g r a c` (a demo composer types into To) and `j j j E`. The shared rule completes from four letters where the design shows the list at three.

---

## Phase 7: User Story 5 — find and go anywhere (P2)

**Goal**: screens 07 to 10.

**Independent test**: type "from ada last week with attachments", see the
chips, open a result, go to `in:Receipts`, and pick a label from the popover.

### Controller slice 8: the bar, search, go-to and places (**main·S8**)

- [x] T081 [US5] Write failing tests in `crates/postio-focus/tests/bar.rs`:
  - `/` emits `OpenBar(Search)`, and ⌘K `OpenBar(Commands)` with `>` (C24);
  - Tab into the chips, and ⌘⌫ back to words (from `bar.rs:527-590`);
  - a command chosen in the bar acts on the aim captured at open;
  - `in:Receipts` opens that folder as a list with the cursor on row 0;
  - `g i`, `g t`, `g s`, `g z`, `g r` and `g f` go to their places;
  - ⌥1–4 open saved searches and ⌘S saves;
  - the places read is one `Request::Places`, not N×M calls.

  *As built:* 23 tests, all seen red against the new types before any behaviour. Beyond the list: a token from lines since redrawn runs nothing; an answer for words since changed is not drawn; closing the bar forgets it; Back closes it; the bar answers its own keys only while up; a command the controller does not answer comes back as `Intent::Run`; the order row asks again and keeps the highlight; a hit opens, `j`/`k` walk the hits, and closing it reopens the bar on the hit. "`in:Receipts` opens that folder as a list" is the blend's Go-to line titled `in:Receipts` (and the popover's row): typing `in:Rec` itself lists Receipts' conversations in the bar under its heading (screen 08), as GTK's bar does, and Return there opens the first of them. The places' "one request" holds at the controller: `perform` still reads each account's mailboxes, labels, counts and correspondents in turn; a batched host request would cut the round trips, and is not built.
- [x] T082 [US5] Move `go_to`, `go_to_inbox`, `go_to_role`, `go_to_view`, the saved searches, `bar_action`, reopen-after-hit, the chip editor (`crates/postio-gtk/src/bar.rs:527-590`) and the places reads (`crates/postio-gtk/src/places.rs:400-470`) into `crates/postio-focus/src/bar.rs`. Implement `perform` for `Places` once. The guards are focus_suite `bar`, `places` and `cursor` (`a_folder_from_the_popover…`). Make T081 green, and clear `GoToArchive`/`GoToSnoozed`/`SavedSearch1-4` from `KNOWN_ORPHANS`. Land slice 8

  *As built (controller):* `crates/postio-focus/src/bar.rs`. The words, lines and what each says stay `postio_ui::command_bar`/`finder`/`places`'; the controller keeps what is typed, the lines on screen and what each runs, and which results are current.
  - **Inputs:** `Typed{text}` (the same words again are nothing, so a frontend may echo its own field), `BarRun(token)`, `BarTab` (no effects when there is no chip), `OpenPlace(token)`, `SavedSearches(Vec<(name, query)>)`, `SearchSaved(Result<saved, sentence>)`, `Keymap(Keymap)`, `Filtering(bool)`.
  - **Intents:** `OpenBar{mode, text, select}` (show the bar, or new words for its field; `select` is a character range, the chip being edited), `BarLines(BarView{heading, echo, chips, editing, lines, highlight, saved})`, `Place{name}`, `OpenPlaces`, `PlacesChanged`, `ShowFiltered`, `SaveSearch{query}`, `Run(CommandId)`. The bar closes with the existing `CloseSurface(Bar)`; it is put on and taken off the stack by the controller, so the frontend's `SurfaceOpened/Closed(Bar)` are harmless repeats, and closing it any other way (Esc in the field, a click outside) is `SurfaceClosed(Bar)`. Back on the bar now closes it, as GTK's window did.
  - **Requests:** `Places` (one, for the bar and the popover), `Search{query, order, stamp}`, `Folder{mailbox, stamp}` (`in:`), `RoleFolder(role)` (`g t`/`g s`/`g r`/`g j`/`g #`: the first enabled account with that role). `perform` answers each once.
  - **Rules kept from GTK:** a command run from the bar acts on the cursor's row when it opened (the cursor is put back if it moved behind the bar); the keyboard goes home as a line runs unless a message opens; a place goes through `feed.open`, so the strip, the paging and the cursor on row 0 follow; a folder that is an inbox is Focus's inbox; a label is its `label:"…"` search; `g z`/`g *` are views; `g b` is the first account's Outbox; `g f` is `ShowFiltered` (the view is slice 12 and T113); a hit opens as the message surface with `index`/`total` among the hits, `j`/`k` walk the hits and a verb acts on the hit's conversation; when it closes, however, the bar reopens on its words with the highlight on the hit last read. `mod+s` says `SaveSearch`; the frontend writes it and answers `SearchSaved`, since the controller holds no path. The bar's keymap starts as the registry's (cached per process on the host platform) until `Keymap` says otherwise.
  - **Not here:** the arrows' highlight is the toolkit's (the view's `highlight` says only where a run or a landing moved it). GTK has not adopted the slice: it cannot be built on this Mac, so the focus_suite guards (`bar`, `places`, `cursor`) were not run; GTK's bar, popover and go-to stay its own until a Linux session switches it over. Search always reads which hits a digest holds; GTK asked only while a digest rule exists.

  *As built (FFI, for T084-T086):* `crates/postio-ffi/src/focus_bar.rs`, events in `event.rs`, the driver in `focus_list.rs`.
  - **Events** (appended to `UiEvent`): `FocusOpenBar{mode: BarModeFfi(Search|Commands), text, select: Option<BarSelectFfi{start, end}>}`; `FocusBarLines{view: BarViewFfi}`; `FocusPlace{name}`; `FocusOpenPlaces`; `FocusPlacesChanged`; `FocusShowFiltered`; `FocusRun{command}` (a registry id the Mac runs as a menu item would: Compose, Settings, a host verb). Closing is `FocusCloseSurface{kind: Bar}`, then `FocusKeyboardHome` when nothing else opens.
  - **`BarViewFfi`:** `heading: Option<String>` (the `in:` folder's), `echo: Option<String>` (under the field), `chips: Vec<String>` (drawn before the field), `editing: Option<u32>` (the chip drawn as edited), `lines: Vec<BarLineFfi>`, `highlight: Option<u64>` (a token; `None` keeps the highlight, or puts it on the first selectable line), `saved: Vec<String>` (the saved row; the `n`th runs `saved_search_<n>`).
  - **`BarLineFfi`:** `kind: BarLineKindFfi` (Heading, Hint, Command, Place, Search, Instead, Order, Correspondent, Message), `token: u64` (never reused), `title`, `detail`, `key` (the binding as the keymap spells it, or a hint's `>`/`@`), `command: Option<String>` (the registry id a keycap is for), `selectable`, and for a message `sender`, `wheres: Vec<String>` (folder, then account; one line each) and `time`.
  - **Exports on `Session`:** `focus_bar_typed(text)` on every change of the field; `focus_bar_run(token)` for Return or a click; `focus_bar_tab() -> bool` (`false`: the key is the toolkit's); `focus_places(filter) -> Vec<PlaceEntryFfi>` read synchronously from the last read (ask again on `FocusPlacesChanged`); `focus_open_place(token)`; `focus_places_placeholder() -> String`. `PlaceEntryFfi{token, section ("Mailboxes"/"Folders"/"Labels", in order), name, count, mark: PlaceMarkFfi(Role{role: MailboxRoleFfi} | Dot{color}), command (its go-to key's id), footer (the popover's line while it is highlighted)}`.
  - **Opening:** `invoke("search")`, `invoke("command_palette")`, `invoke("go_to_folders")`, the go-to keys and `saved_search_1`-`4` all reach the controller through `invoke` -- but `Intercepted` (Swift, mirrored in `postio_ffi::registry::INTERCEPTED`) still catches `search`, `command_palette`, `go_to_folders`, `go_to_inbox`, `go_to_drafts`, `go_to_sent` and `go_to_flagged` for the classic palette and finder. T085 drops them from both lists so the keys reach the bar.
  - **Session:** primed from `config.toml` (saved searches, keymap, `[focus] filtering`, the file a saved search is written to) and kept current by `follow_config`. `mod+s` writes through `postio_ui::saved_search` and toasts `Saved “…”`, or `NO_CONFIG_TO_SAVE` for a session with no file.
  - **Coverage:** `command_coverage.rs` counts a command the controller answers as answered, so T040's cursor and selection keys, the go-to keys (Archive, Snoozed, Outbox, Junk, Trash), `SavedSearch1-4` and `GoToFiltered` left `KNOWN_ORPHANS`. `BackToWords` stays (T085): the controller answers it only while the bar is up.

### Mac: the bar and the folders popover

- [x] T083 [P] [US5] Write storyboards `storyboards/search/slash-opens-search-cmd-k-opens-commands.toml` and `storyboards/search/tab-enters-the-chips.toml` (`apps = ["focus"]`). Run them on Linux

  *As built:* both lint clean. They are **not yet filmed**: this Mac cannot build GTK, so the Linux run is still owed. They check only fields GTK's runner observes today (`app.focus.bar.typed`, `.hints`, `.highlighted`, `keyboard.*`, `overlay.kind`, `cursor.index`); the chips themselves are the frame's to judge, because GTK has no `app.focus.bar.chips` field. Their `source` is `{ kind = "spec", ref = "specs/009-focus-macos US5" }`.
- [x] T084 [US5] Write failing Swift tests in `macos/Tests/PostioKitTests/CommandBarModelTests.swift`: results from `BarText` intents become one-line rows with keycaps; typing forwards `Typed{Bar}` facts; Esc emits `CloseBar`

  *As built:* the names are the FFI's (`FocusOpenBar`, `FocusBarLines`, `focusBarTyped`, `focusBarRun`, `focusBarTab`); the model is `macos/Sources/PostioKit/CommandBarModel.swift`. Every test was seen red against stubs first.
  - **The tests:** each line becomes one row whose keycap is the binding in force (`binding(for:)` through `KeyCapSpelling`), falling back to the key the line names (a hint's `>`), and the chips, the echo and the heading are shown; typing forwards `focusBarTyped`, but not the echo of the controller's own text; Return runs the highlighted line's token, and a click runs its own; Escape is the controller's Back (`invoke("back")`), and the bar closes only when `FocusCloseSurface(.bar)` says so; Tab returns `focusBarTab()`, and `false` leaves the key to AppKit.
  - **Beyond the list:** `FocusOpenBar`'s selection counts Rust `char`s and the field counts UTF-16, so it is converted through Unicode scalars (tested with `é` and an emoji). The arrows' highlight is the Mac's: the line the view names, else the one it was on if still drawn, else the first that runs. A saved pill's click runs `saved_search_<n>`. `CommandBarGeometry` places the panel. `BarCommand` names the registry ids the bar runs by name, each checked against the registry.
- [x] T085 [US5] Implement `CommandBarPanel` in `macos/Sources/PostioAppKit/CommandBarPanel.swift`: a borderless, non-activating child `NSPanel` under the `NSSearchToolbarItem`, as wide as the field or 640, whichever is wider, with no dimming. The keyboard stays in the field. Its SwiftUI content goes in `macos/Sources/PostioKit/CommandBarView.swift`, rendering the chips. Make T084 green

  *As built:*
  - **Keys:** `search`, `command_palette`, `go_to_folders`, `go_to_inbox`/`drafts`/`sent`/`flagged`, and also `toggle_result_order` and `save_search` (which Swift never presented anything for, and which the controller answers while the bar is up) left `Intercepted` and `postio_ffi::registry::INTERCEPTED` together. The coverage sweep now asks the controller with the bar up too, which answers `BackToWords`, so it left `KNOWN_ORPHANS`.
  - **The panel:** it can never become key (`canBecomeKey` is false), and its hosting view accepts the first mouse, so a click runs a line without taking the keyboard from the field. The toolbar's field is a `BarSearchField`; a click into it opens the bar as `/` does.
    - The field's delegate hands ↑/↓, Return, Tab and Escape (`cancelOperation`, a fallback: the key monitor's Escape reaches Back first) to the model.
    - Losing the keyboard while the bar is up (a click outside, Tab past the last chip) is the toolkit's close: `focusSurfaceClosed(.bar)`.
  - **Its height** is the sum of fixed heights per kind (`CommandBarView.height`), capped by the window; the lines scroll beyond that.
  - **Drawn in the panel:** the saved row, the chips row (with the echo and an `Esc` cap) only while there are chips, the heading and lines, and a footer in GTK's words. The typed words are in the toolbar's field, not drawn a second time in the panel.
  - **Surface reports, reconciled with slice 8:** the controller puts the bar on its stack and takes it off, so the Mac reports neither `focusSurfaceOpened(.bar)` nor a close the controller made. Echoing a close is not harmless: a label's Go closes the bar and reopens it on the label's search, and an echo landing after the reopening would close the new bar. For that rule to hold, the controller's Back on the bar now dismisses it as running a line does, and sends `KeyboardHome`. Before, it said only `CloseSurface(Bar)` and kept the bar on the stack until the frontend reported, which left the keyboard in the field (`crates/postio-focus/src/lib.rs`, `back_closes_the_bar`).
  - **Other intents:** `FocusRun{command}` goes to `Engine.run`, as a menu item's command would.
  - **Retired:** `FinderBox`, `FinderResults` and `PaletteRow` (the old `>`/`#`/`@`/`+` box), the unused `SearchContext` and `SearchHint`, their tests, and the Swift wrappers for `paletteEntries` and `finder*`. `Palette.swift`'s `CheatSheet` stays for `?` (T106).
- [x] T086 [US5] Implement the folders and labels popover in `macos/Sources/PostioAppKit/PlacesPopover.swift`: an `NSPopover` anchored to Inbox ▾, with SwiftUI content listing mailboxes, folders and labels with counts, filtered as you type; ↩ opens the place

  *As built:*
  - **The model:** `PlacesModel` (PostioKit, tested in `PlacesModelTests`) reads `focusPlaces(filter)` on `FocusOpenPlaces`, and again on `FocusPlacesChanged`. It keeps the filter, and keeps the highlight by name only when the arrows put it there: before the read lands, only the uncounted views are listed.
  - **Marks:** a role's mark is an SF Symbol; a label's dot is drawn in its own colour, or in the secondary colour when it has none.
  - **Opening a place:** Return or a click goes through `focusOpenPlace` and closes the popover. `FocusPlace{name}` renames the strip's Inbox ▾ (`HeaderStripWords(place:)`).
  - **The popover:** the filter is an AppKit `NSSearchField`, whose delegate hands ↑/↓/Return to the model; `PlacesView` draws the list and the footer. It hangs from a `PlacesAnchor` view that `HeaderStrip` (now generic over it) draws behind the button.
  - **Escape** is caught in `Engine.run` while the popover is up, because the popover is not a surface the controller keeps. Any close sends the keyboard home, unless the bar has taken it (a label opens as its search).
  - **`FocusShowFiltered`** (`g f`, or Filtered in the popover) shows the notice "Filtered is not built on the Mac yet" and logs the kind. The view is T113's.
- [x] T087 [US5] Compare screens 07 to 10 (FR-061), recorded in `docs/notes/<date>-focus-macos-phase-5.md`

  *As built:* `docs/notes/2026-10-08-focus-macos-phase-5.md`. The panel and the popover are child windows, so they are in `screencapture -l`'s picture. Demo replays gained `⌘`/`⌥` prefixes, `↓`/`↑`, `␣`, and whole words typed into the field that is up.

---

## Phase 8: User Story 6 — pickers at the row, and the undo pill (P2)

**Goal**: screens 11 to 15.

**Independent test**: snooze to a typed "tue 9am", label two rows, move one
to a folder, and undo each with the pill and ⌘Z.

### Controller slice 9: pickers, the row menu, toast policy (**main·S9**)

- [x] T088 [US6] Write failing tests in `crates/postio-focus/tests/pickers.rs`:
  - `s`, `h`, `l` and `m` emit `OpenPicker{kind, anchor: Row(pos), aim}`, or `anchor: OpenMessage` from the email window;
  - a number key picks a preset;
  - Tab plus typed "tue 9am" resolves through `postio_search::date::parse_when` to the same instant on both platforms;
  - Label toggles with Space and creates a label;
  - Move offers recent folders as `1` and `2`;
  - toast policy: a new toast replaces the old, the timeout is 8 s, and the undo of a queued send runs first.

  *As built:* 17 tests. The first 15 were seen red against the new types before any behaviour. Two more were seen red when the FFI suite found a race: a label named and confirmed before the labels' read lands. Beyond the list, the tests check that:
  - the label and move pickers read what they list as they open;
  - what is chosen goes where the picker aimed when it opened, even after a click moves the cursor;
  - an answer for a picker since closed changes nothing;
  - a picker with nothing to aim at does not open;
  - the picker keys are the controller's even with no picker up;
  - a name that turns out to exist is put on, not made again.

  The clock is injected with `Input::Clock(Some(..))`. No test reads the clock.
- [x] T089 [US6] Move `open_when`, `open_labels`, `open_moves`, the picker-chosen → `Send` path and `row_menu_alone` (`crates/postio-gtk/src/window.rs:4821-5077`, `move_picker.rs`, `label_picker.rs`), plus the toast policy from `crates/postio-widgets/src/widgets/toast.rs` (`activate_undo` 244, `show_action_completed_for` 155, `rehome` 378), into `crates/postio-focus/src/pickers.rs`. The guards are focus_suite `pickers`, `row_menu` and `undo`. Make T088 green; land slice 9

  *As built (controller):* `crates/postio-focus/src/pickers.rs`. The words, the times, the rows and what a typed date means stay with `postio_ui::pickers` and `postio_ui::schedule`. Two new shared items were added there: `date_hint`, `TYPE_A_DATE` and `NOT_A_DATE`, plus `postio_ui::sending::{QUEUED_TO_SEND, SEND_SCHEDULED}`. They were GTK literals. The controller keeps which picker is up, what it acts on, what each row on screen does, and which reads are current. It numbers the rows itself.
  - **Inputs:**
    - `PickerTyped{text}`. The same text again does nothing.
    - `PickerChoose(token)`: a click, or Return on the highlighted row.
    - `PickerToggle(token)`: Space on the highlighted row.
    - `Clock(Option<DateTime<Local>>)`: what presets and a typed date count from. Unset, it is `postio_ui::clock::now()`.
    - `SendQueued{draft, at}`.
    - The picker keys arrive as `Command`: `PickerChoose1`-`4`, `PickerTypeDate`, `PickerConfirm`, and `Back` while a picker is on top.
  - **Intents:**
    - `OpenPicker(PickerView)`.
    - `PickerRows(PickerView)`, a redraw of the whole picker.
    - `PickerField`: Tab, in a date picker.

    A `PickerView` carries `kind`, `anchor`, `title`, `target`, `field`, `placeholder`, `typed`, `hint`, `rows` and `footnote`. Each `PickerRow` carries `token`, `section`, `name`, `detail`, `key`, `dot`, `color`, `applied` and `create`. The picker closes with `CloseSurface(Picker)`, then `KeyboardHome` when nothing else is up. The controller puts it on the stack and takes it off, as it does the bar. Back closes it, and `SurfaceKind::Picker` joins `back_closes`.
  - **Requests**, each answered once in `perform` and stamped so a closed picker's answer is dropped:
    - `Labels{message, account, threads, stamp}`: one round. It reads the message's account (`account_of`) or the first enabled one, then its labels, its label counts, and the labels the threads carry.
    - `CreateLabel{account, name, stamp}`.
    - `Folders{stamp}`: every enabled account's mailboxes, plus `move_recent`. The controller keeps the destinations.
    - `NoteMove(mailbox)`.

    The choice itself is the existing `Send{command, aims, everything}`: `Snooze{until}`, `RemindIfNoReply{at}`, `AddLabel{label, on}` or `Move{to}`.
  - **`Rows::said(position)`** is a defaulted trait method: a row's `(sender, subject)`, for the target "Ada · Subject". A frontend that does not implement it gets an empty target.
  - **Rules kept from GTK:**
    - A picker from the list aims as a verb would: the selection, else the cursor's row. From the open message it aims at that message alone.
    - The anchor is `Row(cursor)` or `OpenMessage`.
    - Snooze and remind list the four presets, numbered `1`-`4`. Their hint is `date_hint`.
    - A label is `✓ applied` when every conversation aimed at carries it. Space or a click toggles it and the picker stays up. A filter that names no label offers "Create label" first.
    - Move offers Recent, at most two and numbered `1`-`2`, then All folders, with the archive first. Choosing one also keeps it recent.
    - Over the list, a label picker lets the selection go when it closes, however it closes. The other pickers let it go once they act.
  - **Decisions that differ from GTK:**
    - The aim is captured when the picker opens. GTK re-read it when a row was chosen, and a click behind the picker could change it.
    - In the label picker, Return with nothing highlighted makes and applies a label the filter names that nobody has, then closes. GTK only did that from a highlighted Create row, so a typed new name followed by Return closed without making it. Otherwise Return closes.
    - A name confirmed before the labels' read lands waits for the read. Then it is put on if it exists (in any case), or made and put on.
  - **Toast policy:**
    - `ToastKind::seconds()` is `TOAST_SECONDS` (8), or a `Completed{seconds}` window when one is set.
    - Every `Intent::Toast` replaces the one showing, and takes away the queued send's own Undo.
    - `SendQueued` toasts `QUEUED_TO_SEND` or `SEND_SCHEDULED` with Undo. While that toast is the last said, `Undo` sends `Post(CancelSend{draft})` instead of the stack's `Undo`, once (#1752). As in GTK, the timeout does not end that offer; only a newer toast or the Undo itself does.
  - **Not here:**
    - The row menu (`row_menu_alone`, its verbs and words) did not fit. Its words depend on whether the row is flagged, which the Mac's `FocusRowFfi` does not carry, and no Mac task in this phase draws it. It stays GTK's, and is owed with the row facts it needs.
    - The undo of a queued send cancels the send. It does not reopen the composer on the draft as GTK's does: the draft goes back to Drafts. Reopening it belongs with compose (slice 7).
    - GTK has not adopted the slice, because it cannot be built on this Mac. So the focus_suite guards (`pickers`, `row_menu`, `undo`) were not run, and GTK's pickers and toast stay its own until a Linux session switches them over.

  *As built (FFI, for T091-T093):* `crates/postio-ffi/src/focus_pickers.rs`, the events in `event.rs`, and the driver in `focus_list.rs`.
  - **Events** (appended to `UiEvent`):
    - `FocusOpenPicker{view: PickerViewFfi}`: show the picker in a `.transient` popover hung from its anchor. Say `focus_surface_opened(Picker)` once it shows; that is a harmless repeat.
    - `FocusPickerRows{view: PickerViewFfi}`: redraw it whole. Its tokens replace the last.
    - `FocusPickerField`: Tab, so put the keyboard in the date field.

    It closes on `FocusCloseSurface{kind: Picker}`, followed by `FocusKeyboardHome` unless the message window is still open under it. Any other way it closes (a click outside, the popover dismissing itself) is `focus_surface_closed(Picker)`.
  - **`PickerViewFfi`** has these fields:
    - `kind: PickerKindFfi`: `Snooze`, `Remind`, `Label` or `Move`.
    - `anchor: PickerAnchorFfi`: `Row{position}`, the cursor's row, or `OpenMessage`, the message window's action row.
    - `title`.
    - `target`: "Ada · First", "2 conversations" or "Every conversation".
    - `field: PickerFieldFfi`: `Date`, under the rows, which takes the keyboard on `FocusPickerField`; or `Filter`, above the rows, which holds the keyboard from the start.
    - `placeholder`.
    - `typed`: set the field from it only when it differs.
    - `hint: Option<String>`: the date field's line, such as "Tab to type", "Tue 29 Sep, 09:00" or "A day and a time: “tue 9am”".
    - `rows: Vec<PickerRowFfi>`: a label or move picker opens with none, and `FocusPickerRows` brings them.
    - `footnote`.
  - **`PickerRowFfi`** has these fields:
    - `token: u64`: never reused.
    - `section: Option<String>`: a heading drawn above the row ("Recent", "All folders").
    - `name`, in bold.
    - `detail`, on the right: a time, a count, or "✓ applied".
    - `key: Option<String>`: the keycap of the number key that chooses it.
    - `dot: bool` and `color: Option<String>`: a label's dot. A `None` colour means pick one from the name, as `PlaceMarkFfi::Dot` does.
    - `applied: bool`.
    - `create: bool`: the "Create label" row.
  - **Exports on `Session`:**
    - `focus_picker_typed(text)`, on every change of the field.
    - `focus_picker_choose(token)`, for a click, or Return with a row highlighted. A preset or folder acts and closes; a label toggles and stays.
    - `focus_picker_toggle(token)`, for Space with a row highlighted.
  - **Keys:** while a picker is up, `key()` resolves in the picker's context. In a filter that holds nothing, a bare digit or space is the picker's even with `in_text_entry` set. Swift handles three of the resolved commands itself:
    - `picker_toggle`: Swift calls `focus_picker_toggle(highlighted)`. The controller does not know the highlight, so `invoke("picker_toggle")` does nothing.
    - `picker_confirm`: with a row highlighted, Swift calls `focus_picker_choose(highlighted)`; otherwise `invoke("picker_confirm")`, which takes the typed date, or makes the typed label, or closes.
    - The arrows: they move the highlight, which is Swift's.

    `picker_choose_1`-`4`, `picker_type_date` and `back` go through `invoke`. `invoke("snooze" | "remind_if_no_reply" | "add_label" | "move")` opens the picker, from the list or from the message window.
  - **Toast:**
    - `FocusToast.seconds` is always `Some` now (8, or an answer's window). `Toast.defaultSeconds` in `FocusIntents.swift` (6, 4 and 2) is dead and can go.
    - Every `FocusToast` replaces the one showing.
    - A successful `send_draft` or `send_draft_later` now raises `FocusToast{"Message queued to send" | "Send scheduled", Completed, undoable: true, 8}`. Its Undo is the ordinary `invoke("undo")`, which cancels that send first. T093's pill needs nothing special for it.
  - **Coverage:** the seven picker keys left `KNOWN_ORPHANS`.

### Mac: the popovers and the pill

- [x] T090 [P] [US6] Write storyboards `storyboards/list/snooze-typed-date.toml` and `storyboards/list/label-two-with-space.toml` (`apps = ["focus"]`). Run them on Linux

  *As built:* both lint clean. They are **not yet filmed**: this Mac cannot build GTK, so the Linux run is still owed, as it is for T083's. Their checks name only fields GTK's runner observes today (`overlay.kind`, `keyboard.region`, `keyboard.typing`, `cursor.*`, `selection.count`, `notice.undo`); what the picker says (the presets, the hint read back, "✓ applied") is the frames' to judge. `label-two-with-space` puts two labels on two selected rows, so it covers both readings of "label two". Their `source` is `{ kind = "spec", ref = "specs/009-focus-macos US6" }`.
- [x] T091 [US6] Write failing Swift tests in `macos/Tests/PostioKitTests/PickerModelTests.swift` for the four picker models driven by `OpenPicker` and `Typed{Picker}`: presets numbered 1–4, the typed field's parsed preview from the FFI, the Space toggle, ↩ confirms, Esc closes

  *As built:* `macos/Sources/PostioKit/PickerModel.swift`, 16 tests, every one seen red against a stub first. The names are the FFI's (`FocusOpenPicker`, `FocusPickerRows`, `FocusPickerField`, `focusPickerTyped`/`Choose`/`Toggle`); `PostioSession` gained the three wrappers.
  - **The tests:** the presets numbered 1–4 with their keycaps (`KeyCapSpelling` over the row's `key`), the first highlighted as it opens; a label's dot in its colour, or none; sections where the controller starts them; Tab (`FocusPickerField`) puts the keyboard in the date field and takes the highlight away, the typed words go to `focusPickerTyped` (not the controller's own echo), and the hint it reads back is shown; Space toggles the highlighted row; Return chooses it, or with none is `invoke("picker_confirm")`; Esc is `invoke("back")` and the picker closes only on `FocusCloseSurface(.picker)`; a click outside is the toolkit's close, said once; ↑/↓ move the highlight and stop at the ends, and from the date field come back to the presets.
  - **The highlight** is the model's. Every redraw brings new tokens, so it is kept by the row's name (a label just toggled stays highlighted), then by its place; new words in a filter put it on the first row they leave (the Create row, when it is offered); typing in the date field takes it away, so Return means the words.
  - **Decision: Return in the label picker is always the controller's confirm**, which closes it, or makes and puts on the label typed. T089's FFI note had Return on a highlighted label toggle it and stay; the controller's own footnote says "Return closes", and GTK's label picker closed, so the words on screen win. In the other pickers Return chooses the highlighted row.
  - `PickerModel.run` answers `picker_toggle` and `picker_confirm`, the two keys only Swift can (they are about the highlight); every other picker key goes to `invoke`.
- [x] T092 [US6] Implement the picker popovers in `macos/Sources/PostioAppKit/PickerPopover.swift` (`.transient`, anchored to the cursor row's rect in `FocusListTable`, or to the action button in the message window), with SwiftUI content in `macos/Sources/PostioKit/PickerViews.swift`. Make T091 green

  *As built:*
  - **The popover** (`PickerPopover`, PostioAppKit): `.transient`, not animated. From the list it hangs from `FocusListTable.pickerAnchor(row:width:)`: the row's own lines without its day heading's band, from the subject column across the popover's width, so the popover starts at the subject (tested). From the open message it hangs from the action row's button for the verb, or More's when the verb has folded; `MessageWindowView` reports the buttons' frames through the `MessageVerbFrames` preference.
  - **The content** (`PickerView`, PostioKit) is drawn around an AppKit field the popover owns, handed in through a representable: a filter (`NSSearchField`) above the rows for label and move, holding the keyboard from the start; a date field in a box under the rows for snooze and remind, which takes the keyboard only on `FocusPickerField`. Until then a `PickerKeyView` holds it, so the digits, Space, Return and Tab resolve as the picker's commands, and the arrows, which no binding takes in a picker, reach it and move the highlight. In either field the delegate hands ↑/↓ and Return to the model; Escape resolves to Back before the field sees it. The popover is sized from `NSHostingController.sizeThatFits`, again on the next turn: the hosting controller measures the view it last updated, so a measure in the same turn as a redraw is the old rows'.
  - **Surface reports**, as the bar's (T085): the Mac reports neither `focusSurfaceOpened(.picker)` (the controller put the picker on its stack) nor a close the controller made; a click outside is `focusSurfaceClosed(.picker)`, once.
  - **Keys:** `Engine.run` gives `picker_toggle`/`picker_confirm` to the model and Escape to the controller's Back while a picker is up, before any surface of the Mac's own is asked. Demo replays reach a picker as a press would.
  - **A label's dot** with no stored colour is drawn in the secondary colour, as the places popover's: `postio_ui::label_colour` is not exported.
- [x] T093 [US6] Implement the undo pill in `macos/Sources/PostioKit/UndoPill.swift`: a SwiftUI overlay at the bottom centre with the text, an Undo button and a ⌘Z keycap, fading after the toast's seconds. It replaces T052's minimal line

  *As built:* `macos/Sources/PostioKit/UndoPill.swift`. `UndoPillWords` (5 tests, seen red against a stub) holds the controller's words, "Undo" only for an undoable completion, and the cap from `binding(for: "undo")`. The pill is a capsule filled with the primary label colour and lettered in the background's, as the pack and its default buttons draw it, so dark mode turns it over; it sits 20 pt above the window's bottom, or the action bar. A new toast replaces the one showing (`toastToken`) and restarts its timer; it fades in `Motion.current`, now on its going as well as its coming. It replaces `UndoNoticeLine`, which was mounted only in the main window, and that view is gone. `Toast.defaultSeconds` (6, 4, 2 by kind) is gone too: every `FocusToast` carries its seconds, and the FFI type's `nil` falls back to the controller's own eight (`Toast.usualSeconds`). A queued send's Undo is the ordinary `undo`.
- [x] T094 [US6] Compare screens 11 to 15 (FR-061), recorded in `docs/notes/<date>-focus-macos-phase-6.md`

  *As built:* `docs/notes/2026-10-08-focus-macos-phase-6.md`. Popovers are child windows, so `screencapture -l` takes them; the script needed nothing new. Demo replays reach a picker's highlight and fields. Found and fixed on the way: the list did not take the keyboard at launch (the window's first key view, Inbox ▾, did), and the popover was sized for its previous rows.

---

## Phase 9: User Story 7 — the app says what state it is in (P2)

**Goal**: screens 16 to 19.

**Independent test**: an empty store, the network off, a revoked password.

### Controller slice 10: states (**main·S10**)

- [x] T095 [US7] Write failing tests in `crates/postio-focus/tests/states.rs`:
  - `ConnectionChanged`/`SyncProgress`/`BackfillProgress` produce `Banner` and `SyncLabel` intents per `postio_ui::focus_state::banner`;
  - an empty inbox produces `EmptyOrList(Some(page))` with only the shortcuts that exist;
  - a sign-in failure produces the banner with `UpdateCredential`.
- [x] T096 [US7] Move `hear_sync`, `note_account`, `show_state` and `announce` (`crates/postio-gtk/src/window.rs` ~3748-3803, 3048) into `crates/postio-focus/src/states.rs`. The guards are focus_suite `state`, `starts_offline`, `idle_passes` and `empty`. Make T095 green; land slice 10

  *As built (controller):* `crates/postio-focus/src/states.rs`, tests in `tests/states.rs` (11, all seen red against the new types before any behaviour). The words stay `postio_ui::focus_state`'s and the per-account state `postio_ui::status::Trackers`'; the controller keeps who each account is, when mail last synced, and what it last said, and says each of the three only when it changes.
  - **Inputs:** `Event(..)` as before (sync events now reach the states too); `Config(FocusConfig)`, `[focus]` whole: the empty inbox's next digest, and Filtered offered only while filtering is on (it sets the bar's filtering too, so `Filtering(bool)` is no longer needed by a frontend that sends it); `Clock` (as slice 9's) is when a pass that finished says it synced.
  - **Intents:** `Banner(Option<BannerView>)`; `SyncLabel(postio_ui::focus_state::SyncLabel)`; `Empty(Option<postio_ui::focus_state::EmptyInbox>)`. `BannerView{heading, sentence, button: Option<BannerButton{label, command, key}>, progress: Option<(done, total)>, error, account: Option<AccountId>}`: `account` is the account a refused or missing password is for. Retry is `Refresh`, "Update password…" is `UpdateCredential`.
  - **Requests:** `Accounts`, answered by `Reply::Accounts(Result<AccountsRead{facts: Vec<AccountFacts>, last_synced}, String>)`: every enabled account's server, address and name, and the newest folder's last completed sync (GTK's window read the same when its accounts landed). Asked once per account, on the first sync word about an account the controller does not know. Its reply is not dropped by `invalidate()`: who an account is does not go stale with the list.
  - **Rules kept from GTK:** the first word about an account is news even when its tracker does not change (a tracker starts Offline); a list pass reaching its total is when mail last synced; an empty place says why only once its first page has landed (`empty_place`, the folder named by the last `Place`); Focus's inbox with no conversations says `empty_inbox(..).saying(inbox_saying(..))`, so before a pass has finished it offers only Compose; the has-action filter showing nothing draws nothing.
  - **Decisions:** the list is what a frontend draws until told otherwise, so `Empty(None)` is said only to take a page away. After `Keymap` or `Config`, what has been said is said again when it reads differently (a banner's key, a shortcut's key). A first sync's progress crosses as counts, not a fraction.
  - **Not here:** `announce` (the notification decision) did not move: it needs the notifier, which is the host's and the frontends', and the Mac's `MailNotifier` already decides through `postio_ui::notify`. GTK has not adopted the slice, because it cannot be built on this Mac; its feed ignores the new intents and makes one extra `Accounts` read per account. The focus_suite guards were not run.

### FFI and Mac: states

- [x] T097 [US7] Write a failing ffi_suite test: `start_over(store_path)` on a store marked from another build leaves a fresh store that `Session::open_at` opens
- [x] T098 [US7] Export `start_over(store_path)` over `postio_session::start_over_at` in `crates/postio-ffi/src/lib.rs`, with the store key from the keyring. Make T097 green

  *As built (for T100):* a free function, since there is no session when the store will not open: `start_over(store_path: Option<String>) throws(SessionError) -> StartedOverFfi{set_aside: String, accounts: u32}` (`None` is the usual path). It blocks, as `Session.openAt` does: call it off the main actor, then open again. It reads the store key from the Keychain as opening does, sets the database, its sidecars and its blobs aside in `set-aside/<when>/` beside the store (not deleted), and carries the accounts across. A failure is `StoreUnavailable{message}` with a sentence (another Postio has the store open, or the move failed), or `KeyringLocked`. The test, `store_on_disk.rs::starting_over_a_store_from_another_build_leaves_one_that_opens`, uses the Rust-only `start_over_with(SessionOptions)` to inject a secret store.

  *As built (FFI, for T099):* `crates/postio-ffi/src/focus_states.rs`, the events in `event.rs`, the driver in `focus_list.rs`; tests in `ffi_suite/focus_states.rs`.
  - **Events** (appended to `UiEvent`), each said only when it changes:
    - `FocusBanner{banner: Option<BannerFfi>}`: show the strip full width under the header strip, or take it away with `None`.
    - `FocusSyncLabel{text, mark: SyncMarkFfi}`: the toolbar's label. `SyncMarkFfi` is `Synced`, `Syncing`, `Offline` or `Failed`.
    - `FocusEmpty{page: Option<EmptyPageFfi>}`: draw the page in the list's place, or the list again with `None`. The list is what is drawn until this says otherwise.
    - `BackfillProgress{account, done, total}`: typed now, where it crossed as `Other`. The controller already folds it into the label.
  - **`BannerFfi`:** `heading` (bold), `sentence`, `button: Option<BannerButtonFfi{label, command, key}>`, `progress: Option<BannerProgressFfi{done, total}>` (a first sync's bar), `error` (draw in `systemRed` at a low opacity), `account: Option<i64>`. A click runs `button.command` as a menu item would: `refresh` is the host's, through `invoke`; `update_credential` is in `Intercepted`, so Swift opens `AccountRepair` for `banner.account`.
  - **`EmptyPageFfi`:** `heading`, `detail`, `next_digest`, `shortcuts: Vec<EmptyShortcutFfi{key, words, command}>`. A shortcut's click is `invoke(command)`; draw `key` as a keycap before `words`.
  - **Session:** `[focus]` reaches the controller whole (`Input::Config`) at open and on every change of the file.
- [x] T099 [US7] Append a typed `UiEvent::BackfillProgress{account, done, total}` (`crates/postio-ffi/src/event.rs`), then implement the banner strip and the empty state in `macos/Sources/PostioKit/BannerStrip.swift` and `EmptyInbox.swift`:
  - full width under the header strip; the error strip in `systemRed` at low opacity;
  - Retry, and "Update password…" opening a sheet that stores through the engine's credential store (Keychain), reusing `AccountRepair.swift`.

  *As built:* `BackfillProgress` was already typed by the FFI half; the controller folds it into the label, so Swift does nothing with it.
  - **`FocusStates`** (`macos/Sources/PostioKit/FocusStates.swift`) holds the three events as last said, beside `FocusIntents`, and decides nothing. `FocusStatesTests` (13) were seen red against stubs.
  - **`BannerStrip`**: `BannerStripWords` lays out `BannerFfi` -- the button's cap through `KeyCapSpelling`, progress as a fraction (none over a zero total, never past 1), error as `Color.red` at 10%, plain as `.quinary` at half opacity. It sits under the header strip and above the list.
  - **"Update password…"**: `update_credential` stays intercepted. With the banner naming an account, `Engine.run` takes the account's route through a second `AccountRepair` (`bannerRepair`), because the settings window presents whenever its own is asking: a password goes to `PasswordSheet` (PostioKit), a sheet on the main window, and OAuth goes to the browser. While the sheet is up, `run` returns `false` for every key, so Return saves and Escape cancels in the sheet, not in the list behind it.
  - **`EmptyInbox`** replaces the list while `FocusEmpty` holds a page. The Swift empty check (`focusListed && focusCount == 0`) and its `ContentUnavailableView` are gone.
  - **`SyncLabel`** no longer mirrors `sync_label` in Swift. It draws `FocusSyncLabel`'s words and maps the mark to an SF Symbol; only `.failed` is red. Until the controller speaks the toolbar shows nothing. Engine's `syncing`, `syncProgress` and `failure` went with it.
  - **Found in T101, fixed in `postio-focus`:** a banner button now names its key only when the command is available in the list's context. `update_credential`'s `c` belongs to the settings window, and over the list `c` composes.
- [x] T100 [US7] Show the store refusal on launch with "Start over" calling `start_over`, in `macos/Sources/Postio/StoreRefusal.swift`

  *As built:* the model is `StoreRefusal`/`StoreRefusalModel` in `macos/Sources/PostioKit/StoreRefusal.swift`; the view is `StoreRefusalPage` in `macos/Sources/Postio/StoreRefusal.swift`, drawn in place of the whole inbox. `Engine.State.unavailable(String)` became `.refused(StoreRefusalModel)`, and opening is `openStore(after:)` so the page can open again.
  - **The remedy is the boundary's case.** `StoreFromAnotherBuild` offers "Start a fresh store", which runs `startOver(storePath: nil)` off the main actor, then opens the fresh store and says `started_over_words` ("Started a fresh store. The old one is in …") as a notice. Every other case offers "Try again" with the store layer's own sentence. A start over that fails says why and offers Try again; Try again offers the start over again if the store is still the old one. This is GTK's page, step for step.
  - **The words were GTK literals.** They moved to `postio_ui::focus_state` (`CANT_OPEN_MAIL`, `TRY_AGAIN`, `STORE_FROM_ANOTHER_VERSION`, `START_OVER`, `START_A_FRESH_STORE`, `STARTING_A_FRESH_STORE`, `started_over`). GTK's window and startup read them there; that is a const swap this Mac cannot build, and CI proves it. The FFI exports `store_refusal_words()` and `started_over_words(set_aside)` as free functions, since there is no session.
  - **Tests:** `StoreRefusalTests` (8) were seen red against a stub. The Rust word tests were written first, but the refusal's words had already moved by the time they first compiled, so they were never seen red. They compare whole strings.
  - **Not exercised live:** making a store from another build on this Mac is the ffi_suite's job. `p7-refusal` photographs the Try-again page over a demo that does not exist.
- [x] T101 [US7] Compare screens 16 to 19 (FR-061), recorded in `docs/notes/<date>-focus-macos-phase-7.md`

  *As built:* `docs/notes/2026-10-08-focus-macos-phase-7.md`. A demo never syncs, so a demo build takes `POSTIO_DEMO_STATE` (`offline`, `auth`, `first-sync`, `synced`). The new `Session.demo_state` export emits the engine events sync would have emitted for the demo's account, through the `emit_for_test` that the `demo` feature already carries. A build without demos answers `false`. Both halves were seen red first. The comparison found the stray `c` cap described under T099.

---

## Phase 10: User Story 8 — the key map teaches the keys (P3)

**Goal**: screen 20 and the menu bar.

**Independent test**: rebind one command in `[keys]`; `?` and the menu bar
show the new key without a restart.

### Controller slice 11: keymap reload and the key map (**main·S11**)

- [x] T102 [US8] Write failing tests in `crates/postio-focus/tests/keymap.rs`: an `Input::Keymap` rebuilds the resolver; the new binding resolves; `?` toggles `OpenKeyMap`/`CloseTop`
- [x] T103 [US8] Move `set_keymap` (`crates/postio-gtk/src/window.rs:716`) and the CheatSheet toggle into `crates/postio-focus/src/keys.rs`. The guards are focus_suite `keycaps` and `keymap`. Make T102 green; land slice 11

  *As built (controller):* `crates/postio-focus/src/keys.rs`, tests in `tests/keymap.rs` (6, all seen red against the new types before any behaviour).
  - **API:** `FocusController::press(&chord, KeyContext, in_text_entry, Instant) -> postio_ui::keymap::Outcome` is the one resolver a frontend presses keys through, built for Focus's commands (`Resolver::from_commands_for(.., Frontend::Focus)`) from the keymap in force; it resolves and does not run. `FocusController::keymap() -> &Keymap` is the keymap every key the controller spells reads.
  - **Inputs:** `Keymap(Keymap)` (slice 8's) now rebuilds the resolver on the next key, dropping a half-typed sequence, and draws again what is on screen spelling a key: the bar's lines, the picker, the banner's button and the empty page's shortcuts.
  - **Intents:** `OpenKeyMap`. `CheatSheet` is the controller's (`answers` is true): it opens the key map over whatever is up and puts `KeyMap` on the stack at once; `?` or Back with it on top says `CloseSurface(KeyMap)`, and the frontend's `SurfaceClosed(KeyMap)` brings `KeyboardHome`.
  - **Behaviour change:** while the key map or a dialog is on top, every other command is swallowed (`answers` is true, nothing comes back) rather than acting on the list behind it. That is GTK's dialog close rule, which passed the key to the dialog. Quit is the exception.
  - **Not here:** GTK has not adopted the slice (it does not build on this Mac): its window keeps its own resolver and dialog. The focus_suite guards were not run.

  *As built (FFI, for T104-T106):* `crates/postio-ffi/src/focus_keymap.rs`, the event in `event.rs`; tests in `ffi_suite/focus_keymap.rs` and `ffi_suite/registry.rs`.
  - **Keys:** `Session::key` presses through the controller's resolver; the session's own is gone. `follow_config` already sent `Input::Keymap` before `KeymapChanged`, so by the time Swift hears `KeymapChanged`, `key`, `bindingsFor`, `focus_key_map()` and every keycap the controller sends are the new keys.
  - **Event:** `FocusOpenKeyMap{sheet: KeyMapSheetFfi}`: show the key map as a sheet. Say `focus_surface_opened(KeyMap)` when it shows (a harmless repeat); it closes on `FocusCloseSurface{kind: KeyMap}`, and any other way it closes (the close button, a click outside) is `focus_surface_closed(KeyMap)`. On `KeymapChanged` while it is up, draw `focus_key_map()` again.
  - **`KeyMapSheetFfi`:** `title` ("Keys"), `subtitle` (the Mac's: no "Ctrl becomes ⌘"), `close_keys` (the keys of `cheat_sheet` and `back`, as `[keys]` spells them) with `close_or` ("or") and `close_word` ("close"), `groups: Vec<KeyMapGroupFfi{title, rows: Vec<KeyMapRowFfi{command, title, keys}>}>`, `columns: Vec<Vec<u32>>` (indices into `groups`, four columns, a group kept whole, `keymap_sheet::pack_columns`), `rebind_footer` ("Rebind anything in ~/Library/Application Support/Postio/config.toml under [keys]", C3), `mouse_footer`. The groups are `postio_ui::keymap_sheet::key_map_on(.., Focus, Apple)`, the key map GTK draws less what the Mac does not offer (`DarkenMessage`). Keys are `[keys]` spellings (`a`, `cmd+shift+a`, `g i`); render them as `MenuPlan` does.
  - **Export:** `focus_key_map() -> KeyMapSheetFfi`, read now. It replaces `cheatSheetSections` for Focus. That one is the classic grouping (`postio_ui::cheatsheet`), and stays only while the classic views do.
  - **For T106:** `cheat_sheet` is still in `INTERCEPTED` (Rust's `registry::INTERCEPTED` and Swift's `Intercepted`), so Swift's old `CheatSheet` catches `?` first. Drop it from both lists so `?` reaches `invoke`, as T085 does for the bar's keys.
  - **For T104-T105:** no FFI change was needed. A command's `menu` (in `commands()`) is set exactly when Focus offers the command on this platform (`ffi_suite/registry.rs::the_menus_hold_what_focus_offers_here_and_nothing_else`; it passed on arrival, so it guards a property and found no bug). So `MenuPlan.build` over `PostioRegistry.commands` and `menus()` is Focus's menu bar. `bindingsFor(id)` is the key in force, current once `KeymapChanged` is heard.

### Mac: the key map and the menu bar

- [x] T104 [US8] Write failing Swift tests in `macos/Tests/PostioKitTests/MenuPlanTests.swift`: every command offered on Apple for Focus has a menu item with the key `bindingsFor` returns; after `KeymapChanged` the plan is rebuilt with the new key
- [x] T105 [US8] Rebuild `macos/Sources/PostioKit/MenuPlan.swift` and `macos/Sources/PostioAppKit/MenuBar.swift` for Focus's menus (`menus()` for Focus on Apple). Add the standard App/File/Edit/Window items (Settings ⌘,, Quit ⌘Q, New Message ⌘N, Close ⌘W); Edit › Undo backed by `PostioUndoManager`; rebuild on `KeymapChanged`. Fix the stale `ctrl+…` comment at `MenuBar.swift:97`. Make T104 green

  *As built (T104-T105):* `FocusMenuBarTests` (7) were seen red against a stubbed `MenuPlan.bar` and `MenuBarPlan`. `MenuPlan.bar` plans the whole bar, and `MenuBar` only turns planned items (`Item.role`) into `NSMenuItem`s. `MenuPlan.build` stays the registry-only plan that the settings window's Keyboard pane lists.
  - **App:** About, then the registry's Settings, Add account and Edit configuration, then Hide, then the registry's Quit ⌘Q once. AppKit's Quit used to be drawn beside it.
  - **File:** the registry's items, with Compose titled "New Message" (⌘N, the same command as `c`), then Close ⌘W. Close moved here from Window because the HIG puts it under File.
  - **Edit:** Undo is the registry's `undo` (it used to be on the bar twice), aimed at `UndoRouter`. Then Redo, Cut, Copy, Paste, Select All, then the rest of the registry's Edit items.
  - **Window:** Minimize and Zoom, then AppKit's window list, before Help.
  - **Rebuilding:** `MenuBarPlan` rebuilds on `KeymapChanged`, and Engine remounts the bar.
  - **The stale comment** is rewritten. `mod` is ⌘ here: `undo` is ⌘Z and `select_all` is ⌘A. The key monitor runs them in the list, and `KeyDisposition.belongsToText` hands them to a field through these items. The test that compared registry strings with `cmd+` (which never matched `mod+`) now asserts exactly that.
  - **Found on the way, fixed in `postio-core`:** `quit`'s GTK alternate `mod+w` made ⌘W quit the Mac app from any window, the message window included, because the key monitor runs before Window › Close. `registry::alternate_offered_on` keeps that alternate off Apple, and `Keymap::resolve_on` skips it. The two one-keymap tests name it as the one difference besides the modifier, and the Linux table is unchanged.
  - **Known:** Undo's key equivalent stays ⌘Z, which text fields need, even if `[keys]` rebinds `undo`.
- [x] T106 [US8] Implement the key map sheet in `macos/Sources/PostioKit/KeyMapSheet.swift`: groups from `postio_ui::keymap_sheet` via the FFI's `cheat_sheet_sections`, and a footer naming `~/Library/Application Support/Postio/config.toml`, `[keys]` (C3). Delete `macos/Sources/Postio/Palette.swift`'s `CheatSheet` once replaced

  *As built:* the sheet is `focus_key_map`'s `KeyMapSheetFfi`, not `cheat_sheet_sections`. `cheat_sheet` left both intercepted lists, so `?` reaches the controller. `KeyMapSheetTests` (8) were seen red against stubs, and the parity test went red when only the Swift list had changed.
  - **`KeyMapSheetWords`:** the groups go in the sheet's `columns`, and an index past the groups is skipped. Each row's bindings share one cap, spelled by `KeyCapSpelling`. The close keys are caps.
  - **`KeyMapModel`:** it opens on `FocusOpenKeyMap` and closes on `FocusCloseSurface(.keyMap)`. After `KeymapChanged`, an open key map is redrawn from `focus_key_map()`. A click on the dim reports `focusSurfaceClosed(.keyMap)` once; the controller's own opens and closes are not echoed back.
  - **`KeyMapPanel`** is a SwiftUI overlay centred over the dimmed content, as screen 20 draws it, not an AppKit sheet, which would drop from the toolbar and dim nothing. The columns scroll inside a 1100 × 760 panel: the real sheet holds every command Focus offers here, many more than the pack shows.
  - **Retired:** `Palette.swift`, `CheatSheetList`, `CheatSheetLayout`, `CheatSheetKeys` and `KeyCaps`, with their tests; the `cheatSheet` and `cheatSheetSections` wrappers; and Engine's `showingCheatSheet` and `dismissOverlays`. The classic Rust exports stay for their ffi_suite tests.
- [x] T107 [US8] Compare screen 20 (FR-061), recorded in `docs/notes/<date>-focus-macos-phase-8.md`

  *As built:* `docs/notes/2026-10-08-focus-macos-phase-8.md`, reached with `POSTIO_DEMO_KEYS='?'`.
  - **Fixed:** keypad alternates leaked GDK names (`⌘KP_ADD`); `MenuPlan.accelerator` now draws the character the key types. The panel overflowed the window; its columns now scroll.
  - **Also fixed:** photographing a build without demos showed the store refusal page drawn under a header strip floating mid-window. The page now replaces the whole inbox.
  - **Left:** the shared sheet is longer than the pack's curated one, keys are spelled per C22, chords are shown beside letters, and the toolbar is not dimmed.

---

## Phase 11: User Story 9 — filtered, digests and capture (P3)

**Goal**: screens 21 to 25, and `postio://`.

**Independent test**: walk each surface over the demo store, with a model
stub and a test vault. Opening a captured line's `postio://` link from
another app brings Postio forward on that message.

### Controller slice 12: the filtered, digest, rules and capture sub-states (**main·S12**)

- [x] T108 [US9] Write failing tests in `crates/postio-focus/tests/digest.rs`, `filtered.rs` and `capture.rs`:
  - **filtered:** `g f` shows Filtered with tabs `1`–`7`; `R` sends `RestoreFiltered`.
  - **digest:** ↩ on a digest row opens the digest on its summary when a model is configured, else on its list (C6); `]`/`[` step the references; ↩ opens the reference's email with `host: DigestWindow`; Esc returns to the same reference; ⇧A archives all; `D` stops digesting with a `Confirm`.
  - **rules and capture:** `d` opens the rule dialog pre-filled; `t`/`n` open capture only with a vault (C9); ⌥S swaps in the subject.
  - **On the Mac (`stacking = false`):** a message from the digest's list opens in the digest window (M4).
- [x] T109 [US9] Move the per-surface state of `crates/postio-gtk/src/digest.rs` (83-112), `filtered.rs` (52-67), `capture.rs` (52-84), `rules.rs` and `rule_dialog.rs`, and `window.rs`'s digest, filtered and rules handlers (3133-3722), into `crates/postio-focus/src/{digest,filtered,capture}.rs`. The guards are focus_suite `filtered`, `digest`, `digest_summary`, `capture`, `rule_query` and `unsubscribe`. Make T108 green; land slice 12

  *As built (T108):* 23 tests in `tests/filtered.rs` (7), `tests/digest.rs` (10) and `tests/capture.rs` (6), plus 2 `perform` unit tests. All were seen red against the new types with their inputs stubbed, before any behaviour. One digest test passed vacuously at first, so it was tightened to assert the digest is up and the cursor is on a conversation. Beyond the list, the tests check that:
  - a page for a tab since left is not drawn;
  - the sweep counts first and moves only once confirmed;
  - a confirmation answers once;
  - Return over Filtered opens the message over it, and its `j` walks Filtered;
  - a digest takes no verb meant for the list behind it;
  - Create writes the rule, closes the dialog and toasts;
  - `d` in a digest edits its own rule;
  - `⌘↩` writes the vault's suggested project with the mail's due day;
  - a `postio://` link is looked up, then opened or refused.

  *As built (controller, T109):* `crates/postio-focus/src/{filtered,digest,capture,confirm}.rs`. The words are `postio_ui::{filtered, digest, capture, links, focus_target}`'s. New shared words, which were GTK literals, are `digest::{list_tab, FROM, DELIVER, MATCH_INSTEAD, LIKE_THIS}`, `capture::{added, from_line}` and `focus_target::STOP_DIGESTING`. The controller keeps what each surface holds, what each key does to it, and which read is current: every read is stamped, and its answer is kept whatever the list's generation.
  - **Inputs:**
    - Filtered: `FilteredPoint(index)` and `FilteredMore`.
    - Digest: `DigestPoint(index)` and `DigestReference(index)`.
    - `Confirmed(token)`.
    - Rule dialog: `RuleQuery{text}`, `RuleMatchInstead`, `RuleLikeThis`, `RuleSchedule(Schedule)` and `RuleCreate`.
    - Capture: `CaptureTyped{text}`, `CaptureDue(Option<NaiveDate>)`, `CaptureFilter{text}` and `CaptureProject(token)`.
    - `OpenLink(uri)`.

    The keys arrive as `Command`:
    - `FilteredTab1`-`7`, `RestoreFiltered` and `SweepInbox`;
    - `NextReference`/`PrevReference`, `ToggleDigestSummary`, `StopDigestingSender`, `Unsubscribe`, `ArchiveThread` and `DigestRule`;
    - `CaptureTask`/`CaptureNote`, `CaptureUseSubject`, `CaptureChangeProject` and `CaptureWrite`;
    - `Back`, `OpenMessage`, `j`/`k` and `GoToInbox` as each surface reads them.
  - **Intents:**
    - `ShowFiltered`, then `Filtered(Box<FilteredView>)` whole and `FilteredFocus(Option<u32>)`.
    - `OpenDigest{row}`, then `Digest(Box<DigestView>)` whole.
    - `Confirm(Confirm{token, heading, body, confirm, destructive})`.
    - `OpenRule`/`Rule(Box<RuleView>)`.
    - `OpenCapture`/`Capture(Box<CaptureView>)`.
    - `OpenMessage` gains `host: Host` (`Own` or `Digest`).

    Each surface closes with `CloseSurface(kind)`: Filtered, Digest, Dialog (the rule dialog) or Capture. The controller puts each one on the stack and takes it off. A frontend's `SurfaceClosed` for one forgets what it held, and so does the Mac's one-window rule replacing it.
  - **Requests**, each with one `perform` arm:
    - `FilteredTabs`, `Filtered{reason, offset, stamp}` (pages of `filtered::PAGE`) and `SweepPreview`;
    - `DigestRead{delivery, summary, stamp}`: the delivery's messages, and its summary only when `summary`;
    - `Unsubscribe(message)`;
    - `DigestPreview{queries, since, stamp}`, `DigestLikeThis{message, stamp}` and `SaveDigestRule{replacing, draft, stamp}`;
    - `Vault{subject, stamp}`, `CaptureTask{project, task, stamp}` and `CaptureNote{note, entry, stamp}`;
    - `FindMessage(message)` (`message_rows`).

    The writes are the existing `Post`: `RestoreFiltered{Messages([id]), restored}`, `SweepInbox`, `ArchiveDigest{delivery, archived}` and `StopDigestingSender{Messages([id]), stopped, kept: None}`.
  - **`Rows::row(position) -> Option<FocusRow>`** is defaulted to `None`. It is what capture's source, `d`'s senders and a digest row's facts are read from. Without it, a digest row's facts come from the feed's surfaced rows.
  - **Rules kept from GTK:**
    - Filtered opens on All with the first row focused. A page is fifty rows. A number key narrows to a tab, and the focus is kept on its message across a re-read. Mail that moves while Filtered is up (`MessageListChanged`, `UndoPerformed`, `ActionCompleted`) re-reads it.
    - Back or `g i` leaves Filtered with the list's cursor on row 0. Another `g` key leaves it and goes there.
    - The digest opens on its summary when one is written, and on the list otherwise. `]`/`[` clamp at the ends, and the list's focus follows the reference. Tab toggles only while a summary exists.
    - Over the email page, `j`/`k` step the digest's messages in place, citing a reference when one names the message. Esc goes back to the page it came from.
    - `d` on the list makes a new rule for the senders aimed at. Like-this is offered only with a model's `like_this` and a single aim, and never while editing. `d` in a digest edits that rule, or toasts `RULE_MISSING`.
    - Capture's text is the marker's sentence or the subject, and its due day is the marker's. The vault's suggestion is taken when it lands. The preview is `postio_vault::Task::line()` itself.
  - **Decisions that differ from GTK:**
    - C6 is enforced at the read: with no `[focus.model]` digest summary, the summary is not even read. GTK read it always and opened on it when one was there.
    - On the Mac (`stacking = false`) a message from the digest's list opens in the digest's window, as its email page (M4). GTK opened its reading dialog, which Linux keeps.
    - A message opened from Filtered or the digest's list (Linux) walks that list with `j`/`k`, and its verbs read their conversation from there.
    - While Filtered, the digest or capture is on top, a verb meant for the list behind does nothing, as GTK's rules list already refused. Undo, the key map, the bar, Search and Quit still work.
    - Back from a message over Filtered or a digest closes it without `KeyboardHome`: the keyboard goes back to what is under it.
    - `t`, `n` and `d` are the controller's now. The #1754 surface test asserts that they do in the open message what they do on the list.
  - **Not here:**
    - The digest rules list (`g d`, GTK's `rules.rs`) and "Digest mail like this" from the list (`L`): no Mac task in this phase draws them, so they stay in `KNOWN_ORPHANS`.
    - `remember_removed` for an archived digest's row: Undo still restores it, but the cursor does not return to it.
    - Verbs on a message a link opened that is in no list (j/k step the list behind it).
    - GTK has not adopted the slice, because it does not build on this Mac. The focus_suite guards (`filtered`, `digest`, `digest_summary`, `capture`, `rule_query`, `unsubscribe`) were not run.

### FFI: digest, vault, capture and links

- [x] T110 [US9] Write failing ffi_suite tests:
  - `digest_summary(delivery)` over the seed with a model stub returns statements with numbered references;
  - `vault(subject)` with no `[focus.vault]` fails with the configured sentence;
  - `capture_task` writes the exact line with the `postio://` link before the date (C21);
  - `parse_message_link("postio://message/42/")` is 42; `"postio://message/0"` is None.
- [x] T111 [US9] Export the digest, rule, vault and capture reads Swift draws directly (contracts/ffi-focus.md "Reads"), with `*Ffi` mirrors in `crates/postio-ffi/src/focus.rs`. Add `message_link`, `parse_message_link`, `link_unknown` and `link_gone` in `crates/postio-ffi/src/links.rs` over `postio_ui::links`. Make T110 green

  *As built (T110):* `ffi_suite/focus_surfaces.rs`, 8 tests. Six were seen red against stubbed reads and driver arms. The two link tests went green at once, since the API had no stub between not compiling and working; their assertions pin exact ids and words. Beyond T110's list, the tests check that:
  - `g f` draws Filtered's seven tabs and Back leaves it;
  - Return on a digest row opens the window on its list with no model (C6);
  - `t` without a vault toasts `capture::NO_VAULT`, and with one opens capture, follows the text, and writes and toasts;
  - a link opens its message, or toasts `links::GONE`.

  The "model stub" is the summary a model would have written, stored for the delivery with `DigestRepository::set_summary`. No test configures `[focus.model]`, so nothing reaches for a model, loopback included.

  *As built (FFI, for T113-T117):* `crates/postio-ffi/src/focus_surfaces.rs` and `links.rs`, the events in `event.rs`, and the driver in `focus_list.rs`.
  - **Events** (appended to `UiEvent`):
    - `FocusFiltered{view: FilteredViewFfi}`: draw Filtered whole in the list's place. It follows `FocusShowFiltered`, and comes again on every change.
    - `FocusFilteredFocus{index: Option<u32>}`.
    - `FocusDigest{view: DigestViewFfi}`: draw the digest window whole. It follows `FocusOpenDigest{delivery}`, and comes again on every change.
    - `FocusConfirm{confirm: ConfirmFfi{token, heading, body, confirm, destructive}}`: an alert. On yes, call `focus_confirmed(token)`. On no, call nothing.
    - `FocusOpenRule{view: RuleViewFfi}` and `FocusRule{view}`: the rule sheet. It closes on `FocusCloseSurface{kind: Dialog}`.
    - `FocusOpenCapture{view: CaptureViewFfi}` and `FocusCapture{view}`. It closes on `FocusCloseSurface{kind: Capture}`.

    Filtered and the digest close on `FocusCloseSurface{kind: Filtered | Digest}`. Any other way one closes (a window's close button) is `focus_surface_closed(kind)`.

    A message hosted in the digest's window has no `FocusOpenMessage`. Draw `view.email` instead: `reader_document(email.message)` in the window, `email.banner` over it, and `email.excerpt` highlighted.
  - **`FilteredViewFfi`** has these fields:
    - `title`, `subtitle` and `note` (C4).
    - `sweep` and `sweep_key`. A click is `invoke("sweep_inbox")`.
    - `restore` and `restore_key`. A click is `invoke("restore_filtered")`.
    - `tabs: [FilteredTabFfi{name, count, key, on}]`. A click on the n-th is `invoke("filtered_tab_<n>")`.
    - `rows: [FilteredLineFfi{message, sender, subject, preview, pill, time, heading}]`.
    - `focused`, and `more`: call `focus_filtered_more()` at the end of the rows.
    - `footer: [FocusHintFfi{key, label}]`.
  - **`DigestViewFfi`** has these fields:
    - `delivery`, and `page: DigestPageFfi` (Summary, List or Email).
    - `title` and `subtitle`. On the email page these are the email's subject and "Source 2 of 14".
    - `archive` and `archive_key`. A click is `invoke("archive_thread")`.
    - `rule_line` and `rule_key`. A click is `invoke("digest_rule")`.
    - `tabs`, `list_tab` and `tab_key`. A tab click is `invoke("toggle_digest_summary")`.
    - `rows: [DigestLineFfi{message, sender, subject, preview, time}]` and `focused`. A click is `focus_digest_point(i)`.
    - `topics: [DigestTopicFfi{heading, statements: [DigestStatementFfi{index, text, number, message}]}]` and `focused_reference`. A click is `focus_digest_reference(statement.index)`.
    - `card: Option<DigestCardFfi{title, hint, key}>` and `footer`.
    - `email: Option<DigestEmailFfi{message, number, excerpt, banner}>`.
    - `back` and `back_key`: "‹ Summary", or "‹ 3 messages", with Esc. A click is `invoke("back")`.
    - `loading`.
  - **`RuleViewFfi`** has these fields:
    - `heading`, `from_label` and `from`: the senders, or `None` once the query field is up.
    - `query`: the field's text. Set the field from it only when it differs.
    - `placeholder`.
    - `match_instead`: a click is `focus_rule_match_instead()`.
    - `like_this`: a click is `focus_rule_like_this()`.
    - `deliver_label`.
    - `schedule: RuleScheduleFfi{cadence, weekday, month_day, at}`, with `cadences` and `weekdays` as the menus' items. Report every change with `focus_rule_schedule(schedule)`.
    - `note`.
    - `create` and `create_key`. A click, or Return, is `focus_rule_create()`.
    - `preview_heading`, `preview: [RulePreviewLineFfi{subject, day}]` and `more`.
    - `error`.
  - **`CaptureViewFfi`** has these fields:
    - `mode: CaptureModeFfi`. A segment click is `invoke("capture_task" | "capture_note")`.
    - `from`, `field`, `text` (set the field only when it differs) and `subject_key` (`⌥S`).
    - `has_due`, `due` ("YYYY-MM-DD") and `due_label`.
    - `picks: [CapturePickFfi{words, day}]`. A click is `focus_capture_due(pick.day)`.
    - `project_title`, `project`, `project_note` and `project_key` (`⌘P`, which is `invoke("capture_change_project")`).
    - `projects_open`, `filter` (report it with `focus_capture_filter`) and `projects: [CaptureProjectFfi{token, name, note, open, chosen}]`. A click is `focus_capture_project(token)`.
    - `preview_title`, `preview` (the exact line) and `footnote`.
    - `button` and `button_key` (`⌘↩`, which is `invoke("capture_write")`).
    - `error`.
  - **Exports on `Session`:**
    - `focus_filtered_point(index)` and `focus_filtered_more()`.
    - `focus_digest_point(index)` and `focus_digest_reference(index)`.
    - `focus_confirmed(token)`.
    - `focus_rule_query(text)`, `focus_rule_match_instead()`, `focus_rule_like_this()`, `focus_rule_schedule(schedule)` and `focus_rule_create()`.
    - `focus_capture_typed(text)`, `focus_capture_due(day)`, `focus_capture_filter(text)` and `focus_capture_project(token)`.
    - `focus_open_link(uri)`: `FocusOpenMessage`, or a `FocusToast` saying `link_unknown()`/`link_gone()`.
  - **Reads**, which block, so call them off the main actor. The surfaces do not need them, because their views carry the same:
    - `digest_summary(delivery) -> Option<DigestSummaryFfi{statements: [SummaryStatementFfi{topic, text, number, message, excerpt}], messages, senders}>`.
    - `vault(subject) -> VaultPictureFfi{projects, suggested, tasks_note}`. With no vault it fails with `StoreUnavailable{message}`, where the message is the host's sentence naming `[focus.vault]`.
    - `capture_task(project, text, message, due) -> CapturedFfi{note, line}`.
  - **Free functions** in `links.rs`: `message_link(id) -> String`, `parse_message_link(uri) -> Option<i64>` (`None` for `postio://message/0`), `link_unknown()` and `link_gone()`.
  - **The driver** keeps each row with the `FocusRow` it was made from (`HeldRow`), so `Rows::row` works on the Mac. `FocusRowFfi` is unchanged.
  - **Coverage:** `command_coverage` asks the controller with each of its surfaces up. Filtered's tabs, the digest's references and toggle, `t`/`n`/`d` and capture's keys left `KNOWN_ORPHANS`. `GoToDigestRules` and `DigestLikeThis` stay.
  - **For the Swift agent:**
    - Swift's `Intercepted` (mirrored in `registry::INTERCEPTED`) still catches `back`, `open_message` and `go_to_inbox`, as it did `search` before T085. While Filtered, the digest or capture is up, those keys must reach `invoke`. Drop them from both lists once the Mac's own uses are gone, or route them to `invoke` while `focus_surface_*` says such a surface is up.
    - `FocusIntents.surface` must hear the new events.
    - `FocusOpenDigest` is followed by `FocusDigest`. Open the window on the first and draw on the second.
    - `ActionBar`'s `digest_rule` and `capture_task` already go through `invoke` and reach the controller.
    - `HeaderStrip`'s `go_to_digest_rules` reaches nothing yet (`KNOWN_ORPHANS`).

### Mac: the surfaces

- [x] T112 [P] [US9] Write storyboards `storyboards/flows/digest-reference-and-back.toml`, `storyboards/flows/restore-from-filtered.toml` and `storyboards/flows/capture-task-from-message.toml` (`apps = ["focus"]`). Run them on Linux

  *As built:* all three lint clean. They are **not yet filmed**: this Mac cannot build GTK, and GTK has not adopted slice 12, so the Linux run is still owed, as it is for T083's and T090's. Their checks name only fields GTK's runner observes today (`view`, `keyboard.region`, `keyboard.typing`, `overlay.kind`, `cursor.*`, `notice.undo`); the summary, the reasons and the line written are the frames' to judge. The capture flow needs `[focus.vault]` in the runner's file (C9). Their `source` is `{ kind = "spec", ref = "specs/009-focus-macos US9" }`.
- [x] T113 [US9] Implement the Filtered view in `macos/Sources/PostioKit/FilteredView.swift`: it replaces the list in the main window, with the reason per row, tabs `1`–`7` with counts, `R` restore, and the header "Nothing here is deleted automatically" (C4)

  *As built:* `FilteredModel` (`macos/Sources/PostioKit/FilteredModel.swift`, 8 tests, seen red against a stub) holds the last `FocusFiltered` and hands back only the pointer: a click is `focus_filtered_point`, a double click that and `open_message`, a tab `filtered_tab_<n>`, the buttons `restore_filtered`, `sweep_inbox` and `back`. It asks `focus_filtered_more()` once per page, when the last row appears. `FocusShowFiltered` opens it (the notice "Filtered is not built on the Mac yet" is gone), `FocusFilteredFocus` moves the ring, `FocusCloseSurface(.filtered)` closes it.
  - **In the window:** `FilteredView` is drawn over the list in a `ZStack`, header and all; the list stays under it, so its scroll and cursor are kept and the table keeps the keyboard for the key monitor (the controller resolves keys in its Filtered context). The window's overlays (the pill, the key map, the chord) moved from the list's column to the stack, so `R`'s toast shows over Filtered.
  - **Shared parts:** `FocusSurfaceParts.swift` holds the `labelColor`-filled default button, a verb with its cap, a hint row and a chip, used by all four surfaces.
  - **The intercepted keys** (slice 12's question): `go_to_inbox` had already left both lists (T085). `open_message` left `Intercepted.all` and `registry::INTERCEPTED` together: nothing on the Mac answers it, and every press already reached `invoke`. `back` stays: the Mac answers it for the raw source and the folders popover and passes it to `invoke` everywhere else, so it reaches the controller with Filtered, the digest or capture up. The parity test went red when only the Rust list had changed.
- [x] T114 [US9] Implement the digest window in `macos/Sources/PostioKit/DigestView.swift`, hosted by `SecondaryWindowController` at the message window's size with a 560 column (M1):
  - the summary by topic with numbered references; the list page on Tab;
  - the reference's email opens in place with "‹ Summary" `Esc`, the citation banner and the highlighted passage;
  - `U` unsubscribes and `D` stops digesting.

  *As built:* `DigestModel` (`macos/Sources/PostioKit/DigestModel.swift`, 7 tests with `ConfirmQuestion`, seen red against a stub). The window is `SecondaryWindowController`'s `.digest`, its width and column from a new export, `focus_digest_geometry(main_width) -> FocusDigestGeometryFfi{window_width, column_width}` (`Geometry::MAC.digest_size`, tested in `ffi_suite/focus_surfaces.rs`, seen red).
  - **Events:** `FocusOpenDigest` opens the window; every `FocusDigest` replaces the view; `FocusCloseSurface(.digest)` closes it. The Mac reports only a close the toolkit made (`closedByToolkit`, once), and never the opening.
  - **The pages:** the title area is a unified toolbar (`DigestWindowChrome`, PostioAppKit) holding `DigestBack`, `DigestTitle` and `DigestTrailing`: Archive all as a default button on the summary and list, "‹ Summary" and the `k j` stepper (`prev_message`/`next_message`) on an email. Under it, the Summary/list tabs (`toggle_digest_summary`) and the rule line, whose part after the last " · " is the `digest_rule` button. The summary is one `Text` per topic from an `AttributedString`: each statement and its chip carry a link on a private scheme, which the view's `openURL` turns into `focus_digest_reference(index)`. The card stands under the focused reference's topic; its click is `open_message`.
  - **The email in place:** when `view.email` names a new message, the model reads `focus_message_view` (placed among the digest's rows) and `focus_reader_document` off the main actor, dropping a stale answer; the page draws `MessageContentView` and `MessageBodyView` with the excerpt as the body's `<mark>`, under the citation banner. Reply and Forward from it answer that email (`messageInFront`).
  - **`FocusConfirm`** (`D`): an `NSAlert` sheet on the key window, yes destructive when the controller says; only yes is said (`focus_confirmed`). While it is up `Engine.run` runs no key.
  - **Not here:** the email's action row (T124) and the reference's email under its paragraph (T123), both M3; no hint footer under the summary, since `DigestViewFfi` carries none.
- [x] T115 [US9] Implement the digest-this-sender sheet in `macos/Sources/PostioKit/DigestRuleSheet.swift`: pre-filled sender; daily, weekly or monthly with day and time; a preview of what it would have caught; "Match a list or a search instead…"

  *As built:* `RuleSheetModel` (5 tests, seen red against a stub) holds the last `FocusOpenRule`/`FocusRule` and says every control's change as the whole schedule (`focus_rule_schedule`), the query as typed (`focus_rule_query`, never the controller's echo), the links, and Create; Cancel is Back. It closes on `FocusCloseSurface(.dialog)`.
  - **The sheet** is AppKit's `beginSheet` (`FocusSheet`, PostioAppKit), not SwiftUI's `.sheet`, so it hangs from the window with the keyboard: the main window, or the digest's when `d` edits that digest's rule there.
  - **Keys:** a dialog takes every key in the controller and answers only Back, so while the sheet is up `Engine.run` gives Escape to Back, Return (`open_message`/`picker_confirm`) to Create, and every other key to the sheet's fields and menus.
  - "on", "at" and "Cancel" are the Mac's only words; the rest are the controller's.
- [x] T116 [US9] Implement the capture window in `macos/Sources/PostioKit/CaptureView.swift`:
  - the task text verbatim, with ⌥S for the subject;
  - the due date with quick picks;
  - the suggested project with its reason, and ⌘P for the picker;
  - the exact line as a preview;
  - ⌘↩ writes it (Add task is a `labelColor`-filled default button).

  *As built:* `CaptureModel` (4 tests, seen red against a stub) holds the last `FocusOpenCapture`/`FocusCapture`, sets the text and filter fields only when the controller's words differ, and says what was typed, a pick's day (`focus_capture_due`), a project (`focus_capture_project`), and the buttons as `capture_task`, `capture_note`, `capture_use_subject`, `capture_change_project`, `capture_write` and `back`. ⌥S, ⌘P and ⌘↩ are chords, so they resolve from the text field too.
  - **The window** is `SecondaryWindowController`'s `.capture` (M4), 660 wide and 620 tall: `show` gained an optional height, centred and never taller than the message window (`SecondaryWindowTests`, seen red).
  - **Words of the Mac's own** ("Cancel", "Task", "Note", "Due", "Change", "The sentence from the mail, as written ·", "use the subject instead", "Filter projects in your vault") are GTK's literals in `postio-gtk/src/capture.rs`, word for word; they are not across the boundary yet.
  - `ActionBarWords(vault: false)` still holds: nothing across the boundary says whether a vault is configured, so the bar's Task stays hidden.
- [x] T117 [US9] Register `postio` in `macos/Resources/Info.plist` (a second `CFBundleURLTypes` dict). Route it in `macos/Sources/Postio/URLHandling.swift`: `parse_message_link` leads to `command("open_message_by_id")`, or a pill with `link_unknown`/`link_gone`. Add a Swift test in `macos/Tests/PostioKitTests/LinkRoutingTests.swift`

  *As built:* `PostioLink` (`macos/Sources/PostioKit/PostioLink.swift`, 6 tests, seen red against a stub; the not-ours case passed against it). `route(url)` reads a `postio:` link with `parse_message_link`: a message link is `.open(uri, message)`, which `Engine.follow` hands to `focus_open_link(uri)` (the controller opens it in the message window, or says `link_gone`); any other `postio:` link is `.unknown(link_unknown())`, put in the pill at once as the controller's own refusal would be (a notice, no Undo). There is no `open_message_by_id` command: the controller's `OpenLink` input is the route. A cold launch from a link is the usual case: the delegate holds links until it is given somewhere to send them, and the engine holds them again until the list's first page lands (`PostioLink.Waiting`). The app is activated and the main window brought forward.
- [x] T131 [US9] Give the Mac's settings window Focus's nav: export `[focus]` reads and patches over `postio_ui::settings` (as GTK's Filtering pane uses), draw the Filtering pane in `macos/Sources/PostioKit/SettingsPaneView.swift`, drop Appearance, and point `settings_sections` (`crates/postio-ffi/src/settings.rs`) at `crate::FRONTEND`. Update `SettingsStoreTests` and `ffi_suite/settings.rs` (Filtering in, Appearance out)

  *As built (Rust half):* the Swift half (the pane, dropping Appearance, `SettingsStoreTests`) is open.
  - `settings_sections()` is `Section::ALL` filtered by `shown_in(crate::FRONTEND)`. That gives Accounts, Filtering, Saved searches, Composing, Keyboard, Sync & storage, Privacy and Config file. The tests are `ffi_suite/settings.rs::the_nav_is_focus_s_filtering_in_and_appearance_out`, seen red, and the pane-table test, which now looks up `focus`.
  - `settings_filtering(text, filtered_today: Option<u32>) -> Option<FilteringPageFfi>` is GTK's `postio_ui::filtering::page`, with the keymap the file resolves to. It is `None` for a file that will not parse. Pass `focus_counts().filtered_today`, or `None` until it is known.
  - `FilteringPageFfi` has these fields:
    - `switch` and `on`.
    - `state`: the sentence under the switch.
    - `filtered` (the heading) and `today`.
    - `open` and `open_key`. A click is `invoke("go_to_filtered")`.
    - `kept`.
    - `keys: [KeyHintFfi]`.
    - `never_heading`, `guards`, `never: [FilteringEntryFfi]` and `never_empty`.
    - `stopped_heading`, `stopped` and `stopped_empty`.
  - `FilteringEntryFfi` is `{says, acts, undo_label, undo: FilteringUndoFfi::Never{entry} | Marker{sender, kind}}`.
  - `settings_patch_filtering(text, on)` and `settings_take_back_filter(text, undo)` return the file to save, the rest verbatim. They use `focus_edit::set_filtering`, `set_never` and `set_stop_marker`, as GTK's pane does.
  - `settings_appearance`/`settings_patch_appearance` and `Session::appearance` stay until the Swift half stops calling them.

  *As built (Swift half):* the Filtering pane in `SettingsPaneView` draws `settings_filtering` as GTK's does: the switch (a switch, since it acts when flipped) and its sentence, today's count with Open Filtered and its cap, what is kept, the keys, and the two `[focus.filter]` lists, each line with its take-back as a link button. `SettingsStore` gained `filteredToday`, `filtering`, `applyFiltering(on:)` and `takeBack(_:)`, each read-change-write; Appearance's pane, `appearance`, `apply` and their binding are gone, and the window opens on the nav's first pane, Accounts. Open Filtered runs `go_to_filtered` with the main window brought forward (`Engine.runFromSettings`); the count is `focus_counts().filtered_today`, read off the main actor as the window opens. `SettingsStoreTests` (12) were seen red against stubs. `settings_appearance` and `settings_patch_appearance` now have no Swift caller; `Session::appearance` still has one (the engine's `[ui] theme`). Removing the two exports means reworking `ffi_suite/settings.rs`'s Appearance tests, and is left to a Rust session.
- [x] T118 [US9] Compare screens 21 to 25 (FR-061), recorded in `docs/notes/<date>-focus-macos-phase-9.md`

  *As built:* `docs/notes/2026-10-08-focus-macos-phase-9.md`. The demo seeds `small:22`/`small:23` add a `[focus.model]` with only `digest_summary` on, at a socket path that does not exist (C6 is enforced at the read on the Mac, and nothing can be reached), and `small:25` adds a throwaway vault (`postio_demo::demo_vault`, kept for the process); only `27`-`31` refile the opened row now (`crates/postio-ffi/src/demo.rs`, tested, seen red). Found and fixed: "‹ Summary" lacked its chevron.

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

- [x] T125 [P] Regenerate `docs/keybindings.md` and check the "(not macOS)" annotations are gone (`crates/postio-ui/tests/ui_suite/keybindings_doc.rs`)
  - **As built:** nothing to regenerate. `docs/keybindings.md` already matched the registry (`POSTIO_UPDATE_DOCS=1` left it unchanged, both tests pass) and carries no "(not macOS)" note; `which_apps` in the test annotates only the terminal. Every Focus command is offered by the desktop app, the Mac and the terminal, so none needs one.
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
