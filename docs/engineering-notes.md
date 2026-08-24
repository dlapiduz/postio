# Engineering notes

Hard-won lessons that aren't obvious from reading the code — the kind of thing
that used to live in `bd remember` / `bd memories`. This doc replaces that: as
issue tracking moves from beads to GitHub, this is where that knowledge has to
live instead, since it isn't tied to any single issue or PR and a future
session (or contributor) has no other way to trip over it before they hit the
same wall.

**Adding to this file**: when you learn something the hard way — a trap that
looks like a bug but isn't, an invariant that isn't visible from the type
signature, a measurement that contradicts intuition — add it under the
relevant section below, or start a new one. Write it the way these are
written: what happened, why, and what to do instead. Don't summarize away the
specifics (file names, function names, dates, bead/issue IDs) — those are
what make an entry findable later and distinguishable from a similar-sounding
one.

For architecture *decisions* and their reasoning, see `docs/ARCHITECTURE.md`
and `docs/decisions/` instead — this file is for gotchas and lessons, not
the rationale for how the system is shaped. A few entries below point back to
ARCHITECTURE.md where the full write-up already lives there.

---

## Product scope & design decisions

**Scope, as decided 2026-08-22.** Linux/GTK4 only, IMAP+SMTP only, targeting
iCloud with an app-specific password (no OAuth in v1). HTML bodies render in
WebKitGTK 6.0 locked down (JS off, network off, `cid:` custom scheme, injected
Postio CSS). Storage is SQLite for metadata/threading/sync-state/FTS5 plus a
content-addressed blob dir for raw messages and attachments — no
maildir/mbox/notmuch and no store picker. AI is deliberately *not* in v1
(PRODUCT.md §23) despite being a founding principle; it is epic E12 (now
tracked
as GitHub issues under the [Postio Roadmap](https://github.com/users/dlapiduz/projects/2)
project). Approved plan: `~/.claude/plans/ethereal-fluttering-kettle.md`.

**Design source of truth.** `Design/Mail Client.dc.html` is a Claude Design
canvas whose PLATE direction (option 1b — airy desktop, 40px rows, key hints
on the focused row only) was chosen. It settled several questions an earlier
brief had answered differently, and **PRODUCT.md now records the resolution**
rather than the argument: keys are `e`=reply, `a`=archive, `A`=archive-thread,
`u`=undo, `t`=thread; compose takes over the reading pane rather than opening a
window; the sidebar says "Flagged" not "Starred". The canvas remains the
authority on *visual* detail, and that target is the Industry design system's
*identity* (Barlow / Barlow Condensed /
IBM Plex Mono, steel accent `#5980a6`, hairlines) *without* its wireframe
chrome (no blueprint corner marks, no transparent line-drawing cards), keeping
real Adwaita window chrome. The canvas path `~/.config/postmark/` is an
earlier project name — use `postio`.

**Hard constraints from the user.** (1) TDD is mandatory — failing test
first, then implementation. (2) The app must feel instant — transitions
`<=100ms` or absent, pane switches and thread drill-in use *no* transition,
and the PRODUCT.md §18 budgets (`<500ms` start, `<16ms` interaction, `<100ms`
search) are enforced by criterion benches that fail CI, not checked by hand
at the end.

**Privacy stance.** "Nothing leaves this machine that the user did not ask
for" is a stated principle in CLAUDE.md and an invariant in `/ux-architect`.
Remote image blocking and the hardened reader are done. Two vectors that were
open until audited: read receipts (`Disposition-Notification-To` must never
be auto-sent) and List-Unsubscribe One-Click (the POST confirms the address is
live to a spammer, so it must be deliberately user-initiated). The audit is
proved with a local request-logging server against the
`html-tracking-pixel-remote-images` corpus fixture, not asserted.

**Surfaces policy** (audited 2026-08-23). The app has exactly **one** modal
(`adw::AlertDialog` in `composer.rs`, the discard-draft confirmation). The
palette, cheat sheet and settings panel are all `add_overlay` on the main
window, not dialogs — `AccessibleRole::Dialog` on the panel and cheat sheet is
a screen-reader role, not a window. The composer takes over the reading pane
per canvas 2a; the list keeps its scroll and selection. Policy: in-place
overlay is the default, a modal must be justified in the issue that adds one,
detached windows are opt-in only. The discard modal could become an undo
toast since drafts are in SQLite — deliberately left alone for now at the
user's call.

**There is exactly one way to express "which messages".** A search is a
query; a saved search is a named query; a virtual folder in the sidebar is a
pinned saved search; a filter/rule is a saved search plus actions evaluated on
arrival. `crates/postio-config/src/filters.rs` already implements the schema
and names it this way (`[filters]` — named saved queries, with `pinned = show
in sidebar`) — but no runtime reads `FilterConfig` yet, so the sidebar doesn't
render pinned filters and there's no rules engine (tracked as GitHub issue
for the filters/rules engine). The boundary that keeps this honest: parsing
lives in `postio-search` (pure, no SQL/toolkit), `postio-config` keeps queries
as TEXT and never parses, `postio-index` executes a parsed query against
FTS5. **Do not invent a second matching language for rules** — one parser,
one syntax to learn, and dry-run comes free by running the query. What this
does *not* say: a real IMAP mailbox is not a saved search. It has
UIDVALIDITY, server state, a `MailboxRole`, and mail physically lives in it;
`a` archives *into* one. The sidebar shows two lookalike things that behave
differently — mailboxes mail moves between, and virtual folders that are
queries re-run on open. Collapsing that breaks move, archive and sync. Full
write-up: `docs/ARCHITECTURE.md` section 6.

