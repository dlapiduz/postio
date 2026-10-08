# Research: Postio Focus on macOS

Each entry records a decision, why it was taken, and what was rejected.
File references are to `main` at 28f6bde1.

## R1. Focus's behaviour moves to a new crate, `postio-focus`

**Decision.** A new crate `crates/postio-focus` holds a sans-IO
`FocusController`. Inputs go in: a resolved key, an engine `Event`, a client
reply, a timer, and facts about the UI. Out come `Effect`s: `Show(Intent)`
for the app to draw, `Ask(Ticket, Request)` for the engine, and `Timer`. One
async function, `perform(&Client, Request) -> Reply`, is the only mapping
from a request to the client. GTK drives the controller from the glib main
loop. The FFI `Session` drives it from its tokio runtime and hands intents to
Swift.

**Rationale.**
- `postio-ui` cannot host it. `postio-client` already depends on `postio-ui`
  (`protocol.rs:301,314,403,642`; `api.rs:735`), and the controller needs
  client types: `FocusCounts`, list pages, vault and filtered rows.
- `postio-ffi` cannot host it either: it is a uniffi staticlib, and GTK
  cannot depend on it.
- The terminal already proves the pattern. `postio-tui/src/app.rs:1-7` is an
  `Input`/`Effect` machine that "does no I/O itself" (spec 005 R11).
- Sans-IO makes every rule a unit test that runs in milliseconds on either
  host. That matters because `postio-gtk` cannot be compiled on the Mac.
- `press` is synchronous, so the host can decide whether to swallow a key
  within one frame (SC-004).

**Alternatives rejected.**
- *A module in `postio-ui`*: it would create a dependency cycle.
- *A controller that owns a `Client` and spawns its own tasks*: it would tie
  the crate to one executor, and its rules could no longer be tested
  without I/O.
- *Porting the rules to Swift*: it violates FR-002 and FR-004, and leaves
  three copies (GTK, the terminal, Swift).
- *Exporting `postio-gtk`'s window over the FFI*: GTK types cannot cross the
  boundary.

**Boundary.** Add a `postio-focus` rule to
`scripts/checks/check-crate-boundaries.py`.
- *Banned*: GTK, GDK, libadwaita and webkit (with their `-sys` crates); the
  store engines (`turso*`, `rusqlite`, `libsqlite3-sys`); `io-imap`;
  `postio-host`, `-session`, `-storage`, `-runtime`, `-widgets`, `-gtk`;
  `uniffi`.
- *Not allowed as direct dependencies*: `tokio`, `glib`, `async-std`.

This is a rule other work must obey after this feature, so it gets
ADR 0045 (plan.md, "ADR").

## R2. The controller is extracted from GTK in twelve slices, landed on `main`

**Decision.** Twelve slices, each one PR to `main`:

1. skeleton and geometry by platform;
2. feed;
3. cursor, selection, Has action and strip counts;
4. verbs, aim and the undo cursor;
5. key routing and the surface stack, with 5b, the #1754 fix, as its own PR;
6. email window;
7. compose;
8. bar, search, go-to and places;
9. pickers, row menu and the undo pill;
10. app states;
11. keymap reload and the key map;
12. filtered, digest, rules and capture.

In each PR, GTK adopts the slice without changing behaviour. Its public
`handle_key`/`act`/`observe` stay as shims, because `focus_suite` drives
them. Each slice is verified on the Mac with `cargo test -p postio-focus` and
the boundary script, and on Linux by CI's `postio-gtk` suites.

