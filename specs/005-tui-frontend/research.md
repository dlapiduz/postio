# Research: Postio in the terminal

Phase 0 for [plan.md](./plan.md). Each section is a decision, why it was taken,
and what it was chosen over. Versions were checked on 2026-09-23 against
crates.io; line numbers are against `origin/main` at `d08ff473`.

**On inherited decisions.** The maintainer asked (2026-09-23) that no ADR be
assumed to apply to the terminal frontend just because it exists. Each ADR
below is cited only where its *reasoning* holds for a terminal; where it was a
decision about the GTK surface, this document says so and decides afresh.

---

## R1. Two frontends on one store: one process owns it

**Decision**: A new headless process, `postio-daemon`, is the only process that
opens the store. It owns the store, the blob directory, the keyring key, the
sync engines, the operation-queue drainer, the event hub, the undo stacks and
the egress recorder. The GTK app and `postio-tui` are its clients, over a Unix
socket in `$XDG_RUNTIME_DIR`. The first frontend to start spawns it; it exits
a short grace period after its last client leaves. The macOS frontend keeps
running the same host in-process (R2), so it loses nothing and needs no
daemon.

**Rationale** — measured, not inherited:

- **Turso refuses a second process.** `turso_core 0.8.0-pre.11` takes an
  exclusive `fcntl` lock on the database file unless opened `NoLock`/
  `ReadOnly` (`turso_core/io/unix.rs:278-298`; upstream's own test,
  `multiprocess_tests.rs:928`, "default non-multiprocess open should stay
  DB-file locked across processes"). Postio takes no lock of its own; the only
  thing that keeps two openers apart today is that the GTK app is a
  single-instance `gtk::Application` (`crates/postio-app/src/lib.rs:186-200`).
  A second process gets `LimboError::LockingError`.
- **The experimental multi-process mode is not a foundation.**
  `Builder::experimental_multiprocess_wal(true)` coordinates through a `.tshm`
  sidecar and does accept built-in encryption. But it **refuses `VACUUM`**
  (`translate/vacuum.rs:45`), which is how the store reclaims pages, it has
  open panics and aborts upstream (#7213, #8348, #9005, filed as late as
  2026-09-15), the sidecar format "may change", the FTS cross-process snapshot
  fix (#8975) post-dates our pin, and there is no cross-process commit signal
  (#7397).
- **Even with a working multi-process engine, two hosts would be wrong.** The
  queue claim `OperationRepository::mark_in_flight`
  (`crates/postio-storage/src/repository/operations.rs:599`) is an
  unconditional write, not a compare-and-set. It is correct because exactly one
  drainer exists. `LocalStore` keeps per-process caches (count witnesses,
  `Marks`, `note_removed`: `crates/postio-runtime/src/store/local.rs:55-247`)
  that another process's commit would silently stale. Exactly-once send
  (ADR 0021) and "each remote effect once" (FR-042) hold by construction with
  one process and only by careful election with two.
- **The seams already exist.** `postio_session::Wiring`
  (`crates/postio-session/src/lib.rs:291-392`) is a GTK-free host. `Command`
  and `Event` already derive serde (`crates/postio-core/src/command.rs:357`,
  `event.rs:88`). `EventHub::subscribe(label)` (`bridge.rs:536`) gives every
  subscriber every event, which is exactly one socket connection's stream.
  The FFI `Session` already states the contract the socket needs: "commands
  go down and events come up" (`crates/postio-ffi/src/session.rs:310-317`).
- **Freshness comes free.** A change reaches the other frontend as the event
  the daemon already emits, in microseconds, well inside FR-041's one second,
  with no polling and no file watching.
- **The key stays in one place.** Only the daemon reads the keyring. A
  frontend never holds the store key.

**Alternatives considered**:

- *Both processes open the store; a lock file elects who syncs.* Needs the
  experimental mode above, still needs a second channel for change
  notification, and makes the queue's correctness depend on the election never
  splitting. Rejected.
- *The first frontend hosts; the second connects; hand over on exit.* The
  surviving frontend must switch from client to host mid-session, and can only
  do so once the old process has released Turso's lock. Sync stops for as long
  as that takes, and it doubles the paths to test. Rejected.
- *The terminal frontend refuses to start while the GTK app is open.* Free and
  correct, and it is the fallback message if the daemon cannot be reached, but
  it is not what was asked for. Rejected as the design.

**Consequences taken on**: `postio-app` reaches the store and blobs directly
about 78 times across 13 files (`lib.rs`, `search.rs:303-339,736`,
`compose.rs:661`, `settings_accounts.rs`, `reading.rs`, …). Each becomes a
request on the protocol. This is the bulk of the non-terminal work, and it is
also FR-004's "logic once": the same requests are what the terminal frontend
makes. `postio-gtk` and `postio-ui` never name the store, so they are
unaffected. This decision outlives the feature and binds all later work, so it
is recorded as **ADR 0041** (Proposed on this branch).

### R1a. Undo across two frontends

**Decision (default, flagged to the maintainer)**: The daemon keeps one undo
stack **per client**. `u` in the terminal undoes the last thing done in the
terminal, and the same for the GTK app. Entries carry their origin; the
existing coalescing (1 s) and expiry (600 s) in `UndoStack`
(`crates/postio-core/src/undo.rs:212`) apply per stack.

**Rationale**: The undo notice appears in the frontend where the action
happened. An undo that reverses something done in another window, which the
user may not be looking at, is the "surprise rather than a mercy" that
`PRODUCT.md` §16 says the stack forgets to avoid.

**Alternative**: one global stack. Simpler, and defensible for a single user.
Rejected by default; one line of configuration if the maintainer prefers it.

### R1b. The daemon's lifetime

**Decision (default, flagged to the maintainer)**: The daemon exits 30 s after
its last client disconnects. It does not linger to sync with no frontend
open.

**Rationale**: Today Postio syncs exactly while it is open. A daemon that stays
alive with no window changes when mail is fetched without the user having
asked, which Principle VI treats as a product decision, not an engineering
one. The grace period covers quitting one frontend and opening the other.

**Alternative**: linger indefinitely (systemd user service). Better for
notifications and fresh mail on open. Left to the maintainer.

### R1c. Notifications with two frontends

**Decision**: The daemon decides whether an arrival notifies
(`postio_ui::notify::decide`, unchanged) and tells exactly one client to
deliver it: the GTK client if one is connected (so a click still raises the
window, as today), otherwise the terminal client, which delivers through the
desktop notification service over D-Bus when reachable and an in-screen notice
otherwise. One arrival produces one notification.

---

## R2. The frontend-neutral seam: `postio-client` and `postio-host`

**Decision**: Two new library crates.

- **`postio-host`**: the daemon. `Wiring` plus the store-side logic that
  today sits in `postio-app` (body loading, thread assembly, draft storage and
  autosave, send queueing, identities and signatures, recipient suggestions,
  part open/save, onboarding persistence and the OAuth loopback, settings
  writes, the remote-image allowlist, unsubscribe activation, startup
  maintenance). It serves the protocol. Binary: `postio-daemon`. It also
  exposes an **in-process transport**, so a frontend can run the host inside
  itself.
- **`postio-client`**: what every frontend holds. A typed facade: commands,
  an event stream, and queries (paging, reading, search, compose, parts,
  onboarding, settings). It owns the wire format and the socket transport,
  and it spawns the daemon when none answers.

The GTK app, the terminal frontend and `postio-ffi` all talk to
`postio-client`. GTK and the terminal frontend use the socket. `postio-ffi`
and the integration suites use the in-process transport, so the macOS
frontend is unchanged in behaviour and `app_suite` needs no daemon.

**Rationale**: `postio-ffi`'s `Session` already is most of this facade, but it
is read-only: its default `Bridge` has a no-op handler and it never calls
`actions::wire` (`crates/postio-ffi/src/session.rs:~890`). Its query surface
(`open_scope`, `row_at`, `invoke`, `search`, `palette_entries`,
`reader_document`, `next_event`) is the model for `postio-client`'s API,
extended with writes. Frontend presentation logic stays in `postio-ui`, which
is already toolkit-free (keymap `Resolver`, `ListWindow`, `Paging`,
`SelectionState`, sidebar, palette, cheat sheet, notify, status, format).

**Alternatives**: one crate holding protocol, client and host (drags the store
engine into every frontend's graph, so the terminal binary would carry it
twice); a protocol-only crate plus separate client and host (a third crate
with no behaviour of its own).

---

## R3. The terminal toolkit: ratatui on crossterm

**Decision**: `ratatui 0.30` (MIT) with the `crossterm 0.29` backend (MIT).

**Rationale**: ratatui is the standard Rust terminal UI library and is
immediate-mode: the frontend draws a frame from state, which fits Postio's
"events up, repaint" loop exactly. 0.30 split it into `ratatui-core` plus
widgets, so third-party widgets depend only on the core. crossterm provides
everything the spec needs: mouse capture with modifiers, drag and wheel;
bracketed paste (`Event::Paste`, which is how terminals deliver drag-and-drop);
the kitty keyboard protocol (`PushKeyboardEnhancementFlags`), which is what
distinguishes `Ctrl+Enter` and `Ctrl+Shift+…`; OSC 52 copy; resize events.

**What ratatui does not do, and Postio already does**: windowing. `List` and
`Table` draw what they are handed; the frontend hands them only the visible
rows from `postio_ui::list::ListWindow`, exactly as the GTK list does.

**Alternatives**: `termina` (Helix's backend, strong on keyboard protocols,
but the widget ecosystem assumes crossterm); `termion` (weaker keyboard
protocol support); `termwiz` (heavy, and incompatible with the image crate
R8 needs next); `cursive` (no release since 2024-08, retained-mode);
`tuirealm` (a component framework on ratatui; its pattern is borrowed, not
the dependency).

**Prior art borrowed** (licences checked; nothing AGPL):
**himalaya-tui** (Pimalaya, MIT/Apache: ratatui + io-imap, three panes,
composer, `$EDITOR`; the closest project, and "Pimalaya first" applies in
spirit), **gitui** (app structure, key config, testing), **yazi** (mouse,
suspend/resume, `$EDITOR`, images), **zellij** (keyboard protocol), **aerc**
(Go, MIT; composer UX). **meli** and **neomutt** are GPL: ideas only, no code.
**helix** is MPL-2.0: copied files would stay MPL, so patterns only.

---

## R4. Key and mouse input

**Decision**: A crossterm adapter turns `KeyEvent` into
`postio_ui::keymap::Chord` and feeds the existing `Resolver`
(`crates/postio-ui/src/keymap.rs:801-917`), which already implements
`[keys]` overrides and conflict reporting. There is no terminal keymap.

- The kitty keyboard protocol is enabled when
  `supports_keyboard_enhancement()` says so.
- A command whose default chord a legacy terminal cannot deliver
  (`Ctrl+Enter`, some `Ctrl+Shift` chords) gets an **additional** default in
  the registry's `alternate_bindings`, never a replacement. The GTK app sees
  one more alternate binding and nothing else (FR-005).
- A test enumerates `registry::all()` and asserts that every command has at
  least one chord a legacy terminal can deliver (SC-001).
- Mouse: click, wheel, `Shift`/`Ctrl`-click and drag, mapped onto
  `SelectionState` and the same commands the keys run. A pane divider is
  dragged as `Down`, `Drag`…, `Up`.

---

## R5. Reading: sanitised HTML → Markdown → styled lines

**Decision**:

1. The body goes through the reader's existing path unchanged:
   `postio_body::sanitize::sanitize_body_in` (`sanitize.rs:282`) with the
   sender's remote-image permission, then the reader-view or original
   choice (`postio_ui::reader::document::suits_reader_view`, `:646`).
2. Sanitised HTML is converted to CommonMark by a new
   **`postio_body::markdown::from_html`**, built on **`htmd`** (Apache-2.0,
   an html5ever-based HTML→Markdown converter; `postio-body` already depends
   on html5ever 0.39). It keeps tables, which `postio_body::parse` flattens
   (`parse.rs:185`). If `htmd` fails the corpus test (below), the fallback is
   our own walk over the same html5ever tree, following htmd's structure.
3. Markdown is drawn with **`tui-markdown`** (MIT/Apache, by a ratatui
   maintainer, on pulldown-cmark 0.13), with a `StyleSheet` built from R9's
   palette. Bare URLs are linked by a `linkify` pass, since pulldown-cmark
   does not autolink them.
4. **Plain-text messages are not parsed as Markdown.** They are shown
   verbatim, with `>` runs folded (`postio_body::quote`), so a line starting
   `#` stays a line starting `#` (US2 scenario 3).
5. **Quote folding** reuses `fold_html_quotes`
   (`crates/postio-body/src/quote.rs:34`). Folded regions become collapsible
   blocks in the rendered message, one per `<details>`.
6. **Escape sequences are neutralised at one chokepoint**: every string that
   originated in mail passes through `postio_ui::terminal::sanitize` (C0, C1,
   ESC, DEL, bidi overrides) before it becomes a styled span. No widget takes a
   `&str` from mail any other way, and a test holds the whole corpus to that.
7. Images, inline or remote, are placeholders
   (`[image: alt · 34 KB]`) occupying the image's place in the rendered
   message (R8).

**Rationale**: `document.rs:412` says "do not reach for html2text — convert
from here". That warning is about losing what the sanitiser guarantees, and
converting *after* the sanitiser keeps it. Converting through the composer
`Document` instead was rejected for reading because it is a deliberately
narrow subset (h1–h3 only, no tables, no strikethrough, cid-only images), so
FR-010's "tables at least as faithfully as plain text" would fail.

**Spike T0.1 result (2026-09-23, T005)**: `htmd 0.5.5` over every corpus
message with an HTML body, after `sanitize_body(…, Blocked)` and
`fold_html_quotes`: **all safety promises hold**, with no tag, script,
`javascript:` or remote image in any output, and a GFM table survives as a
table (`crates/postio-body/tests/body_suite/markdown_corpus.rs`). Two
structural gaps: an `<img>` whose remote `src` the sanitiser stripped
**disappears** instead of leaving a placeholder, and `<details>`/`<summary>`
fold markers are dropped, keeping their content. Both are handled with
`htmd`'s custom element handlers, so **T040 wraps htmd** with `img` and
`details` handlers rather than writing the walk. **Cost to watch**: htmd
0.5.5 depends on html5ever **0.38** while `postio-body` uses 0.39, so the
graph carries two HTML parsers. T092's size measurement decides whether that
matters for SC-004. If it does, the fallback is our own walk on 0.39 (a few
hundred lines, with htmd as the structural reference).

**Test**: a corpus-wide test in the pattern of
`crates/postio-body/tests/body_suite/replying.rs:639` renders every
`crates/postio-model/tests/corpus/*.eml` through the terminal reader and
asserts: no raw tag, no script text, no remote URL rendered as an image, no
control character from message content (SC-005).

**Alternatives**: `html2text` (himalaya-tui's choice: plain text, loses
structure); `termimad` (its own non-CommonMark parser, writes to the terminal
directly, not a ratatui buffer); `mdcat` (a program, not a widget; ideas
borrowed).

---

## R6. Writing: Markdown in, the same HTML out

**Decision**:

- **The editing surface** is **`ratatui-textarea` 0.9** (MIT, the ratatui
  organisation's maintained fork of `tui-textarea`). It is modeless, with
  undo/redo, selection, a yank buffer, soft wrap and wide characters. Click to
  place the cursor is wired with `screen_to_data` and `CursorMove::Jump`.
  Single-line fields (To, Cc, Subject) use `tui-input`. `edtui` was rejected
  because it is vim-first, and the composer is modeless (clarified
  2026-09-23).
- **Markdown → Document**: a new `postio_body::markdown::to_document` on
  **pulldown-cmark 0.13** (MIT), mapping every construct the `Document`
  represents (contracts/markdown.md). Constructs it does not represent
  (tables, strikethrough, task lists, footnotes, raw HTML, remote images)
  become their Markdown source text inside a paragraph. The HTML part is then
  `postio_body::outgoing::render` (`outgoing.rs:124`), the generator the GTK
  composer uses, so HTML parts are byte-identical (SC-006).
- **The plain-text part is the Markdown as written (FR-021).** ADR 0003
  rejected a Markdown-authored composer *for the GTK editor* in favour of
  WYSIWYG. The maintainer has asked for Markdown writing in the terminal,
  where there is no WYSIWYG to prefer, so that rejection does not apply here.
  What does apply is ADR 0003's privacy constraint, kept verbatim: nothing in
  compose may fetch a remote URL. `![](https://…)` is never fetched; it goes
  out as text.
- **Plain only when nothing was formatted**: if
  `to_document(markdown).is_plain_text()`
  (`crates/postio-body/src/document.rs:358`), the message goes as text/plain
  only: exactly the GTK rule (`crates/postio-gtk/src/composer.rs:1227-1234`).
- **The text part is not `format=flowed`.** Flowed text's trailing-space
  soft breaks conflict with Markdown's two-space hard breaks. The terminal
  path sets `format=fixed` on its text part; the GTK path is unchanged.
- **Replies**: the opaque quote (`postio_body::replying::quote_of`,
  `replying.rs:135`) cannot be edited as Markdown without losing what
  ADR 0033 exists to keep. The composer shows it below the editor as a
  read-only, foldable `> ` region, which the user can keep or remove whole.
  The HTML part carries the `Quoted` block untouched; the text part carries
  `Quoted::text()` with `> ` prefixes after the user's Markdown. ADR 0033's
  reasoning (a quote should look like the message being answered) holds in a
  terminal too, so it is followed.
- **Drafts** gain one column, `body_markdown TEXT` (NULL by default). The
  terminal frontend writes it together with `body_text` and `body_html`; a
  GTK save writes NULL. Reopening in the terminal uses `body_markdown` when it
  is present, and otherwise `postio_body::markdown::from_document(parse(html))`,
  a new serializer (the inverse mapping in contracts/markdown.md). So a draft
  moves between frontends without losing anything the document represents
  (FR-023). Old drafts simply have NULL.
- **`$EDITOR`** receives the Markdown as a temporary file with mode 0600 in
  `$XDG_RUNTIME_DIR`, deleted on return. The terminal is restored and
  re-entered around it, as in yazi and helix. Markdown survives an editor
  losslessly, which is the property ADR 0003 records the GTK composer as
  having given up.
- **Live preview**: the same R5 renderer applied to the HTML part that would be
  sent, in a split or a toggle. It shows the result of the real generator,
  not a second guess at it.

---

## R7. Drop, paste and clipboard

**Decision**: follow the pattern terminal applications already use (the
maintainer cited Claude Code).

- **Drag and drop**: terminals deliver a dropped file as its path inside a
  bracketed paste. `postio_ui::paste::classify(text)` (new, pure) splits a
  paste into paths and text: `file://` URIs (percent-decoded), shell-quoted
  and backslash-escaped paths, newline- or space-separated. A token is a path
  only if it names an existing, readable regular file; everything else is
  text (US3 scenario 10, edge case "looks like a path but is prose").
- **Clipboard image**: on the paste key, the composer asks the clipboard for
  an image. It uses **`arboard` 3.6** (MIT/Apache) with `wayland-data-control`
  and `image-data`, falling back to `wl-paste --list-types`/`--type image/png`
  and then `xclip`. This is what Claude Code does, and what a compositor
  without data-control needs. The bytes go to the blob store through the host
  (`install_inline_image`, moved from `crates/postio-app/src/compose.rs:125-165`)
  and become an inline `Content-ID`, exactly as a GTK paste does.
- **The clipboard is read only on a paste** (FR-026). Nothing polls it.
- **No clipboard** (SSH, no tool): "Clipboard unavailable here", while
  bracketed paste, and therefore drops, still work.
- **Copying out** (a link, an address) uses OSC 52, so it works over SSH.

---

## R8. Images: placeholders now, pixels next

**Decision**: every image is a placeholder that occupies the image's place in
the rendered message and can be opened in the system viewer (xdg-open). The
next iteration draws them with **`ratatui-image` 11** (MIT; kitty, sixel,
iTerm2, half-blocks). Two things are done now so that iteration restructures
nothing: the placeholder is a node of the rendered message with the image's
identity and a reserved size, and startup reserves the terminal-query slot
(before raw mode) where `Picker::from_query_stdio()` will go, next to R9's
palette query.

---

## R9. Colour

**Decision** (clarified 2026-09-23): draw with the terminal's palette (the
default foreground and background plus the 16 ANSI colours). On a true-colour
terminal (`COLORTERM=truecolor|24bit`), additionally use Postio's steel accent
for the selected row and the focus ring. The accent is read from the generated
design tokens (`postio_ui::tokens`), never retyped. With `NO_COLOR` there is
no colour at all; unread, flagged, selected and focus each also have a
glyph or an attribute (bold, reverse, a marker column). Every role can be
overridden under a new `[tui.colors]` table in `config.toml`.
`terminal-colorsaurus` (MIT/Apache) queries the background for light/dark
before raw mode, with a timeout. It is not needed for the palette itself,
which the terminal applies.

---

## R10. Packaging and size

**Decision** (clarified 2026-09-23: its own flatpak, plus a standalone
download):

- **Standalone**: `postio-tui-<ver>-x86_64-linux.tar.zst` with `postio-tui`
  and `postio-daemon`, built for `x86_64-unknown-linux-gnu` against a
  conservative glibc. `postio-daemon` needs D-Bus for the Secret Service; the
  terminal frontend itself needs no display server, GTK or WebKit.
- **Flatpak**: `dev.postio.PostioTui` on `org.freedesktop.Platform` (no GTK),
  built by a second job in `.github/workflows/release.yml` and attached to the
  GitHub release, as the desktop bundle already is. Flathub does not accept
  console-only applications
  (<https://docs.flathub.org/docs/for-app-authors/requirements>). That does
  not block this, because Postio distributes its flatpak as a release bundle,
  not through Flathub. Launch: `flatpak run dev.postio.PostioTui`, and the
  bundle's docs give a shell alias.
- **Sharing the store across two flatpaks.** Each flatpak gets a private data
  directory by default, which would give each frontend its own store and break
  FR-040. So both manifests, desktop and terminal, grant
  `--filesystem=xdg-data/postio:create` and
  `--filesystem=xdg-run/postio:create`. The store and the socket then live at
  the host paths `~/.local/share/postio` and `$XDG_RUNTIME_DIR/postio`, which
  the standalone download uses too. The desktop flatpak's store moves there
  from `~/.var/app/…`. No backwards compatibility: an existing flatpak store is
  resynced.
- **Version skew**: each package ships its own `postio-daemon`. The protocol
  handshake requires an exact build match. A client that finds a daemon from
  a different build says which two versions disagree and refuses, rather than
  talking past it.
- **Size levers**: a `release-tui` profile (`lto = "fat"`,
  `codegen-units = 1`, `strip = true`, `panic = "abort"`, `opt-level = "s"`)
  for `postio-tui`. syntect is left out (code blocks are drawn without
  highlighting). The terminal frontend links no store engine at all, since
  the daemon does, so the `postio-tui` binary itself stays small. SC-004 is
  measured package against package (spec, Assumptions).

---

## R11. Testing

- **Screens**: `ratatui::backend::TestBackend` with `insta` 1.48 snapshots of
  the rendered buffer as text. Styles are asserted by indexing cells. This
  asserts on what a person would see (Principle IV).
- **Input**: the app loop is `update(&mut App, Event) -> Effects` with the
  event source injected, so tests feed synthetic `crossterm::Event` values
  (keys, mouse, `Paste`, `Resize`) and never need a tty. Terminal-frontend
  tests need no compositor, so they run in the default suite on CI.
- **Against the engine**: through `postio-client`'s in-process transport over
  a fixture store and the `MailBackend` mock, the same way `app_suite` does,
  so no test touches the network.
- **Two frontends**: an integration suite starts a real `postio-daemon` on a
  temporary runtime directory, connects two clients, and asserts US6's
  scenarios, including one send leaving once (the mock counts submissions).
- **Budgets are counts** (Principle V): rows requested per frame from
  `ListWindow`, statements per page via
  `postio_storage::test_support::counting` on the daemon side, and round trips
  per keystroke on the protocol (a new counter in `postio-client`).
- **Parity**: a test enumerates `registry::all()` and asserts that each
  command is reachable in the terminal frontend by chord and by palette (SC-001).
- **GTK and macOS unchanged**: `app_suite`, `postio-gtk`, `postio-ffi` and the
  macOS package tests pass with no change beyond import paths (FR-005). A test
  that has to be weakened is a regression.