**Selection and cursor are never the same thing.** The message list has two
states. *Cursor* — where the keyboard is. `GtkSingleSelection` is the cursor,
not the selection; the name is GTK's, the meaning is ours. Moved by `j`/`k`/
click. Drawn as canvas 1b draws it: accent tint ground, 3px steel left edge,
key hints on it alone. *Selection* — what an action will hit.
`postio_core::state::Selection` (`These(ids) | Everything{except}`) — never a
`Vec` for select-all, because the list is windowed over paged SQLite. Built
deliberately: `x`, Ctrl-click, Shift-click, a click on the row's check square,
Ctrl+A. Drawn as a steel check replacing the avatar chip, on
`--postio-selected-strong-bg`. The check is what carries "selected", not the
ground: the two grounds are one step apart in light and the *same colour* in
dark (canvas 3c), so a ground-only distinction is invisible in dark. A glyph
reads at a glance and survives high contrast. **A plain click clears the
selection and only moves the cursor** — it does not select the row it lands
on. Two consequences: reading mail one message at a time would otherwise put
a bulk bar over the list on every click, and pressing `x` on the row you just
clicked would take it *out* of a selection you never made. Therefore: an
action with an empty selection must act on the *cursor* row —
`postio-core` keeps `focus` beside `selected` for exactly this, and
`AppState::focus_on` already says the selection follows only when the user
asks. Whoever resolves `MessageTarget::Selection` must fall back to focus
when the selection is empty — without that fallback, `a` after a plain click
archives nothing. The bulk bar is in the list header, replacing the unread
count and the sort while it is up: three verbs (archive/delete/move), each
carrying its key hint, each dispatching the registry's `CommandId`. Everything
else stays in the palette.

**The composer takes over the reading pane only.** (canvas 2a) The list keeps
its scroll and selection, and `gtk_composer.rs` asserts it. One reading pane
means one composition: opening compose while a started draft is retained
reopens that draft rather than replacing it, and the status line says so.
`Esc` **never** discards — it closes and keeps the draft, and
`composer::closing()` is the unit-tested rule for whether there is anything
to keep (recipients, a subject, or body text above the signature; the
signature the composer inserted does not count). Discard is `Ctrl+D` only, it
confirms first, and it is deliberately *not* a button beside Send. The shell
wears a `composing` CSS class while it's open, which dims the sidebar and
list per the canvas — exempted under `.postio-hc`, because high contrast
exists to raise contrast.

**Keymap override precedence.** An explicit `[keys]` entry outranks a
built-in default that wants the same key: the override takes it, the
default's command goes palette-only and says so in `Keymap::problems()`.
Between two explicit overrides, registry order decides. This reverses the
original rule (registry order always wins, override dropped) — changed when
`x` became `toggle_selection`'s default and silently broke `archive = "x"`.
Do not flip it back without reading
`crates/postio-core/tests/config.rs::an_override_takes_a_key_from_the_default_that_had_it`,
which carries the reasoning.

## Post-v1 ideas captured (mostly now tracked as GitHub issues)

These were captured from conversations with the user before the migration to
GitHub Issues. Cross-referenced below where a GitHub issue already exists;
kept here anyway for the reasoning, which didn't all make it into the issue
bodies.

- **Filters/rules engine** — should reuse the search query parser as its
  condition language rather than inventing a second matching syntax. Now
  tracked as issue #5 (part of epic #19, Triage & Filters).
- **MCP support** — direction (server vs. client vs. both) is an *open
  decision*, and prompt injection via attacker-controlled email bodies is the
  dominant security constraint: no MCP tool may send/delete/move without
  explicit human confirmation. Now tracked as issue #14 (epic #22,
  Integration).
- **Richer signatures** — basic per-identity signatures were already in v1
  scope; this covers multiple named signatures, HTML/plaintext variants, and
  placement control. Now tracked as issue #12 (epic #17, Compose).
- **Unified palette** — the user wants VS Code style: one keypress, one box,
  fuzzy matching, with `>` prefix for commands and plain text for mail
  search. This refactored two previously-separate overlays into one. Already
  shipped; no open issue.
- **Smart labels** — deferred to the AI work. Design note: use cheap header
  signals (`List-Unsubscribe`, `Precedence`, `Auto-Submitted`) before
  reaching for a model, and categories must be visible and correctable. Now
  tracked as issue #8 (epic #19, Triage & Filters).
- **Multi-select / bulk actions** — the key design constraint is that
  selection cannot be a `Vec<MessageId>` for "select all" — the list is
  windowed over paged SQLite and must never materialise a mailbox, so model
  selection as an id set *or* a predicate (query + exclusions) and resolve it
  in one SQL statement. Bulk archive of 50k must be one update plus one
  queued operation. Also: selected and focused are distinct states (see
  above) — conflating them is the usual bug. Already shipped; no open issue.

## Architecture reference