**Rationale.**
- `window.rs` is edited by other sessions every day. A long-lived branch
  holding a second copy of its rules would diverge, and the merge would be
  where the bugs hide (#901).
- A behaviour-neutral move is safe to land alone. Each slice is ordered so
  the Mac step that needs it is unblocked first: slices 2–4 serve the
  inbox, 5–6 the email window, and so on.

**Alternatives rejected.**
- *Extract everything, then switch GTK*: a big bang that cannot be
  compiled locally.
- *Keep the slices on the Mac branch*: the divergence above.

**Intended behaviour changes, each its own PR and test first.**
- **Back.** The FFI's `Back` (`session.rs:4258`) converges on GTK's ladder
  (`window.rs:1596-1605`): places, then the bar, then Filtered, then clear.
- **Anchor.** The selection anchor becomes a message id, not a row index.
- **`step_past`.** Its 20 ms polling (`window.rs:1240-1279`) becomes
  event-driven, with a timer to give up.
- **#1754.** `t`, `n` and `d` act in the open message: commands the surface
  does not own fall through to the list's table (5b).

## R3. Selection splits into a plain `Selector` and an observable wrapper

**Decision.** `postio_ui::selection` gains `Selector {selection, anchor,
reach}`. Its methods return whether anything changed. `SelectionState`
stays as the `Rc`/observer wrapper the terminal uses. The controller owns a
`Selector`. GTK keeps a `SelectionState` only as a view mirror, written only
by the intent applier. The FFI's own cursor and selection fields and
`HANDLED_HERE` (`session.rs:133, 624-646, 4236`) are deleted.

**Rationale.** `SelectionState` is `Rc<Inner>` with observers
(`selection.rs:61-212`), so it is not `Send`, and the FFI needs `Send`.

## R4. Rows stay with each toolkit; the controller reads them through a trait

**Decision.**
- **GTK** keeps `WindowedModel` (`postio-widgets/src/list_model.rs:157`).
- **The Mac's** `ListWindow<RowFfi>` (`session.rs:608`) becomes
  `ListWindow<FocusRow>`, owned by the FFI `Session`.
- **The controller** reads rows through `trait Rows { len, row, position_of }`
  and emits `DeliverPage`.

**Rationale.** There is one resident copy of the window (constitution V).
Moving ownership of `ListWindow` into the controller can come later without
changing the intents.

## R5. The Mac registers as `Frontend::Focus`; three-pane commands go

**Decision.**
- The FFI's resolver, availability and settings filter (`session.rs:422,
  4365`; `settings.rs:316`) use `Frontend::Focus` on `Platform::Apple`.
- `Frontend::Macos` and `Requirement::ThreePane` are removed from the
  registry, with the 19 `ThreePane` commands that only the classic Mac
  offered (`registry.rs:846` … `2184`): rail, sidebar, pane cycling, folder
  walking, saved-search editing and the parts panel. Their tests and the
  keybindings doc regenerate.
- `command_coverage.rs`'s `offered_on_the_mac` flips: Focus commands become
  owed and three-pane ones disappear. `INTERCEPTED` and its Swift mirror lose
  the three-pane entries.

**Rationale.** FR-001 and FR-005, and there is no backwards compatibility
(CLAUDE.md).

**Alternative rejected.** Keeping `Macos` as an unused variant leaves a
requirement only a deleted app can meet.

## R6. Keys on the Mac

**Decision.**
- The controller owns the Focus resolver (`postio_ui::keymap::Resolver`) for
  both apps. Swift's existing `KeyMonitor` → `session.key(…)` path stays
  (`macos/Sources/Postio/KeyMonitor.swift`). `inTextEntry`, IME
  marked-text and `KeyDisposition` keep single letters out of fields
  (FR-032).
- `[keys]` lives in `~/Library/Application Support/Postio/config.toml`
  (`postio-config/src/paths.rs`, C3).
- The FFI `Session` starts a `ConfigService::watch`, which is FSEvents
  through `notify` on the Mac. On `ConfigReloaded{keys}` it rebuilds the
  resolver and emits a `KeymapChanged` event, so the menu bar and keycaps
  re-read their bindings. On `{focus}` it re-runs `enable_focus`, as GTK's
  `follow_config` does (`gtk/startup.rs:95-150`).
- M6:
  - **⌘W** closes the focused secondary window. Swift intercepts it; on Linux
    the binding stays Quit.
  - **⌘Q, ⌘N and ⌘,** are already `mod+q`, `c`'s alternate and Settings.
  - **⌘F** is search in the main window and find in the message window, by
    context.
  - **Delete** gains the alternate `BackSpace` (`registry.rs:903-913`), so the
    Mac's ⌫ (which `KeyEvent.swift:91` names `backspace`) is Delete. Its
    contexts are checked against the bar's `mod+BackSpace`.
- A Rust test resolves the whole Focus keymap on both platforms
  (`docs/notes/2026-09-05-the-gate-that-runs-cannot-see-the-platform-that-does-not.md`, FR-034).

**Alternative rejected.** `.keyboardShortcut`/menu key equivalents as the
dispatch path (ADR 0019 Q4): they bypass chords, contexts and the
text-field rule.

## R7. Undo through `NSUndoManager`, over the engine's stack

**Decision.**
- **Engine.** `postio_core::undo::UndoStack::peek` exists (`undo.rs:397`),
  but the stack is private to `Actions` (`postio-session/src/actions.rs:253`).
  Add `Actions::peek_description`, `Req::UndoTop`/`Resp::UndoTop(Option<String>)`,
  `Client::undo_top()`, and an FFI `undo_description()`. That is the
  smallest new engine API, reported per the brief.
- **Mac.** The main window returns a `PostioUndoManager`, an `NSUndoManager`
  subclass. Its `canUndo` and `undoActionName` come from the cached top
  description, refreshed on each `ActionCompleted`/`UndoPerformed` event.
  Its `undo()` sends `undo`.
- **Text fields.** They keep their own undo through the field editor, which
  NSWindow gives the first responder. So ⌘Z in a field undoes typing.
- **Redo.** There is none (the engine has none), and `canRedo` is false.

**Alternative rejected.** Mirroring each action with
`registerUndo(withTarget:)`: the mirror drifts when the pill's Undo, an
expiry or a closed window changes the engine's stack. `peek` already skips
entries whose window has closed.

## R8. Engine updates and Focus on the FFI

**Decision.**
- `Session::open_at` calls `Host::enable_focus(FocusSetup::from_config(..))`
  after the host starts and before `start_syncing`, as GTK
  (`startup.rs:116,174`) and the terminal (`run.rs:72`) do.
  `postio-host/src/focus/mod.rs:191`. **This is the blocker.** Without it
  the Mac gets no filing, markers, digests or reminders.
- `UiEvent` is append-only. It gains `SurfacedChanged`, a typed
  `BackfillProgress{account, done, total}`, `KeymapChanged`, and
  `Intents(Vec<IntentFfi>)`, the controller's output for replies and events
  (§R1).
- The controller's synchronous output from `key`/`command` is returned
  directly.
- Counts are requested by the controller after each landing, as GTK does
  (`window.rs:2649`).

## R9. Message bodies: the shared treated document in `WKWebView` (M5)

**Decision.**
- **The document.** The FFI's reader moves from `body_html`/`document_for`
  (`session.rs:~5420`) to `postio_ui::reader::document::body_html_treated` /
  `document_for_treated` / `prepare_treated` (`document.rs:1054-1401`). The
  reader document record gains the treatment shown, the treatment
  classified, and the render-mode line's words
  (`reader::document::render_mode_words`).
- **The per-sender choice** uses `postio_ui::allowlist`'s `[Treatment]`
  section, so it is the same file as on Linux.
- **The contrast guard** for app colours runs in Rust. In app colours every
  background is stripped, so the ground is the app's own surface. A pass in
  `postio-body` (beside `app_colours`) checks each kept inline colour
  against the light and dark surface tokens at 4.5:1. It drops a colour that
  fails, or emits it under `prefers-color-scheme`. Linux keeps the
  renderer's own guard, and the two agree on the same corpus (a shared
  test).
- **Paper.** Paper fit uses CSS `zoom` from the column width over the
  document's measured width, read with `evaluateJavaScript` on the main
  actor (page JS stays off), floored at 0.85. In dark appearance it adds
  `brightness(0.92)`, forces light appearance on the paper view, and sets
  `underPageBackgroundColor` to white.
- **Remote content.** Add a `WKContentRuleList` that blocks every non-`postio-*`
  load, on top of the CSP. The design asks for it; today only the CSP blocks.

**Alternative rejected (maintainer, 2026-10-07).** `postio-render` on the
Mac: identical pixels, but about 2k lines of new AppKit tile view, about 226
crates in the FFI library, and fonts unproven on the Mac. It stays a later
option, and the document is the same either way.

## R10. Swift package layout

**Decision.** Three targets, with `PostioFFI` and `postio_ffiFFI` as today.

- **`PostioKit`** has no AppKit import (#1264, iOS). It holds view models,
  the intent applier's policy, the row model, geometry use, picker and
  compose models, `KeyDisposition`, `KeyboardContext`, `MenuPlan`, the
  keycap model, settings and first-run models, and the WebKit-only reader
  configuration.
- **`PostioAppKit`** (new): `FocusListTable` (`NSTableView`),
  `SecondaryWindowController`, `CommandBarPanel`, picker popovers,
  `PostioUndoManager`, `MenuBar`, `KeyEvent`, `KeyWindowTracker`,
  `TypingResponder`, `ViewTreeFocus`, `ComposeEditor`, `ReaderView`.
- **`Postio`** (executable): the app, its scenes, the composition root
  (`Engine`, reduced), URL handling.

**Deleted with the three-pane app:** `Sidebar*`, `SavedSearchRows`,
`Conversation*`, `ThreadDocumentView`, `ToolbarPlan`, `ReaderActionPlan`,
`SearchScopeRail`, `SearchRefineBar`, `Parts*`, `FolderRow`, `Shell`, and
their tests (inventory in the plan).

**SwiftPM only.** `open macos/Package.swift` opens it in Xcode. There is no
`.xcodeproj` (ADR 0019 Q8). The bundle stays `scripts/macos-bundle.sh`.

**AppKit vs SwiftUI by surface.**

| Surface | Choice | Why |
|---|---|---|
| Inbox list | AppKit `NSTableView`, view-based rows | 10k+ rows at 60 fps with variable row heights and day headers; the existing `MessageTable` proves the pattern |
| Toolbar | AppKit `NSToolbar` with `NSSearchToolbarItem` | the unified toolbar and the field the bar drops from |
| Header strip, action bar, undo pill, banners, empty state | SwiftUI hosted in the main window | static layouts over view models |
| Email, digest, compose, capture windows | AppKit `NSWindow` via `SecondaryWindowController`; SwiftUI content; `WKWebView` bodies | window placement, one-at-a-time and focus return need AppKit; content does not |
| Command bar | borderless child `NSPanel` under the search item; SwiftUI list | drops from the field with no dimming, and keeps keyboard focus in the field |
| Pickers, folders | `NSPopover` + `NSHostingController` | anchored to a row rect or a button |
| Key map, digest rule, update password | sheets | the design draws them over a dimmed list |
| Settings | the existing SwiftUI ⌘, window | ADR 0031 |

## R11. System services

- **Keychain.** Through the engine's credential store (`postio_account::secret`),
  as today. No Swift Keychain code.
- **`postio://`.**
  - FFI exports `message_link(id)`, `parse_message_link(uri) -> Option<i64>`,
    and the `UNKNOWN`/`GONE` sentences from `postio_ui::links`.
  - `Info.plist` gains a second `CFBundleURLTypes` entry.
  - `URLHandling.swift` routes it to an `OpenMessage` intent.
- **Contacts.**
  - FFI exports `recipient_suggestions(account, prefix, limit, extra:
    Vec<ExternalContactFfi>)`, which runs `postio_ui::recipients::suggest`
    over the directory plus the extras, ranked as Linux ranks them. Contacts
    rank below mail the person has written to.
  - Swift reads `CNContactStore` after one permission prompt. `Info.plist`
    gains `NSContactsUsageDescription`.
- **Sandboxing.** Out of scope. The app is not sandboxed today, so no
  security-scoped bookmarks.
- **Starting the store over.** FFI exports `start_over(store_path)` over
  `postio_session::start_over_at`, for the `StoreFromAnotherBuild` refusal.

## R12. Verifying screens on the Mac

**Decision.**
- **Fixture.** `scripts/macos-shot.sh`, which is new, launches the bundle
  with `POSTIO_STORE`/`POSTIO_CONFIG` over a scratch copy of the seeded demo
  store (`postio_storage::seed`, fictional, reserved domains).
- **Capture.** The main window is set to 1440×900, and the appearance is
  driven with `-AppleInterfaceStyle Dark` for that process. It captures with
  `screencapture -l<windowid>`.
- **Comparison.** `docs/notes/` gets a dated note per step listing each
  screen's differences against its PNG (FR-061).
- **Mac needs.** This needs Screen Recording for the terminal, once
  and approval of the Keychain prompt for each ad-hoc signed build.
- **Storyboards** are written in spec 008's format and filmed on Linux. Mac
  landings that change an interaction carry `interactions-unreviewed`
  (FR-063).
