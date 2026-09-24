---

description: "Task list for the terminal frontend"
---

# Tasks: Postio in the terminal

**Input**: Design documents from `/specs/005-tui-frontend/`

**Prerequisites**: spec.md (clarified), plan.md, research.md, data-model.md,
contracts/, quickstart.md — all written and committed

**Tests**: Test-first is Constitution IV and NON-NEGOTIABLE. Every task below
that changes behaviour names its test *first* and the code second, and the test
is **observed red** before the code is written. A task whose test was never
seen red has not been done. Terminal tests assert on the rendered
`TestBackend` buffer — what a person would see — never on what a widget was
handed.

## Format: `[ID] [P?] [Story] Description`

- **[P]** — touches files nothing else in its phase touches, so it can run beside its siblings
- **[US#]** — the user story it serves; setup, foundational and polish tasks carry none

## Path conventions

Paths are workspace-relative from `~/src/postio-worktrees/tui-frontend`.
Commits end `Refs: specs/005-tui-frontend` and the task id — never
`Refs: #<issue>`. The branch lands once, with every story done (clarified
2026-09-23); priorities order the work, they are not release slices.

## Three rules that hold for every task

1. **Nothing is taken away from GTK or macOS (FR-005).** `app_suite`,
   `postio-gtk`, `postio-ffi` and `macos/` tests pass with no edit beyond an
   import path. A task that needs to weaken one of those tests has found a
   regression, and stops.
2. **No frontend opens the store.** After Phase 2, `postio-app` and
   `postio-tui` reach mail only through `postio-client` (ADR 0041). A diff that
   gives either of them `postio-storage`, `postio-runtime` or `turso` is the
   boundary failing.
3. **Every string from mail reaches the terminal through
   `postio_ui::terminal::SafeText`.** No widget takes a `&str` from a message
   any other way.

---

## Phase 1: Setup

- [X] T001 Create the three crates with empty `lib.rs`/`main.rs` and add them to the workspace `members` and `default-members` (the root `Cargo.toml` says a new member belongs in both, #1500): `crates/postio-client/`, `crates/postio-host/` (with `src/bin/postio-daemon.rs`), `crates/postio-tui/` (bin `postio-tui`). The workspace root `Cargo.toml` is edited in this worktree, never in the main checkout
- [X] T002 Add boundary rules to `scripts/checks/check-crate-boundaries.py` and its docstring: `postio-tui` and `postio-client` ban gtk4\*, libadwaita\*, gdk4\*, gsk4\*, webkit6\*, turso, rusqlite, io-imap; add the missing `postio-ui` entry (gtk4\*, libadwaita\*, webkit6\*, turso, rusqlite) that `crates/postio-ui/src/lib.rs:11` already claims. Test first: a fixture graph where `postio-tui` depends on `turso` fails the check
- [ ] T003 [P] Add each dependency with its first use (`check-unused-deps.py` refuses one that nothing uses yet), not up front — the list: `crates/postio-tui/Cargo.toml` (ratatui 0.30, crossterm 0.29 with `bracketed-paste`+`osc52`, ratatui-textarea 0.9, tui-input 0.15, tui-markdown 0.3, linkify, arboard 3.6 with `wayland-data-control`+`image-data`, terminal-colorsaurus 1.0; dev: insta 1.48) and `pulldown-cmark 0.13` + `htmd` to `crates/postio-body/Cargo.toml`; confirm `cargo deny`/licence checks in `scripts/check.sh` accept every one
- [X] T004 [P] Add a `release-tui` profile to the workspace `Cargo.toml` (`lto = "fat"`, `codegen-units = 1`, `strip = true`, `panic = "abort"`, `opt-level = "s"`) per research R10

---

## Phase 2: Foundational (blocks every user story)

### The three spikes come first — each writes its answer into research.md

- [X] T005 **Spike T0.1** — in `crates/postio-body/tests/body_suite/markdown_corpus.rs`, run `htmd` over the sanitised HTML of every corpus message and assert contracts/markdown.md's `from_html` promises (no tag, no script text, no `javascript:`, no remote URL in an image position; tables survive as GFM tables). Record pass/fail per message in research.md R5. **If it fails, T040 writes the html5ever walk instead of wrapping htmd**
- [X] T006 **Spike T0.2** — in `crates/postio-ui/src/terminal.rs` (the classifier lives in `postio-ui`, which `postio-core` cannot depend on), enumerate `registry::all()` and list every command whose default and alternate bindings are all undeliverable by a legacy (non-kitty-protocol) terminal. Record the list in research.md R4. **T017 adds an alternate for each**
- [X] T007 **Spike T0.3** — measured with the existing `postio-diag` release binary rather than a spike branch (research R1): measure fork+exec of a minimal daemon, then keyring, then store open against the reference mailbox, overlapped with a ratatui startup; record the numbers in research.md R1. If cold start exceeds 500 ms, record which step and stop for a re-think before T020

### Host and client, in-process (GTK unchanged)

- [X] T008 Write `crates/postio-client/src/protocol.rs`: `Frame`, `Hello`/`Welcome`/`Refused`, `Request{id,body:Req}`, `Response`, `Event`, `Notify`, and the `Req`/`Resp` families of contracts/protocol.md, all serde. Test first: every `Req` variant and every existing `postio_core::Event` variant round-trips through the chosen encoding (pick `postcard` or `bincode` here and record it in contracts/protocol.md)
- [X] T009 Add serde derives to `MessageSummary` and the page/row types that cross the protocol (`crates/postio-runtime/src/store/mod.rs` and `crates/postio-model/`); test first in T008's round-trip suite
- [X] T010 Write `crates/postio-client/src/api.rs`: `Client` with async `send`, `send_tracked`, `events()`, and one typed method per `Req`; a `Transport` trait it is generic over. Model the query surface on `crates/postio-ffi/src/session.rs` (`open_scope`, `row_at`, `invoke`, `search`, `palette_entries`, `next_event`). Test first against a fake transport that records requests
- [X] T011 Write `crates/postio-client/src/counting.rs`: per-`Req`-family round-trip counters readable in tests. Test first: two `page` calls count two
- [X] T012 Write `crates/postio-host/src/lib.rs`: `Host::start(paths, key_source)` composing `Wiring` exactly as `crates/postio-app/src/lib.rs::assemble` (:1388–1446) does — `actions::wire`, `refresh::wire`, `EventHub`, `Bridge` — plus `Host::local_transport()` implementing `postio_client::Transport` over channels, one `EventHub::subscribe("client:<kind>:<id>")` per client. Test first: a `Client` over the local transport archives a fixture message and receives the resulting event
- [X] T013 (in `crates/postio-host/src/lib.rs`, per-client `Actions` and event sorting) `ClientState` per data-model.md with a **per-client `UndoStack`** (research R1a); `Command::Undo` pops the sender's stack only; undo events go only to the owner. Test first: client A archives, client B undoes → nothing is restored; A undoes → restored
- [ ] T014 Move the store half of `crates/postio-app/src/reading.rs` (body loading, `BodyFetcher`, thread fill, unsubscribe activation, `PartSource`/`write_part`/`locate_part`) into `crates/postio-host/src/reading.rs` behind the Reading `Req`s. The `app_suite` reading cases (`click_preview.rs` et al.) are the test and must stay green unchanged
- [ ] T015 Move the store half of `crates/postio-app/src/compose.rs` (`DraftWriter`, `install_autosave`, `save_draft`, `delete_draft`, `queue_send[_at]`, `cancel_queued_send`, `recover`, identities, signatures, recipient suggestions, `attach_file`, `install_inline_image`) into `crates/postio-host/src/compose.rs` behind the Compose `Req`s; `DraftBody` carries `markdown: Option<String>`. `app_suite` compose cases stay green unchanged
- [ ] T016 Move the store halves of `crates/postio-app/src/search.rs`, `settings_accounts.rs`, `settings_credential.rs`, `settings_privacy.rs`, `settings_egress.rs`, `sidebar_backfill.rs`, `export.rs` and the persistence/OAuth-loopback half of `onboarding.rs` into `crates/postio-host/src/{search,settings,onboarding}.rs`; move `crates/postio-gtk/src/reader/allowlist.rs` to `crates/postio-host/src/allowlist.rs` with the TOML format of data-model.md. Test first for the allowlist: a sender allowed through one client is allowed for another
- [X] T017 Add a legacy-terminal alternate binding in `crates/postio-core/src/registry.rs` for every command T006 listed; regenerate `docs/keybindings.md` through its test. Test first: T006's test flips to asserting the list is empty. GTK gains alternates only
- [ ] T018 Put `crates/postio-app` on `postio-client` over `Host::local_transport()`: replace every direct use of `wiring.database`, `.blobs` and `store` (≈78 sites across 13 files) with `Client` calls; `postio-app` loses its `postio-storage`/`postio-runtime` dependencies (dev-dependencies may keep `postio-host`). The whole of `app_suite` is the test and passes unchanged; add a boundary rule banning turso from `postio-app`'s normal graph and watch it go green
- [ ] T019 Put `crates/postio-ffi` on `postio-host` in-process with a **real** bus (it currently wires a no-op handler, `session.rs:~890`), keeping its uniffi surface identical. `postio-ffi` tests and the macOS package tests pass unchanged; a new test proves an archive through the FFI now reaches the store

### Socket transport and the daemon

- [X] T020 Write `crates/postio-client/src/socket.rs`: connect to `$XDG_RUNTIME_DIR/postio/daemon.sock`, handshake with exact `BuildId`, spawn `postio-daemon` (beside own exe, then `PATH`) when nothing answers, retry with backoff for ≤ 2 s. Test first: a handshake with a different build id is `Refused(VersionMismatch)` and the error names both versions
- [X] T021 Write `crates/postio-host/src/serve.rs` and `src/bin/postio-daemon.rs`: `flock` on `daemon.lock`, socket dir `0700`/socket `0600`, `SO_PEERCRED` uid check, `Refused(Starting)` until the store is open, the 30 s drain timer (research R1b), SIGTERM, engine stop as `stop_retained` does. Tests first: a second daemon exits cleanly while one holds the lock; a peer with another uid is refused; the daemon exits 30 s (test clock) after its last client and not before
- [ ] T022 Write `crates/postio-host/src/notify.rs`: notifier election (first `Gtk` client, else first `Tui`) and `Notify` frames using `postio_ui::notify::decide` unchanged. Test first: with one GTK and one TUI client, an arrival produces exactly one `Notify`, to GTK; with only TUI, to TUI
- [ ] T023 Switch `crates/postio-app` to the socket transport at runtime (in-process stays for `app_suite`), and deliver `Notify` through the existing `gio::Notification` path. Run `app_suite` unchanged, then a new `app_suite` case that starts the GTK app against a running daemon
- [ ] T024 Add the `drafts.body_markdown TEXT` column to `crates/postio-storage/src/schema.rs` and the drafts repository (`crates/postio-storage/src/repository/drafts.rs`), written from `DraftBody.markdown` and written NULL by the GTK save path. Test first: a GTK save over a draft with Markdown leaves NULL

### Terminal foundations

- [X] T025 [P] Write `crates/postio-ui/src/terminal.rs`: `SafeText::new(&str)` stripping C0 (except `\n`,`\t`), C1, ESC, DEL and bidi overrides/isolates, replacing each with a visible glyph; `SafeText` is the only way to build a span from mail text. Test first with the escape sequences of quickstart §5
- [ ] T026 [P] Write `crates/postio-tui/src/app.rs`: `App` state per data-model.md "Terminal session" and a pure `update(&mut App, Event) -> Effects`; `Effects` are client calls, redraw, quit. Test first: `Resize` changes the layout without any client call
- [ ] T027 [P] Write `crates/postio-tui/src/input.rs`: crossterm `KeyEvent` → `postio_ui::keymap::Chord` feeding `Resolver::from_commands` with `[keys]` overrides (as `postio-ffi/src/session.rs:299` does). Test first: a `[keys]` override of `archive` is honoured
- [ ] T028 [P] Write `crates/postio-tui/src/term.rs`: enter/leave (raw mode, alternate screen, mouse, bracketed paste, keyboard flags), a panic hook that restores, SIGTSTP/SIGCONT handling, and `with_suspended(|| …)` for `$EDITOR`. Test first against a recording backend: leave restores every mode entered, in reverse order
- [ ] T029 [P] Write `crates/postio-tui/src/caps.rs`: `TerminalCaps` detection (`NO_COLOR`, `COLORTERM`, `supports_keyboard_enhancement`, colorsaurus background with timeout), run before raw mode, with a reserved slot for the image-protocol query (research R8). Test first: `NO_COLOR=1` yields `colour: None` whatever `COLORTERM` says
- [ ] T030 [P] Write `crates/postio-tui/src/theme.rs`: the colour-role table of data-model.md resolved from `[tui.colors]`, the true-colour accent from `postio_ui::tokens` (never retyped) for Selection and Focus only, ANSI indices otherwise, attributes under `NO_COLOR`. Test first: under `NO_COLOR` no cell in a rendered list has a colour
- [ ] T031 Add `[tui]` and `[tui.colors]` to `crates/postio-config` with validation and live reload; test first: an unknown role is a reported problem, not a crash
- [ ] T032 Write `crates/postio-tui/src/layout.rs`: the width table of contracts/tui-surface.md, requested vs shown panes (ADR 0024's rule). Test first: at 60 columns only one pane is shown and the requested set is unchanged; at 40×10 the "Terminal too small" screen

**Checkpoint**: GTK runs as a daemon client with `app_suite` green and
unchanged; macOS is unchanged; the terminal crate has its loop, input, theme
and layout but draws no mail yet.

---

## Phase 3: User Story 1 — Triage the inbox from a terminal (Priority: P1) 🎯

**Goal**: sidebar, windowed list, reader pane and every triage verb, offline.

**Independent test**: start `postio-tui` over a fixture store through the local
transport, no network; drive it with keys; assert the rendered buffer and the
store (quickstart "triage").

- [ ] T033 [P] [US1] Write `crates/postio-tui/src/view/sidebar.rs` over `postio_ui::sidebar` (folders, views, saved searches, counts, accounts). Test first: a snapshot of the fixture sidebar shows Inbox, Flagged, Snoozed and a saved search
- [ ] T034 [P] [US1] Write `crates/postio-tui/src/view/list.rs` drawing only the `postio_ui::list::ListWindow` slice with the marks of contracts/tui-surface.md (`●`, `⚑`, `▌`, `›`), wide characters aligned. Test first: a snapshot with a CJK subject and an emoji sender keeps columns aligned
- [ ] T035 [US1] Wire paging through `postio_ui::paging::Paging` and `Client::page`. Test first (counts, Principle V): scrolling one row costs at most one `Page` request and zero `Body` requests; a 100,000-row fixture scope never requests more than the window
- [ ] T036 [US1] Wire cursor and `postio_ui::selection::SelectionState` and aim commands with `postio_core::aim::command_for`. Test first: US1 scenario 3 — three selected, cursor on a fourth, `a` archives the three only
- [ ] T037 [US1] Write `crates/postio-tui/src/view/notice.rs`: the undo notice ("Archived 12 messages — Undo") from `UndoEntry::description`. Test first: US1 scenario 4 — twelve archives in a burst, one `u`, all twelve return
- [ ] T038 [US1] Write `crates/postio-tui/src/view/status.rs` from `postio_ui::status::SyncStatus` and `list_state` (offline, empty, failed). Test first: with the mock backend offline the list is navigable and the status line says offline (US1 scenario 1)
- [ ] T039 [US1] Write `crates/postio-tui/src/main.rs` startup: caps → connect (spawning the daemon) → enter terminal → first frame; exit codes and messages per contracts/tui-surface.md. Test first: a version-mismatched daemon yields exit 1 and the sentence naming both versions

**Checkpoint**: triage works end to end in a terminal; the reader pane shows a
plain-text placeholder until US2.

---

## Phase 4: User Story 2 — Read mail as Markdown (Priority: P1)

**Goal**: sanitised HTML → Markdown → styled lines; folds, links, placeholders,
attachments.

**Independent test**: render every corpus message and assert the SC-005
properties on the grid; US2 scenarios on the fixture store.

- [ ] T040 [US2] Write `postio_body::markdown::from_html` in `crates/postio-body/src/markdown.rs` wrapping htmd (T005 passed; research R5) with custom `img` handling (placeholder, never dropped) and `details` handling, keeping `<details>` as fold markers and images as `postio-image:<identity>`. Test first: T005's corpus test, now against `from_html`
- [ ] T041 [US2] Write `crates/postio-tui/src/view/reader/render.rs`: Markdown → `RenderedMessage` via tui-markdown with a `StyleSheet` from `theme.rs`, a `linkify` pass, every span through `SafeText`. Test first: a corpus-wide test that the rendered grid contains no control character from content and no tag (SC-005)
- [ ] T042 [US2] Plain-text path in the same file: verbatim with `>` runs folded by `postio_body::quote`, never parsed as Markdown. Test first: US2 scenario 3, a `# not a heading` line renders literally
- [ ] T043 [US2] Folds: collapsible `Fold` blocks, folded by default, toggled by the existing expand/collapse-quote commands. Test first: US2 scenario 1 snapshot, then the snapshot after expanding
- [ ] T044 [US2] Images and remote content: placeholders `[image: alt · size]`, held-back counts in the header, the "allow remote images from this sender" command through `AllowRemoteImages`. Test first: US2 scenario 2 — the mock backend and the egress recorder see zero requests while the message is open
- [ ] T045 [US2] Links: a link focus mode that shows the full target, opening only on deliberate activation (xdg-open, or show-and-copy via OSC 52 when no opener exists). Test first: US2 scenario 4 — nothing is opened until the second activation
- [ ] T046 [US2] Attachments: list with name and size; save (`SavePart`) and open (`OpenPart` + xdg-open), fetching only then. Test first: US2 scenario 5 — the payload is requested only after "open"
- [ ] T047 [US2] Conversations: `J`/`K` walk messages of one scrolling document (ADR 0032's shape) with per-message headers from `postio_ui::reader::header`. Test first: `J` moves to the next message's header row
- [ ] T048 [US2] Unsubscribe on deliberate activation only, through `ActivateUnsubscribe`. Test first: opening a list message sends nothing; the command sends once

---

## Phase 5: User Story 3 — Write mail in Markdown (Priority: P1)

**Goal**: modeless Markdown composer in the reading pane; same HTML as GTK;
the text part is the Markdown; drafts shared; drop and paste.

**Independent test**: compose through keys against the mock backend and assert
the queued bytes (quickstart §3).

- [ ] T049 [P] [US3] Write `postio_body::markdown::to_document` in `crates/postio-body/src/markdown.rs` per contracts/markdown.md's table. Tests first: the table row by row; a fuzz target in `crates/postio-body/fuzz/` that it never panics; `![](http…)` is text, never an image
- [ ] T050 [P] [US3] Write `postio_body::markdown::from_document`. Test first: property test `to_document(from_document(d)) == d` for generated `Quoted`-free Documents
- [ ] T051 [US3] HTML parity test in `crates/postio-body/tests/body_suite/markdown_parity.rs`: for each construct in the table, `render(to_document(md)).1` equals the HTML the GTK composer produces for the same content built through `postio_body::edit`. This is SC-006
- [ ] T052 [US3] Text part: `format=fixed` for Markdown-authored messages in `crates/postio-model/src/outgoing.rs` (the flowed path unchanged for GTK), text = Markdown `++ Quoted::text()` with `> `; text-only when `is_plain_text()`. Tests first: US3 scenarios 2 and 3 on the assembled MIME
- [ ] T053 [US3] Write `crates/postio-tui/src/view/composer.rs`: header fields (tui-input), body (ratatui-textarea, modeless, FR-022a), Cc/Bcc on demand, identity picker, the read-only foldable quote region below the editor. Test first: with the composer focused, typing `a` inserts `a` and archives nothing
- [ ] T054 [US3] Reply, reply all, forward, new: `NewDraft` from the registry commands; the composer takes the reading pane; `Esc` returns to the exact list position. Test first: US3 scenario 1
- [ ] T055 [US3] Recipient autocomplete from `Recipients(prefix)`. Test first: typing `ad` offers the fixture contact `ada@example.com`
- [ ] T056 [US3] Autosave with a 1500 ms debounce through `SaveDraft` writing markdown, text and html; reopen from `body_markdown` or `from_document(parse(html))`. Tests first: US3 scenario 5 — a terminal draft reopens verbatim, and a GTK-saved draft reopens as Markdown
- [ ] T057 [US3] Live preview (`[tui].preview`, split or toggle) rendering the would-be HTML part through T041's renderer. Test first: the preview shows **bold** styled where the source shows `**bold**`
- [ ] T058 [US3] `$EDITOR` handoff through `term::with_suspended` with a `0600` file in `$XDG_RUNTIME_DIR/postio/`, deleted on return. Test first: US3 scenario 6 with a fake editor script that appends a line; the file is gone afterwards
- [ ] T059 [US3] Send with `Ctrl+Enter` (and its legacy alternate from T017) through `QueueSend`; scheduled send through `QueueSend{at}`. Test first: US3 scenario 7 — offline send lands in the Outbox at once, and one submission reaches the mock when it comes online
- [ ] T060 [P] [US3] Write `postio_ui::paste::classify` in `crates/postio-ui/src/paste.rs` per data-model.md: `file://` URIs, shell quoting and escapes, newline/space separation, existence and readability checks. Tests first: prose that looks like a path is `Text`; an unreadable absolute path is `Unreadable`
- [ ] T061 [US3] Drop and paste in the composer: `Event::Paste` → `classify` → attach, notice, or insert. Tests first: US3 scenarios 8, 10 and 11
- [ ] T062 [US3] Write `crates/postio-tui/src/clipboard.rs`: image read on the paste key via arboard, then `wl-paste`, then `xclip`, never otherwise; `AttachBytes{inline:true}` and `![image](cid:…)` at the cursor; "Clipboard unavailable here" when none. Tests first with a fake clipboard: US3 scenario 9; the clipboard is read zero times while typing
- [ ] T063 [US3] Pop-out equivalent: the pop-out command moves the composer to a tab (FR-003). Test first: the same draft id is in the tab, and the reading pane returns to the reader
- [ ] T064 [US3] Attaching by typed path (FR-027) through a path prompt with completion. Test first: attaching `./fixture.pdf` lists it with its size

---

## Phase 6: User Story 4 — Search, palette and jump (Priority: P2)

**Goal**: `/` search as-you-type in the one query language; `Ctrl+K` palette
and `?` cheat sheet from the registry.

**Independent test**: type corpus queries and assert the list; enumerate the
palette against the registry.

- [ ] T065 [P] [US4] Write `crates/postio-tui/src/view/search.rs` over `postio_ui::search` (chips, backspace, `Pacer`) and `Client::search`. Test first: US4 scenario 1 — `from:ada is:unread` typed character by character updates on each key and `is:` shows no error
- [ ] T066 [P] [US4] Write `crates/postio-tui/src/view/palette.rs` over `postio_ui::palette::entries` and the finder prefixes of `postio_ui::finder`. Test first: the palette lists exactly `registry::all()` available in context, each with the chord this terminal can deliver
- [ ] T067 [P] [US4] Write `crates/postio-tui/src/view/cheatsheet.rs` over `postio_ui::cheatsheet::sections`. Test first: every section and binding appears
- [ ] T068 [US4] Write the registry parity test `crates/postio-tui/tests/registry_parity.rs` (SC-001): for every command, reachable by chord (with fallback) and by palette; failing names the command. This is the test that keeps parity from drifting
- [ ] T069 [US4] Saved searches from `config.toml` in the sidebar returning the same results as GTK. Test first: US4 scenario 3 against the same fixture as the GTK case

---

## Phase 7: User Story 5 — Everything reachable by mouse too (Priority: P2)

**Goal**: every gesture in contracts/tui-surface.md §Mouse.

**Independent test**: synthetic mouse events at known cells; assert screen and
store.

- [ ] T070 [US5] Hit testing in `crates/postio-tui/src/view/hit.rs`: each frame records rects → targets; mouse events resolve against the last frame. Test first: a click on the third list row resolves to that row
- [ ] T071 [US5] List and sidebar clicks, `Ctrl`+click toggle, `Shift`+click extend. Tests first: US5 scenario 1
- [ ] T072 [US5] Wheel scrolls only the pane under the pointer. Test first: US5 scenario 2
- [ ] T073 [US5] Links, fold markers, placeholders and attachments respond to clicks (same commands as keys). Test first: a click on a fold marker expands it
- [ ] T074 [US5] Pane divider drag (Down, Drag…, Up) with widths persisted in window state. Test first: dragging 10 columns right widens the list by 10 and survives a restart
- [ ] T075 [US5] Click to place the cursor in the composer via `screen_to_data` + `CursorMove::Jump`. Test first: clicking column 5 of body line 2 puts the cursor there
- [ ] T076 [US5] `[tui].mouse = false` and a terminal reporting no mouse. Test first: US5 scenario 3 — clicks are ignored and every key path still works

---

## Phase 8: User Story 6 — One store, both frontends (Priority: P2)

**Goal**: GTK and the terminal live on one daemon, with no configuration.
(The architecture is in Phase 2; this phase proves the story and closes its
edges.)

**Independent test**: `crates/postio-host/tests/two_clients.rs` against a real
daemon on a temporary runtime dir (quickstart "two_clients").

- [ ] T077 [US6] Write `crates/postio-host/tests/two_clients.rs` harness: a real `postio-daemon` on a temp `$XDG_RUNTIME_DIR` and data dir with the mock backend, and two `Client`s (kinds Gtk and Tui)
- [ ] T078 [US6] In `crates/postio-host/tests/two_clients.rs`: US6 scenario 1: an archive in one client is absent from the other's next `Page` and its event arrives within 1 s (test clock-free: assert on event receipt)
- [ ] T079 [US6] In `crates/postio-host/tests/two_clients.rs`: US6 scenario 2: new mail from the mock is fetched once (the mock counts fetches) and appears in both
- [ ] T080 [US6] In `crates/postio-host/tests/two_clients.rs`: US6 scenario 3 / SC-007: a scripted mixed session from both clients (archive, flag, move, send) — the mock sees each remote effect exactly once and one submission per send
- [ ] T081 [US6] In `crates/postio-host/tests/two_clients.rs`: US6 scenario 4 and FR-043: disconnect one client mid-sync; the other keeps receiving events and the queue drains; then disconnect both and the daemon exits after the grace period
- [ ] T082 [US6] In `crates/postio-host/tests/two_clients.rs`: Draft crossing: a draft saved by the Gtk client with HTML is reopened by the Tui client as Markdown, and a Tui draft opens in the Gtk client with its formatting (FR-023)
- [ ] T083 [US6] `postio-diag` asks the daemon rather than opening the store (`crates/postio-session/src/bin/postio-diag.rs`). Test first: `postio-diag` with a daemon running reports its state instead of "nothing running"

---

## Phase 9: User Story 7 — Set up and configure without the desktop app (Priority: P3)

**Goal**: add an account (preset discovery, password, app password, OAuth)
and change every setting from the terminal.

**Independent test**: onboarding against the mock backend and a fake keyring
through the local transport.

- [ ] T084 [US7] Write `crates/postio-tui/src/view/onboarding.rs`: with no accounts, the first screen offers to add one (US7 scenario 1). Test first: the empty-store snapshot
- [ ] T085 [US7] Discovery and password / app-password flows through `Discover` and `AddAccount`, with the human error sentences of `onboarding::explain`. Test first: a wrong password shows the same sentence GTK shows
- [ ] T086 [US7] OAuth: `BeginOAuth` returns the consent URL; show it in full, copy via OSC 52, open only on activation; the daemon's loopback completes it. Test first: US7 scenario 2 — nothing is opened or fetched until the user acts
- [ ] T087 [US7] Write `crates/postio-tui/src/view/settings.rs` generated from `postio_ui::settings` sections over `Settings`/`PatchSettings`. Test first: every section GTK shows is present (enumerated from `postio_ui::settings`, not listed by hand)
- [ ] T088 [US7] US7 scenario 3: a setting changed in the terminal is picked up live by a running GTK client (through the daemon's config reload). Test first in `two_clients.rs`

---

## Phase 10: Polish & cross-cutting

- [ ] T089 [P] Flatpak for the terminal: `flatpak/dev.postio.PostioTui.json` on `org.freedesktop.Platform` with `postio-tui` and `postio-daemon`, `--filesystem=xdg-data/postio:create`, `--filesystem=xdg-run/postio:create`, `--socket=wayland`, `--socket=fallback-x11` (clipboard), `--talk-name=org.freedesktop.secrets`, `--talk-name=org.freedesktop.Notifications`
- [ ] T090 [P] Desktop flatpak: add `postio-daemon` to `flatpak/dev.postio.Postio.json` and the two shared filesystem grants; the store path is `~/.local/share/postio` everywhere (existing flatpak stores resync — say so in `flatpak/README.md`)
- [ ] T091 Release: `.github/workflows/release.yml` gains `tui-flatpak` and `tui-tarball` jobs, each `needs: suite`, uploading `dev.postio.PostioTui-${VERSION}-x86_64.flatpak` and `postio-tui-${VERSION}-x86_64-linux.tar.zst`, with SBOM and provenance like the desktop bundle. Both set `POSTIO_GIT_COMMIT` so `BuildId` carries the commit: without it, two builds of one version would handshake as equal (T021)
- [ ] T092 Write `scripts/measure-package-size.sh` (quickstart §6) and a CI assertion that each terminal package is under half its desktop counterpart (SC-004); record the first numbers in research.md R10
- [ ] T093 Memory: measure RSS of `postio-tui` vs the GTK client on the reference mailbox (quickstart §6), record in `docs/PERFORMANCE.md`; SC-004's half is the bar
- [ ] T094 Startup budget: a nightly measurement (`POSTIO-MEASUREMENT:` marker, excluded from `profile.default` in `.config/nextest.toml`) of cold (daemon spawn) and warm start to first usable frame against 500 ms (SC-003)
- [ ] T095 [P] Amend `.specify/memory/constitution.md` Additional Constraints → Scope to name the terminal frontend beside GTK; bump 1.1.0 → 1.2.0 with the Sync Impact Report; update Principle VII's boundary list with `postio-tui`/`postio-client`
- [ ] T096 [P] Update `docs/PRODUCT.md` §2 (Platforms) and §23 (v1 scope), and `docs/ARCHITECTURE.md` (the shape diagram: daemon, client, host; §9 boundaries), citing ADR 0041 rather than restating it
- [ ] T097 [P] Move ADR 0041 to Accepted if the maintainer agrees at review, and update `docs/decisions/README.md`
- [ ] T098 [P] `docs/book/` (or README) section: installing and running `postio-tui`, the flatpak alias, SSH notes, clipboard caveats
- [ ] T099 Run quickstart.md end to end by hand and record the outcome in the PR body; confirm `postio-tui` is in `default-members` and the `ci.yml` `changes` job builds it
- [ ] T100 Final FR-005 audit: diff the test files of `postio-app`, `postio-gtk`, `postio-ffi` and `macos/` against `origin/main`; any change beyond import paths is explained in the PR or reverted

---

## Dependencies & execution order

```text
Phase 1 Setup ──► Phase 2 Foundational ──┬─► US1 ──► US2 ──► US3 ──┐
   (T005–T007 spikes gate T020, T040,    ├─► US4 (after US1)        ├─► Phase 10
    T017 before anything else in P2)     ├─► US5 (after US1; T075 after US3)
                                         ├─► US6 (after T020–T024)  │
                                         └─► US7 (after US1) ───────┘
```

- **Inside Phase 2**: T008 → T009 → T010/T011 → T012 → T013 → T014–T016
  (sequential: they share `postio-host`) → T018 → T019; then T020 → T021 →
  T022 → T023; T024 any time after T015. T025–T032 are [P] with each other
  and with T014–T019.
- **US2 depends on US1** for the reader pane; **US3 on US2** for the preview
  renderer (T057) and on T024 for drafts.
- **US4, US5, US7** need only US1's shell; US5's T075 needs US3's composer.
- **US6** needs the daemon (T020–T024) and uses US3's draft path in T082.

## Parallel opportunities

- **Phase 2**: the terminal foundations T025–T032 run beside the host/client
  work T014–T019; the three spikes T005–T007 run together.
- **US1**: T033 and T034 together.
- **US3**: T049, T050 and T060 together (three different files); T051 after
  T049.
- **US4**: T065, T066 and T067 together.
- **Polish**: T089, T090, T095, T096, T097 and T098 together.

## Implementation strategy

- **Build order, not release order.** Nothing lands until every story passes
  (clarified 2026-09-23). The first point where the work is *usable* is the
  end of US1 on top of Phase 2, and the first point where it is *what was
  asked for* is the end of US3; both are checkpoints for a by-hand run of
  quickstart, not merges.
- **Phase 2 is the risk.** It changes the GTK app's plumbing without changing
  its behaviour; `app_suite` unchanged at every commit is how that is
  proven. If T007 finds cold start over budget, or T018 cannot keep
  `app_suite` unchanged, stop and re-plan — do not trade FR-005 for progress.
- **Rebase onto `main` as you go**; a long branch that touches `postio-app`
  this widely will meet other sessions' changes, and the rebase is what finds
  a shared type's new callers.
- **Commit per task**, `Refs: specs/005-tui-frontend` and the task id; land
  with `scripts/issue-land.sh --detach` once T100 is done.