Postio's architecture and the reasoning behind it live in
`docs/ARCHITECTURE.md` (the decisions, each with why it's load-bearing),
`docs/decisions/` (long-form ADRs, e.g. `0001-imap-library.md`), and
`docs/architecture-review-2026-08.md` (standing critique + known gaps). The
crate diagram in `README.md` is mermaid and the one in `CLAUDE.md` is ASCII —
if you update one, update the other; both were previously wrong in the same
way (`postio-search` drawn as a child of `postio-gtk`, `postio-index`
omitted entirely). `postio-search` is a pure *shared* leaf (query language, no
SQL, no toolkit) depended on by `postio-gtk`, `postio-index`, `postio-runtime`
and `postio-app`; `postio-index` owns `rusqlite` and the FTS5 executor.

## GTK & UI gotchas

**`GtkListView` read-ahead is ~205 rows, not a screenful.** Measured against
GTK 4.22.4: 50 items → 50 rows, 200 → 200, 1000 → 205, 5000 → 205. This is why
a 200-item test model looks exactly like "recycling is broken" — the model is
smaller than the window, so "one row per item" and "the whole window" are the
same number. Never diagnose recycling with a model near 200; use 1000+ or the
result is meaningless. The window is filled *synchronously* inside
`ListStore::splice`, not during idle, so the cost lands on the frame that
populates the list. Corollary: the cost that matters is per row *widget*, not
per item — a 4-label `GtkBox` row cost 18.3ms to fill a window against 6.8ms
for a single custom `snapshot()` row. `crates/postio-gtk/tests/gtk_list_recycling.rs`
is the harness.

**GTK integration tests catch real keystrokes and real focus changes.**
`postio-gtk` integration tests call `window.present()` on the developer's
live Wayland session, so the environment leaks in two ways: (1) real
keystrokes typed while they run land in the test's widgets — seen as
`"eGrace Hopper"` in a recipient chip and `"after:aug1n"` in a rendered query;
(2) running the suite in parallel puts several real windows up at once and
focus moves between them — seen as a `RefCell` double-borrow panic in
`finder.rs::refresh` that doesn't reproduce when the test runs alone. Both
pass on re-run. If a GTK test fails with an unexpected character, or a borrow
panic in a module you didn't touch, re-run before investigating. Under
`xvfb-run` (or `scripts/test-headless.sh`, see CLAUDE.md) neither can happen.

**GTK theme startup order matters.** `adw::init()` → `postio_gtk::fonts::install()`
(must run *before* the first widget: a `PangoContext` caches the family it
resolved) → `postio_gtk::style::install_for_application(&app)`, which loads
the generated `tokens.css` from GResource and tags every window with
`.postio-dark`/`.postio-hc`. GTK 4.22 does *not* honour
`@media (prefers-color-scheme/prefers-contrast)` in an application-priority
CSS provider (only in the theme provider, loaded with an explicit variant),
and libadwaita puts no dark class on the tree — that's why the dark/
high-contrast blocks in `data/tokens.css` are keyed off `:root` classes.
Overriding libadwaita's CSS variables (`--window-bg-color` etc.) at
`:root.postio-dark` does repaint stock widgets; overriding `@define-color`
does not scope per-class.

**One test function per GTK integration-test binary — load-bearing, not
style.** GTK initialises once and libtest runs a binary's tests on separate
threads, so a second test in the same file takes the no-display skip branch —
*silently*. It looks exactly like a pass. Seen once: a sidebar-height
regression test added to an existing file passed against deliberately unfixed
code because it never executed; moved to its own binary it failed correctly.
If a new GTK test passes on the first try against code you haven't fixed yet,
check it isn't sharing a binary.

**`postio-gtk` examples/tests cannot read a store.**
`scripts/check-crate-boundaries.py` counts a crate's *own* dev-dependencies,
and an example is built from that graph — so `postio-gtk` cannot have an
example (or test) that touches `postio-storage`, because `rusqlite` would
land in the view layer's graph and fail CI. This is why the render-to-PNG
tool lives at `crates/postio-app/examples/shot.rs`
(`cargo run -p postio-app --example shot`) rather than in `postio-gtk` — its
demo mode reads a seeded store. The complement is
`crates/postio-gtk/examples/surface.rs`: surfaces reached by a keystroke
rather than by data, built from `postio-gtk`'s own types with no database at
all.

**`AppState` is pushed into, never pulled from.**
`postio_core::state::AppState` does *not* observe `postio-gtk`;
`crates/postio-app/src/commands.rs::mirror` pushes the window's mailbox,
selection and cursor into it in the instant *before* a command is sent
(`Window::connect_action`), and nothing else writes it. Two reasons: the
selection genuinely lives in the list widget (it's what the user built with
`x`, Ctrl-click, Ctrl+A) so a pull can't be one gesture out of date, and a
signal-driven push would have to fire on every `j` — the interaction that
happens most and has the tightest budget. The mirror maps
`Selection::Everything` by calling `select_all()` then `toggle_selection()`
per exception, so the predicate is never resolved into the ids it stands for.
It emits into a sink whose reader was dropped on purpose (the "quiet sink"):
the window is where those `SelectionChanged` events came from. If you add
state the handlers resolve against, mirror it here, not with a signal.

**The window delivers one invocation, not two paths.** `postio-gtk`'s
`Window` has two seams out and they are two *views* of one invocation, not
two paths a command can take. `connect_command` carries a `CommandId` (the
composer, the config editor — consumers that need only the verb);
`connect_action` carries a whole `postio_core::Command` (the command bus,
which needs to know what the verb was aimed at). `Window::run` (keyboard,
palette, the undo toast's `win.undo` action) and `Window::act` (mouse: hover
actions, context menu, drops) *both* ask `handled_here()` first — the
window's own commands, closing overlays and moving the cursor, stop there —
and then call `deliver()` exactly once, which feeds both seams. Subscribing
to both would see every gesture twice. Before this was fixed, the mouse path
fired both with *different* invocations (an id defaulting to "the
selection", then the Command naming the hovered row), so one click on a
row's archive button archived the selection *and* that row; the keyboard
never reached `connect_action` at all. Pinned by
`crates/postio-gtk/tests/gtk_dispatch.rs`. Do not add a third way out.

## Storage, sync & search internals

**Mailbox counts are maintained by triggers, not by call sites.**
`mailboxes.total_count`/`unread_count`/`flagged_count` are maintained by
SQLite triggers on `messages` (migration `0003_mailbox_counts.sql`), not by
any Rust call site. Do **not** add `MailboxRepository::recount` calls to new
write paths — the invariant is the table's. `recount`/`recount_account` are
the repair path only (the migration's own one-time backfill). Why this
exists: the column was derived data with no owner — `recount` had two callers
in the whole workspace (`send.rs` for the Sent box, `seed.rs`). The message
list's total comes from that column, the total is the `GListModel`'s
`n_items`, and a `GtkListView` over a model of length zero asks for *no*
pages. So a count that's wrong-low renders an *empty mailbox*, not a wrong
number — on a live account with 81,716 messages, every folder drew nothing
while the page read handed the list 50 real rows and `total=0` in the same
line. It survived every test and every screenshot because everything except
a live account goes through `postio_storage::seed`, and seed recounts.
Diagnosing this class of thing: `POSTIO_LOG=postio_app=debug` and read
`postio::feed: message page read mailbox= page= offset= rows= total=`.
`rows>0` with `total=0` is the signature — it tells "the model never asked"
apart from "the store answered empty". `count` no longer trusts a cached
zero, so any future drift degrades to slow rather than to invisible.

**Storage schema conventions** (migration 0001). Timestamps are `INTEGER`
Unix milliseconds UTC; booleans `INTEGER` 0/1; enums are the model's
`as_str()` snake_case with `CHECK` constraints; ids are
`INTEGER PRIMARY KEY AUTOINCREMENT` (no rowid reuse, the operation queue
depends on it); no `BLOB` columns anywhere — bodies/raw/attachment bytes are
blob-store keys (`messages.raw_blob_id`/`body_text_blob_id`/
`body_html_blob_id`/`headers_blob_id`, `attachments.blob_id`); mailbox
`UIDVALIDITY`/`UIDNEXT`/`HIGHESTMODSEQ` live *only* in `sync_state`, not on
`mailboxes`; recipients and attachments are polymorphic (`message_id` XOR
`draft_id`) so drafts reuse them; thread membership is `messages.thread_id`,
not a duplicated id list; draft bodies *are* inline TEXT (live editor buffer,
not content-addressed). Add schema changes as a new numbered migration —
editing an applied one is rejected by checksum.

**Search query parser contract.** `parse(input, today: NaiveDate) ->
ParsedQuery` is pure and total (no `Result`). Contract for the executor and
chip UI: tokens are a flat ordered `Vec<Token>` with byte `Span` + raw source
text; `TokenKind` is `Filter(Clause{negated,filter}) | Partial{field,value} |
Text(TextTerm{negated,value})`. Partials are half-typed operators and *must*
constrain nothing. Date semantics: `after:` is inclusive (`>=` start of day),
`before:` is exclusive (`<` start of day); relative dates resolve against the
caller-supplied `today`. Sizes are binary (`K` = 1024). `fts_match()` quotes
every term as an FTS5 string literal and returns `None` when there's no
positive free text (FTS5 has no unary NOT), so negative-only text must be
excluded by the executor via `text_terms()`.

**Config live-reload seam.** The watcher thread produces `validate::Checked`
(parse+validate off the UI thread); the UI thread calls
`LiveConfig::apply(checked) -> Reload {Applied, Unchanged, Rejected}`.
`Applied` is the only moment worth diffing for `ConfigChanged`, and
`LiveConfig` keeps the last-good `Config` when a file is `Rejected`.
Validation errors carry line/column via `src/source.rs` (toml `DeTable`
spans) and `Validation::status_line()` renders `"valid · parsed in 2 ms"`.

**`Pool::get()` is a blocking condvar wait.**
`postio_storage::db::Pool::get()` blocks the calling OS thread on a
`std::sync::Condvar` when the pool is exhausted — it is not async-aware. The
sync engine (`postio_runtime::engine::run`) deliberately runs on a
single-thread tokio runtime with no other OS thread to make progress while
blocked. Work that checks out more than one connection concurrently from
tasks running on that thread must acquire every connection it needs
*sequentially* before starting concurrent work, and must never call
`pool.get()` from inside concurrent work once it has started — otherwise two
tasks can both block on the same condvar with nothing left on that thread able
to run and release one: a genuine self-deadlock, not ordinary contention.
`DEFAULT_MAX_CONNECTIONS` is 4, shared with UI-thread reads, so headroom is
thin. `engine::sync_wave` is the one place that does this and is written to
that rule (#32): it pops its mailboxes and takes all of its connections in a
plain `for` loop, and only then builds the `FuturesUnordered`.

**Concurrent mailbox sync: what bounds it, and what it must not break** (#32).
`engine::sync_wave` runs `sync_lanes(pool)` mailbox passes at once — the
database pool's size less two reserved connections (UI reads, engine
housekeeping), clamped to `MAX_SYNC_LANES = 3`. With the default pool of four
that is **two**. Three constraints set those numbers, and none of them is
arbitrary:
- *The database pool is the scarcer one.* A pass holds its SQLite connection
  for the whole pass, and the UI thread reads through the same pool. Take
  them all and the message list stops answering during a first sync.
- *More lanes than IMAP connections is slower, not faster.* The IMAP pool
  defaults to four with one lane held by `IDLE`, and a pass that does not get
  a connection of its own shares one — paying a `SELECT` per batch, because
  `postio_imap::imap::selection` caches the selection *per connection*.
- *Passes are concurrent, never parallel.* The engine's runtime is
  current-thread and the futures are polled by one `FuturesUnordered` on one
  task, so two passes cannot both be inside `initial::enumerate`'s batch
  transaction at once — there is no await between `unchecked_transaction()`
  and `commit()`. Do not add one. Those transactions are `BEGIN DEFERRED` and
  read before they write, so genuinely simultaneous writers would meet
  `SQLITE_BUSY_SNAPSHOT`, which the busy handler does *not* cover.

Two things had to change to survive concurrency, and would have to change
again for anything else that overlaps passes. `StatusTracker` keeps the set of
passes in flight and reports the foremost (earliest-started, i.e. highest
`order::sync_priority`), because one pass finishing is no longer the account
going idle. And a wave cancels itself the moment a job arrives — the shared
`CancelToken` — with the interrupted mailbox pushed back on the front of
`to_sync`; a first sync is minutes long and the user must not queue behind it.
An interrupted pass keeps everything it committed and resumes, which is
`initial`'s resumability doing its job.

**io-imap binding rules** (full ADR: `docs/decisions/0001-imap-library.md`).
1. Pin `io-imap = "=0.6.0"`, `default-features = false` + `"client"`; never
   depend on `imap-codec`/`imap-types` directly, take them from
   `io_imap::codec` / `io_imap::types`.
2. Capabilities come *only* from `session::ImapSessionOpen` /
   `ImapClientStd::connect` — `ImapLoginOptions::ensure_capabilities`
   defaults to `false` and a hand-built auth coroutine can return an empty
   capability vec with no error. An empty post-auth capability list is an
   **error**, never a silent downgrade.
3. Gate QRESYNC on the post-auth `CAPABILITY` list, **never** on the untagged
   `* ENABLED` echo — iCloud omits it.
4. Take expunges from `SELECT`/`EXAMINE` (QRESYNC) `.vanished_earlier`,
   **never** from `FETCH (VANISHED)` — io-imap discards the latter.
5. Do not use `watch::ImapMailboxWatch` (holds a whole-mailbox UID+flag shadow
   in memory); build ENABLE/SELECT/IDLE/SELECT(QRESYNC) from primitives,
   using `watch.rs` as reference only.
6. Log `io_imap` at debug in dev — `send.rs` silently skips undecodable
   untagged responses.
7. No `io-imap` type crosses the `MailBackend` boundary.
8. We are tokio: implement `ImapClientAsync::run` ourselves (~40 lines,
   `examples/tokio_session.rs`).

**A stale-selection bug that reads flags/bodies/deletes onto the wrong
message.** `ImapSession::ensure_selected` caches the selected mailbox *and*
its `UIDVALIDITY` for the life of a pooled connection, so a server-side
renumber is invisible while the session stays on that mailbox. Every
`FetchedMessage` carries the generation from the first `SELECT`, so the
mandatory rebuild never fires and new-generation UIDs are read as old ones —
flags, bodies and deletes land on the wrong messages with no error. Raised to
P0 when found. Related: no per-command deadline means a stalled server
permanently consumes one of a bounded pool's connections.

**Fixtures must not answer for the wiring.** A fixture must never supply by
hand what the application is supposed to produce. Two lines once hid eight
shipped bugs between them: (1) `postio_storage::seed` called
`MailboxRepository::recount_account` after inserting, so every seeded store
had correct cached mailbox counts and a live one had zeros — the message list
drew rows from every fixture in the project and nothing from a real account
with 81,716 messages, and no test could tell; (2) `MockBackend::new()`
invented an INBOX, so no test ever had to say where folders come from, and
nobody noticed `MailBackend::list_mailboxes` had no production caller for the
life of the project. Both are now removed: counts come from migration 0003's
triggers (the same path a real sync uses) and `MockBackend::new()` has no
folders. `crates/postio-storage/tests/seed_is_honest.rs` guards it, and its
failure message says not to repair it by recounting in the fixture — that's
the exact move that hid this. **If a test goes red after touching a fixture,
the failure *is* the bug; do not restore the fixture's shortcut to make it
pass.**

**A received attachment's bytes are not in `Attachment::blob_id`.** That
column is only ever filled on the way *out* — `postio_app::compose` puts a
file the user attached into the blob store and records its key. Nothing in
the receive path writes it, so for every message that arrived from a server
it is `None`, and `parts::Node::downloaded` is correspondingly always false.
What the backfill actually stores is the whole raw message under
`Message::raw_blob_id` (`postio-sync/src/backfill.rs`), so a received part is
extracted from that with `mime::parse` and matched by its MIME path
(`Attachment::part_id`, e.g. `2.1`). `postio_app::reading::part_bytes` is the
worked example. Anything written against `node.downloaded` or
`attachment.blob_id` for incoming mail is reading a field that will never be
set — including inline `cid:` resolution, which is why an inline image that
"should obviously work" may quietly never render.

**`Engine::request_body` queues; it does not fetch.** `Ok(true)` means "there
was something to fetch", not "here it is" — the message goes to the front of
the backfill and the bytes land when the engine's own loop claims the job. A
caller that reads the store on the next line gets nothing, and gets it
*intermittently*, because whether the loop has run yet depends on timing.
Wait for the result: poll for the thing you actually need with a deadline
(`postio_app::reading::wait_for_body`), or watch `backfill_progress` the way
`postio-runtime/tests/engine.rs` does. While waiting, treat a failed read as
"look again" rather than an error — the writer you are waiting for holds the
table, so `SQLITE_LOCKED` there is a sign of progress, not of failure.

**A body fetch replaces the message's attachment rows.** The parser re-reads
the structure and `MessageRepository::update` writes the new set, so an
`AttachmentId` does not survive the fetch it triggered — the row it named is
gone and a new one with a different id describes the same part. The stable
key across a fetch is the MIME path. Resolve an id to a `part_id` *before*
asking for bytes, while the id still means something. Discovered wiring the
parts panel (postio-v62): the save worked for parts already downloaded and
failed only for the ones that had to be fetched, which is the half nobody
tests by hand.


**An account is a row *and* a credential — never one of the two.** Onboarding
writes both, and 0.1.0 wrote the row first. When the keyring write then failed
(a locked keyring, no Secret Service, a D-Bus timeout) the row stayed behind,
and startup routed on `first_account(..).is_some()` — one row was enough. So
the next launch opened an account that could not authenticate, could not sync,
and could not be repaired from inside the application: onboarding is the only
thing in Postio that writes a credential, and it never ran again. Recovering
meant deleting rows from SQLite by hand (issue #67). Two rules came out of it,
and both hold for anything that ever writes an account:

- **The credential goes in first, the row second.** The failure that order
  leaves behind is a secret with no account, which nothing reads;
  `onboarding::persist` rolls it back anyway. The other order strands the
  account, and the account is the half that is fatal.
- **"Has an account" means `postio_app::startup_route` said so** — a row whose
  password the secret store will actually give up. Not a row. That definition
  also covers the credential being deleted or the keyring reset later, which
  no care at write time can prevent, and it is why the onboarding screen is
  reachable a second time (`Status::Reauthenticate`, prefilled from the row).

The check is asynchronous for the reason every keyring call in this codebase
is: `KeyringSecretStore` is a tokio future bounded by a 10s timeout, so it is
spawned on the engine runtime and answered on the glib main context. Reading
it inline would have swapped a wrong guess for a startup that stalls behind a
locked keyring. `postio_app::open_or_onboard` is that decision in a function
rather than in the `activate` closure, so a test can drive it over a real
`Window` — `crates/postio-app/tests/startup_repair.rs`, which fails against
the 0.1.0 routing.


## Testing infrastructure

**`postio_runtime::Engine` does not need a trait in front of it to be
tested.** A proposal to add one was closed as not-needed after being written
on a wrong premise. `Engine::spawn` takes
`EngineParts { backend: Arc<dyn MailBackend>, .. }` — it never constructs a
transport, it is handed one. `postio_imap::backend::MockBackend` is a
complete in-memory `MailBackend` including bodies. So a real `Engine` over a
mock does full syncs, backfills and body fetches with no network and no
display, in the default suite. Proof already in the tree:
`postio-runtime/tests/engine.rs::a_seeded_body_is_actually_fetched`, and
`postio-app/src/refresh.rs`'s own tests. `MailBackend` is the seam, and
CLAUDE.md already names it as the boundary — adding a second trait over
`Engine` would be a duplicate seam kept in step by hand. If you want an
engine call from `postio-app` under test, build the `Engine` with a
`MockBackend` (`refresh.rs` is the nine-line template) rather than
abstracting `Engine`.

**A shared-cache in-memory SQLite database races with a running engine.**
Writing to a `test_support::memory()` database from the test thread while an
`Engine` is running against the same database fails with `SQLITE_LOCKED`
(extended 262, "database table is locked") rather than waiting —
shared-cache in-memory SQLite takes table locks that `busy_timeout` doesn't
cover. A file-backed production database in WAL mode *does* wait, so this is
a test-harness shape only. Do all account/identity/mailbox setup **before**
`Engine::spawn`; the engine writes on link-up (folder discovery) and again on
every drain, so the collision window isn't small. Symptom: an intermittent
failure in an unrelated assertion, roughly 1 run in 4.

**`postio-app` has a lib target — the composition root is testable.** New
modules go in `src/lib.rs` as `pub mod`, not in `main.rs` — `main.rs` is
three lines over `postio_app::run()`. Integration tests live in
`crates/postio-app/tests/` and this is the only place the wiring itself can
be asserted; four of eight shipped wiring bugs lived here precisely because a
bin-only crate can't be linked by `tests/`. The harness shape that matters:
start from the composition root (`feed_the_window`, the same function `run`
calls), never the widget; assert the pane *has* content, never that it
renders content it was given — that's the only assertion that can fail when
the wiring is missing; assert as far from the trigger as possible
(`keystroke.rs` asserts in SQLite, not in the widget); use
`settle_until(|| cond)` with a deadline pumping
`glib::MainContext::default().iteration(false)`, because page reads cross to
the runtime and answer over a channel; one `#[test]` per binary since GTK
initialises once (see the GTK section above). `feed_the_window` reads the
local store only — `start_syncing` is the half that dials a server, split out
so a wiring test never opens a socket.

**Test infrastructure gaps** (audited state, may now be partly closed —
check before assuming). `MockBackend` mocks at the `MailBackend` trait (skips
`io-imap` entirely) and `ImapScript` replays a fixed transcript (cannot
answer unscripted sequences). Neither exercises the wire. A planned
in-process IMAP server on loopback tests the real client stack including
`io-imap`, with fault injection for known iCloud quirks (capabilities hidden
until after login, missing `* ENABLED` echo, malformed FETCH sequence numbers
under QRESYNC). A corpus-seeded SQLite store lets GTK tests, benches and
`examples/shot.rs` render real mail instead of hard-coded demo content.

**Test corpus.** 38 `.eml` fixtures live in
`crates/postio-model/tests/corpus/` with a README describing each. Load them
from *any* crate's tests via dev-dependency `postio-model` with
`features = ["test-corpus"]` (off by default), then
`postio_model::test_corpus::load("name")` / `by_category(Category::X)` /
`all()`. The loader hands out raw bytes, not parsed messages, on purpose.
Adding a fixture = drop the `.eml` in, add a line to the `corpus!` table in
`src/test_corpus.rs`, add a row to the corpus README —
`tests/corpus_loader.rs` fails if any of the three is missed. Extend this
corpus, never start a second one.

**Wall-clock perf assertions in tests are worthless under load.** Do not
assert wall-clock budgets in `postio-gtk` tests. This box runs several
concurrent build/test sessions; measured at load average 18 on 8 cores, the
same thread drill-in measured 14ms, 23ms, 63ms, 89ms and 180ms across runs of
identical code. Best-of-N filters most of it but is still a ceiling, not a
number. Perf budgets belong in benches (`postio-core/benches/perf_budgets.rs`),
which already notes this about shared runners. Check `uptime` before
believing any timing measured interactively.

**`VmRSS` alone is misleading for measuring Postio's memory use.** Measured:
total resident set is 131 MiB on a 1,000-message store and 215 MiB on a
100,000-message one, which reads exactly like the mailbox being loaded — the
one thing PRODUCT.md §18 promises never happens. Split
`/proc/<pid>/status` instead: `RssAnon` is 47 MiB at *both* sizes (what
Postio itself allocates — the windowed list model, the widgets, the
runtime), and the entire difference is `RssFile`, because `postio-storage`
sets `PRAGMA mmap_size = 256 MiB` and SQLite maps as much of the store as it
touches. Those are reclaimable page-cache pages, not mail being held. Anyone
re-measuring must split anon from file, or raise `mmap_size` as a suspect
before the list model. Reproduce with
`crates/postio-runtime/examples/seed_store.rs` and the release binary; the
README carries the table.

**Two worktrees sharing one `CARGO_TARGET_DIR` can hand you another
worktree's library.** Observed 2026-08-24 while working issue #33. This
session had added `crates/postio-core/src/invocation.rs` and a new `Event`
variant; `cargo test -p postio-core` was green, `cargo build --workspace` was
green, and `cargo test -p postio-app` then failed to compile with *"could not
find `invocation` in `postio_core`"* — against source that plainly contained
it. `cargo build -p postio-core` immediately beforehand did not help.

The link line named the culprit: `postio-app` was compiled with
`--extern postio_core=.../libpostio_core-d8f157….rlib`, while the core built
seconds earlier was `libpostio_core-00a841….rlib`. The stale one's dep-info
(`target/debug/deps/postio_core-d8f157….d`) lists its sources **relative to
the workspace root** — `crates/postio-core/src/lib.rs` — and did not mention
`invocation.rs` at all. Only this worktree had that file, so that unit was
built from a *different* worktree of the same workspace and cargo considered
it fresh for this one. Both worktrees present cargo with the same relative
paths and the same package name and version, so they land in the same build
slot and overwrite each other.

Consequences, in rough order of how badly they bite:

- **A green suite can be a lie in either direction.** Your crate can be tested
  against somebody else's version of its dependency. The compile error above
  is the lucky case, because it is loud; the silent case is a test that passes
  against a library your change never reached.
- It is a *race*, so it is intermittent. Re-running often "fixes" it, which is
  the worst possible property — it trains you to re-run instead of to look.
- `scripts/issue-land.sh` shares the target directory by default
  (`export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$MAIN_CHECKOUT/target}"`), so
  the landing gates are exposed to it too.

What to do, until #76 settles it properly:

- Within a **single** `cargo` invocation you are safe — cargo holds the build
  lock for the whole run. So verify across crates in one command
  (`cargo test -p postio-core -p postio-app`) rather than one per crate.
- When a result surprises you, **check the link line before you re-run**:
  `cargo test -p <crate> -v 2>&1 | grep -o "extern <dep>=[^ ]*"`, then
  `head -1 target/debug/deps/<dep>-<hash>.d` and look at whether the source
  list matches the tree you are actually in.
- For a result you are going to stake a merge on, build into a target
  directory of your own: `CARGO_TARGET_DIR=$PWD/target-verify cargo test …`.
  It costs one full build of the third-party crates and nothing after that.

CLAUDE.md and the `/issue` skill both recommend
`export CARGO_TARGET_DIR=~/src/postio/target` to keep the GTK and WebKit
builds warm. That advice is still right about the cost it is avoiding — it
just is not free, and this is the bill.

## Logging & privacy

**Logger installation order.** `log::set_logger` succeeds *once* per
process. `postio-imap`'s `skip_counter` watches io-imap's
`debug!("skipping undecodable untagged response")` and turns it into
`BackendError::ResyncIntegrityLost` — an integrity check, not a log line.
`tracing-subscriber`'s `SubscriberInitExt::init()`/`try_init()` calls
`LogTracer::init()`, which *is* a `set_logger`, so calling `.init()` takes
that one slot and leaves the counter inert: a `CHANGEDSINCE` fetch that
silently dropped deltas is then reported as a complete incremental pull.
**Never use `.init()` on the subscriber in `postio-app`.** Use
`tracing::subscriber::set_global_default()`, and install the bridge *first*
via `postio_imap::imap::install_skip_counter_forwarding_to(Some(Box::new(LogTracer::new())))`,
which composes the counter and the bridge into the one logger the process is
allowed. `skip_counter_is_counting()` reports whether it worked, and
`logging.rs` warns at startup when it didn't. The warning caught this exact
bug in the first live run.

**Logging levels and what may be logged.** (1) *Scope*: a bare `POSTIO_LOG`
level is expanded by `postio-app/src/logging.rs::scope()` into
`"warn,postio_*=<level>,io_imap=<level>"` — applied literally, so
`POSTIO_LOG=debug` means rustls enumerating 146 CA certificates before the
first line about mail. A directive containing `=` or `,` passes through
untouched so `"rustls=trace"` still works. (2) *Privacy*: never log bodies,
subjects, recipient addresses, passwords, file contents, or search query text
(what someone searches their own mail for is as revealing as the mail
itself). Do log ids, counts, durations, outcomes, mailbox paths, capability
names and server endpoints — a folder is a container the user named, a
capability list is the server's public advertisement. (3) An error string
that may name an account goes through `postio_model::address::redact_addresses`
at the *log call site*, not at the source: `SecretError` names the account so
the user can see which one to fix, and that belongs on screen; a log gets
pasted into issues, so the domain survives and the local part doesn't. (4)
Enforced by `crates/postio-runtime/tests/logging_privacy.rs`, which drives
the sync path at TRACE and greps for the seeded store's own
subjects/previews/senders read out of the database. It uses
`set_global_default`, not `set_default` — the engine works on its own
thread, and a thread-local subscriber would make the test pass while
observing an empty buffer.

## Working in a shared git tree

These matter regardless of whether work is tracked in beads or GitHub Issues
— they're about several sessions sharing one working tree and one git index,
not about the tracker.

**`cargo fmt -p <crate>` reformats every file in that crate**, including
other sessions' uncommitted work in the same crate — it's not just
`cargo fmt --all` that's dangerous. `postio-gtk` is the crate where this
bites most, since several sessions work it at once. It reformats rather than
destroys, so the damage is noise in someone else's diff, not lost work.
Before running it, `git status` the crate; if another session has files open
there, expect to hand them whitespace churn, and say so.

**`git commit --only <path>` silently skips untracked files** under that
path — it commits tracked modifications only, so a commit that adds a new
module can land referencing a file that isn't in the tree, exiting 0 and
saying nothing. Before committing, check `git status --short` for `??`
lines under your paths; if there are any, `git add` those exact paths first,
then `git commit --only <paths>` as usual. Never `git add -A` — the tree is
shared.

**Git history was rewritten in place once, before any remote existed**, with
`git filter-repo --replace-text` to scrub personal addresses from every
commit. Every commit SHA changed as a result. Old notes citing pre-rewrite
SHAs no longer resolve. `git-filter-repo` is not packaged by default —
install with `pip install --user`. Deliberately *not* rewritten: `LICENSE`
(copyright holder), `Design/*.dc.html` (rewriting it would churn the design
canvas through history), and provider hostnames in old fixture blobs
(published server names, not personal data). This is history, not a
recurring risk — but it explains why very old references to commit SHAs may
not resolve.

**A plain `git reset` (no `--hard`) on a shared branch can silently drop
another session's already-landed commit** from history. The guard hook
(`.claude/hooks/guard-shared-tree.py`) blocks `git reset --hard` but not a
bare `git reset`. No data is destroyed — the dropped commit object stays
reachable via its hash/reflog — but branch history can briefly lose a
commit, and a subsequent unrelated commit's `git add` can sweep up the
orphaned working-tree changes, burying them under an unrelated message.
**Never run bare `git reset` (mixed or soft) on a shared branch either**, for
the same reason `git reset --hard` is banned — it moves the one shared HEAD
ref for everyone, not just your own view. If you need to undo your own last
commit, `git revert` it instead (adds a new commit, never rewrites the shared
ref backward). If you find this has already happened, don't attempt history
surgery on a live shared branch — verify the content and tests are intact
downstream and move on.

**The git index is shared, so `git add` does not protect your files, and the
reverse also happens.** Staging your own files does not protect them from
another session's commit — whichever session runs `git commit` next commits
*everything currently staged*, from any session. Confirmed in both
directions in one session: work-in-progress staged and about to be committed
landed inside another session's unrelated commit because they committed
first; minutes later, another session's unrelated fix was already staged by
the time this session's `git add` + `git commit` ran, and landed inside this
session's commit instead. Minimizing the add-to-commit gap does not reliably
prevent this — the race is inherent to a shared index across concurrent
sessions and cannot be fully closed from one session's side. **Do not try to
fix a swept commit with `git reset --soft HEAD~1` while other sessions are
live** — that has been tried and lost the race too: another session committed
in the window, so the reset was overtaken and its commit ended up on top of
the swept one. Rewriting shared history to tidy a commit is worse than the
untidy commit — the content is all there and nothing is lost. When you
notice it after the fact (`git show --stat` on your own commit lists paths
you didn't intend), verify the swept-in change is intact and the affected
crate still builds, note it honestly, and move on.
