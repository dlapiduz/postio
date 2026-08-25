# Engineering notes

Hard-won lessons that aren't obvious from reading the code — the kind of thing
that isn't tied to any single issue or PR. A future session (or contributor)
has no other way to trip over it before hitting the same wall, which is why it
lives here rather than in any tracker.

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

**A draft with no local buffer is another client's, and v1 does not adopt
it.** (#175) Activating a Drafts-folder row whose `\Draft` flag is set but
whose `DraftRepository::by_message` comes back `None` was left opening the
reader by #166 — there is no composer buffer to resume, so there was nothing
else to do. The gap #175 closed is narrower than it looks: the reader still
cannot edit the message, but before this it would happily render the message
as though it were an ordinary, readable one once the body backfilled, with no
signal that the row was a dead end. `load_body_or_reason`
(`crates/postio-app/src/compose.rs`) now checks `message.flags.is_draft()`
*before* it looks at `BodyState`, and reports
`postio_gtk::reader::Absent::ForeignDraft` regardless of whether the body has
downloaded — a foreign draft is never "worth waiting for" the way
`Absent::Partial` is, so it does not get a retry key either.

Adopting the row into a local `Draft` — so it becomes editable — was
considered and deliberately deferred rather than half-built. It is not one
decision but several, each with a wrong answer that looks fine until someone
hits it: autosaving an adopted draft moves it (`DraftRepository::save_and_sync`
appends a new copy and expunges the old one, same as any other save), so
picking a row up on this machine silently relocates the other client's
in-progress work; the body and any attachments may not be backfilled yet, so
adoption needs its own wait state distinct from the reader's; and two clients
editing "the same" draft afterward have no lock and no merge story. None of
those has an obvious default, which is why this stayed the cheap interim —
say so on the row — rather than becoming a v1 feature. Revisit if multi-client
drafting becomes a real workflow rather than an edge case.

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

**`EventStream` is not `Clone`, and the reason is a trap rather than a
preference.** It wraps an `async_channel::Receiver`, and that receiver is
*work-stealing*: cloning it does not duplicate the stream, it splits it. Two
handles on one channel each get some of the events and neither gets all of
them. So the obvious way to add a second consumer — clone the receiver —
produces a window that misses an unknown subset of repaints and an MCP server
that misses an unknown subset of answers, both holding state that is silently
wrong, with nothing failing anywhere. That is ADR 0005 Q10's dangerous failure
shape exactly.

Fan-out is `postio_core::bridge::EventHub` (ADR 0013, #176): one queue per
subscriber, an emit is a read lock and one `try_send` each. Two consequences
worth knowing before touching it:

- **Test both streams, never one and a count.** A fan-out bug under a
  work-stealing receiver still delivers *n* events in total, so any assertion
  that counts, or that reads a single subscriber, passes while the split is
  happening. `crates/postio-core/tests/event_hub.rs` asserts the full
  sequence on every subscriber for this reason.
- **`emit` returning `false` means nobody took it**, which on a hub includes a
  hub with no subscribers yet — not only a hub whose subscribers have all
  gone. Same for `EventSink::is_closed`.

The hub keeps **no history**: a subscriber joins at *now* and reads SQLite for
the past. A replay buffer would be a second unbounded in-memory copy of recent
mailbox activity that no consumer asked for, so if one is ever proposed, that
is the argument it has to beat.

## GTK & UI gotchas

**A change the app makes on the user's behalf does not go on the undo stack**
(#71). The dwell mark is the first of these and the rule generalises: `u` takes
back *what you did*, so anything the application decides on its own has to
apply, repaint, and stay off the stack — otherwise the verb the user actually
wants back gets buried under a drift of things they never asked for, and `u`
stops meaning anything predictable. It gets no toast either, for the same
reason at a different scale: reading a mailbox produces one dwell mark per
message rested on, and a toast each would be a banner that never clears.

`postio_session::actions::Recording` is where this lives — `Record`,
`Replay`, and now `Incidental`. The reversal for a dwell mark is `U` (mark
unread), which is already bound, in the palette and on the cheat sheet, so
nothing is unreachable; it is only not on the *stack*.

Two shapes follow from it and are worth copying:
- **The command is the same verb, not a new one.**
  `Command::MarkReadOnDwell` answers `CommandId::MarkUnread` from
  `Command::id()`, so it routes to the same handler and the registry still
  holds one "mark read". A registry entry of its own would also have needed a
  key binding it could never be reached by —
  `postio-core/tests/command_registry.rs` requires one of every entry, and
  rightly.
- **A convergent verb swallows its own rejection.** `set_flag` rejects with
  "Already set" when nothing changes, which is a correct quiet hint for `U`
  and constant noise for a dwell: the cursor resting on mail that has already
  been read is the *ordinary* case. `Actions::run` maps a `Rejected` from the
  dwell to `Ok`, and lets a `Failed` through, because a store that will not
  write is still worth hearing about.

**A dwell timer must be cancelled by anything that makes "in front of a
person" untrue**, not only by the cursor moving. `MessageListView::cancel_dwell`
is called by the window on focus loss (`is-active`) and whenever the composer
takes the reading pane (`sync_reading_pane`). Both are facts about the window
rather than the list, which is why they are pushed in rather than watched for
in the pane. Without the focus one, a machine left alone overnight comes back
with whatever the cursor happened to be on marked read.

The autoselect case was already handled before this landed: `SingleSelection`
parks the cursor on row 0 as soon as the model has rows, and `report_cursor`'s
`landed` flag keeps that from counting as a landing. That is what stops merely
launching Postio from marking the newest message read — see the comment on
`imp::MessageListView::landed`, which anticipated this issue by name.

**To assert on what a widget *draws*, wait for frames and then wait for the
pixels to stop moving.** Neither half is optional, and #90 spent two attempts
learning it.

`pump()` is not a wait — `MainContext::iteration(false)` returns immediately
when nothing is pending, so a pump loop can spin its whole budget without the
frame clock ticking once. A CSS state change (focus, hover, a class added)
reaches the pixels only through a frame, so a test that pumps and then
snapshots is sampling whichever side of that frame it landed on. Count real
frames with a tick callback instead; `gtk_focus_visible.rs::frames` is the
worked example.

Counting frames is still not enough. A fixed budget is a guess that holds
until the machine is loaded, and the symptom is nasty: the first focus test
gave **796, 796, 796, 0, 0** changed pixels across five runs of one build —
never a value in between. Binary, not partial, which is worth knowing because
it rules out every explanation about thresholds, clipped outlines or colours
being too subtle, and points at ordering. Sample repeatedly until two
consecutive renders agree, then compare.

Keep stability as the *precondition* and never as the assertion. Waiting for
"the pixels differ from before" would be waiting for the thing under test —
the exact way an await-for-condition test quietly becomes one that cannot
fail. Settle, then assert.

**`has_focus()` is false on a focused widget in a headless window.** GTK gates
`has-focus` on the toplevel being *active*, and a headless window never is, so
it reads false on a row GTK has put in `FOCUSED` state and is drawing the ring
for. It fails before any rendering happens and reads exactly like "focus never
landed", which cost an hour. Ask the question the CSS asks:

```rust
widget.state_flags().contains(gtk::StateFlags::FOCUSED)
```

**A control worth keeping for pixel tests.** Before believing a render
comparison that reports no change, push a deliberately loud override through a
`GtkCssProvider` — a background colour plus a fat outline. If that reports the
whole surface changed and the real rule reports nothing, the harness works and
the finding is real. If the loud one reports nothing either, the harness is
broken and the finding is not.


**`AdwWindow` draws no titlebar of its own.** `set_content(widget)` on an
`adw::Window` gives a window with no title, no close button and nothing to
drag it by — a stray rectangle rather than a window. The content has to
provide the chrome: an `adw::ToolbarView` with an `adw::HeaderBar` in
`add_top_bar`, which is what `window.rs` does for the main window and what the
detached composer (#48) does for its own. This is invisible to a widget test
and obvious the moment you render it, so if you build a second window, render
it: `cargo run -p postio-app --example shot -- /tmp/x.png demo compose
detached`.

**Reparenting a widget is how you move a surface without losing its state.**
The composer detaches by taking the same widget out of the reading pane and
into a window — `reader.remove(&composer)`, then the new window's layout
`set_content(Some(&composer))` — rather than by building a second composer
from the draft. Everything a rebuild would have to copy (every entry's text,
the `GtkTextBuffer`'s cursor, the identity `DropDown`'s selection, the
`postio_body::EditHistory`) simply never moves, so "detaching keeps them" is a
property of doing it this way rather than a list of things to remember. The
one thing a reparent really does lose is the **focus**: unparenting drops it,
so read `focused_field()` before and restore it after.

Two things that follow, and bit while building it:

- **Unparent from the actual parent.** Once the composer is a `ToolbarView`'s
  content, it is the *toolbar view* it has to come off, not the window.
- **`destroy()`, not `close()`, when you are inside `close-request`.**
  `close()` re-emits the signal you are handling.

**A satellite window's keys must forward to the main window's resolver, not
grow a keymap of their own.** The detached composer installs an
`EventControllerKey` that calls `Window::handle_key_in`, so `[keys]` in
`config.toml`, the registry and the palette all reach both containers and
there is only one keymap to keep in step. Two things genuinely differ and are
therefore passed in rather than read off the main window: the keyboard
`Context` (the main window has gone back to `List` by then) and whether the
user is typing, which is a fact about the *satellite's* focus —
`GtkWindowExt::focus` on the wrong window reports a widget nobody is looking
at, and the resolver's "typing always wins" rule would then swallow every
single-key binding.


**The cursor, the selection and an activation are three different facts, and
a surface that follows the wrong one silently follows nothing.** `postio-gtk`'s
message list keeps them apart on purpose: `j`/`k` move the *cursor*, `x` and
`Shift+J` change the *selection* an action would hit, and Enter or a double
click *activates*. That separation is correct and `gtk_selection.rs` enforces
it — but it means "wire this to the list" is not a well-formed instruction,
and picking the wrong one produces a surface that is fully built, fully
tested, and fed by nothing.

That is exactly how #70 shipped: `reading.rs` fed the reading pane from
`connect_activated`, so a mail client's right-hand column was blank unless the
user guessed that Return was required. Every layer underneath passed. If you
are wiring a surface to the list, say out loud which of the three you mean.

Three consequences worth knowing before you use `connect_cursor_moved`:

- **`SingleSelection` autoselects row 0 as soon as the model has rows.** That
  is not a person choosing anything, so it is deliberately *not* reported.
  Filling the reading pane there would, once #71's dwell timer exists, mark
  the newest message read because the application was opened. `move_cursor_to`
  and `extend_by` are the only paths that count as a landing.
- **The cursor lands before the mail arrives.** `set_source` sizes the model
  with placeholders and `deliver` fills it afterwards, so on a first page the
  cursor is already in place by the time there is anything to show and
  `notify::selected` has been and gone. Hence the `items_changed` hookup,
  which is also the fast-scroll case.
- **`items_changed` also fires for `update_row`** — a flag toggle, an incoming
  `\Seen`, any sync edit. That is not a landing. Reporting is therefore
  deduplicated on the *message id* rather than on the signal, which is what
  tells the three sources apart.

**An empty `MessageBody` is four different situations, and rendering it draws
the same nothing for all four.** #70's other half. A body that was never
downloaded, one whose blobs will not read, one that genuinely has no text or
HTML part, and one that is fine — the first three all reached the reader as
`MessageBody::default()`. On a mailbox mid-backfill that is most messages, for
minutes, so a correctly-working client looked broken.

`BodyState` is what distinguishes them and it has to be:
`MessageRepository::body_blobs` answers a row naming no blobs *both* for a
message nobody has downloaded and for one that was downloaded and had nothing
in it. Identical at the blob layer, opposite to a reader — one is worth
waiting for and one is finished. `compose.rs::load_body_or_reason` reads
`message.sync.body_state.has_body()` first for that reason;
`postio_gtk::reader::Absent` is the vocabulary it maps onto.

`load_body` keeps its old shape beside it, because the reply path genuinely
does not care: quoting nothing is the right degraded behaviour there.

**"Is this surface open" and "does it have the keyboard" are different
questions, and conflating them silently kills keybindings.** `Window::key_context`
asked `Finder::is_open()`, which was right until search began deliberately
leaving the field up after a query — from then on the resolver stayed pinned
to `Search` while the user was back in the message list, and every bare key
was dropped for the rest of the session with nothing logged. That is #73,
reported as "single-key bindings stop working, seemingly at random". Any
surface that can stay open while the keyboard is elsewhere has to be asked
the second question.

**`is_focus()` is not `has_focus()`.** `has_focus` additionally requires the
toplevel to be the *active* window, so it is false whenever the user has
alt-tabbed away — and always false under a headless compositor, where no
window is ever active. When the question is "which widget is the keyboard on
in this window", use `is_focus()`, or `GtkWindowExt::focus(window)`. This has
now cost time twice: once in #73, and once trying to prove a focus ring was
drawn (#90).


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

**Re-pointing a mailbox role relabels folders; it never moves mail.** `[mailboxes]`
lets a user say which folder is the archive on a server that advertises no
`SPECIAL-USE` and names its folders in a language `match_name` was never
taught (#164). The question that needed deciding was what happens to mail
already filed under the old resolution when that mapping changes, and the
answer is nothing:

- A role is a property of a **mailbox row** — which folder plays which part —
  and not of any message. Messages live in folders; re-pointing `archive` says
  nothing about where anything already is.
- Moving mail to match would mean Postio issuing IMAP moves the user never
  asked for, on a config edit. That is squarely against "nothing leaves this
  machine that the user did not ask for".
- It would also be irreversible in a way relabelling is not. Edit the line
  back and the labels swap back; moved mail stays moved.

The non-obvious consequence, and the one a test caught rather than the design:
**a pinned role has to be taken away from whatever held it before.** Point
`archive` at a new folder on a server that already has one called `Archive`
and, without that rule, two rows wear the role — and `by_role` returns one, so
archiving goes to an arbitrary one of them and which one can change between
runs. The previous holder is demoted to `Regular`, which is what it is once it
is not the archive.

Precedence is **override → `SPECIAL-USE` → name guess**, and the override sits
above the server attribute rather than merely above the guess: a server that
marks a folder `\Junk` is usually right, and "usually" is what an override is
for. It is one function, `RoleOverrides::resolve`, so the precedence is
stateable in one place — but it is called from `postio-sync`'s reconciliation
rather than from `MailboxRole::resolve`'s own call site at the IMAP edge,
because that layer parses what the *server* said and has no business reading a
config file.

`[mailboxes]` is read once at startup, so a mapping edited while Postio is
running takes effect at the next start. The engine is spawned with its parts
and folder discovery runs inside it, so applying a change live means reaching
into a running task.


**A `oneshot`-reply `Job` on the engine is a fact nobody will ever hear.**
`Engine::backfill_progress` could always answer how far the body queue had
got, and in the entire workspace nothing called it. So the longest phase of a
first sync — the bodies, not the message list — reached the frontend as no
event at all, and `announce_status` maps `Syncing` with no progress onto
`ConnectionState::Online`, which the sidebar draws as **idle**. The
application was reported as doing nothing while it downloaded a mailbox
(#74). That is worse than silence: a user watching `idle` concludes it is
stuck and goes looking for a bug that is not there.

The rule this suggests: a pull-shaped API is right for *asking* (a settings
pane, a diagnostic), and is never sufficient for anything the status line
needs. If a subsystem moves the status, it pushes through `announce_status`'s
neighbourhood, which exists precisely so the frontend does not have to know
which subsystem moved it. Check for other `Job` variants with a
`oneshot::Sender` whose only reader is a test.

A push added to the engine loop needs the same throttle the sync side already
has. `StatusTracker` puts a 250 ms floor under a pass's batches and never
drops the batch that finishes the pass; the backfill's announcement follows
that policy exactly, because a body settling is not a redraw and a fast
server produces far more of them than a status line can show.

**The list phase and the body phase are not the same status, and folding them
together loses the thing the user needs.** A mailbox mid-initial-sync cannot
be read; one whose bodies are still arriving is perfectly usable. So `syncing`
stayed the list's word and the backfill got `downloading`, with
`SyncStatus::progress` and `SyncStatus::backfill` as separate fields and the
list outranking the bodies when both are running.

Two details worth keeping:

- **The backfill has an honest denominator and the list pass does not.** The
  list's `total` is `UIDNEXT - 1`, an upper bound that expunged messages leave
  gaps in, so a pass routinely finishes well short of it — which is why that
  line reads `fetched 1204` and deliberately not `of`. `BackfillProgress`
  keeps every queued message in exactly one of its counts, so
  `settled + pending + in_flight` really is everything, and `bodies 412 of
  2000` is true.
- **Both phases must clear their number when the queue drains**, or the line
  sticks — `syncing 89%` on a finished folder was the original version of this
  bug, and `downloading 2000 of 2000` would have been the new one.

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
  transaction at once — there is no await between the `BEGIN` and the
  `commit()`. Do not add one: on a current-thread runtime a task that blocks
  in SQLite's busy handler blocks every other lane with it, so an await inside
  a write transaction turns lock contention into a stall of the whole engine.

**Every outermost write transaction is `BEGIN IMMEDIATE`, and has to be**
(#79). A deferred transaction takes no lock, and every write path in the
storage layer reads before it writes — `SyncStateRepository::mutate` loads the
state before saving it, `upsert_batch` looks a UID up before choosing insert
or update. So a deferred transaction is holding a *read* lock by the time it
writes and has to promote, and SQLite will not let a promotion wait: blocking
a connection that already holds a read lock could deadlock against the writer
it would be waiting for, so it returns `SQLITE_BUSY` and deliberately does
**not** invoke the busy handler. `PRAGMA busy_timeout = 5000` never gets a say
and the write fails on the spot.

The second writer is always there — the UI thread writes local-first on every
flag, archive and draft autosave, through the same pool — so this was never
theoretical. `crates/postio-sync/tests/concurrent_writers.rs` loses a sync
pass's *first* batch to it, every run, without the fix.

Two places decide this and both had to change:
- `postio_storage::repository::Scope::open`, the chokepoint for all 26
  grouped writes in that crate. A bare `SAVEPOINT` outside a transaction
  *starts* a deferred one, so `Scope` now asks `Connection::is_autocommit()`
  and issues `BEGIN IMMEDIATE`/`COMMIT` when it is outermost and
  `SAVEPOINT`/`RELEASE` when it is nested.
- The batch transactions in `postio_sync::initial` and `postio_sync::resync`,
  which open their own transaction rather than going through `Scope`.

Each is independently load-bearing: reverting either alone puts that test back
to failing every run. And do not expect the extended code to be
`SQLITE_BUSY_SNAPSHOT` (517) — the promotion failure reports plain
`SQLITE_BUSY` (5) about as often, and the two are one problem with one fix.
What identifies it is that it arrives in milliseconds against a five-second
timeout.

**A concurrency test must not use `test_support::memory()`** (#79). An
in-memory database is opened with SQLite's shared cache, a different locking
model from the WAL one Postio runs on: locks are per-table and a reader blocks
a writer outright rather than the two proceeding side by side. Combined with
the current-thread runtime above, one lane waiting on such a lock blocks every
other lane, and `sync_wave.rs` — whose whole subject is that passes overlap —
went from green to timing out purely because of the store underneath it, not
because anything about the engine had changed. It uses `test_support::temp()`
for that reason. #79's own testing note reached this from the other direction:
in-memory fails with `SQLITE_LOCKED`, which is a different bug.

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

**The parts panel's held-back "trackers" count is always zero (postio-m2ex).**
`PartsPanel::set_held_back(remote_images, trackers)` takes two counts, but
`postio_body::Sanitized::remote_blocked` — the sanitizer's only signal — is
one number: every `<img src>` pointing at a remote host, counted the same way
whether it is a 1200px product photo or a 1×1 open-rate beacon. `Window::reader()`
wires `Reader::connect_rendered` straight to `set_held_back(count, 0)`, so the
note in the panel only ever says "N remote images", never "and 1 tracker",
until something in `postio-body::sanitize` can actually tell the two apart
(a size/dimension heuristic, most likely). Not a bug — `set_held_back`'s
two-count shape was already there waiting for this, and postio-m2ex's own
issue text called it out as the one part "needing a change outside
postio-app". The tracker-detection work is its own issue (#174) rather than
a heuristic guessed at under `set_held_back`'s wiring.

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


**A bulk flag write rebuilds `messages.flags` rather than editing it.** The
column is documented as canonical spellings in `FlagSet` order, and five of
them — `\Seen`, `\Answered`, `\Flagged`, `\Deleted`, `\Draft` — are
denormalised into booleans beside it so the list and the sidebar never parse a
string. Those five are also, and not by accident, the five lowest-ranked
persistable flags in `Flag::rank`. That is what makes a whole-mailbox flag
write expressible without reading a row: the text is always "those five, in
column order, then the keywords", so `MessageRepository::set_flag_on_set`
rebuilds the head from the booleans and keeps the tail by stripping the five
system spellings out of the text it already has.

The tempting version — `replace` the flag out, append it back on — is one
`replace` shorter and puts `\Seen` last. Nothing catches that, because
everything reads the column back through `FlagSet`, which re-sorts. The
invariant it breaks is the schema's, and it would surface as a diff against a
server's flag list months later. `bulk::the_flags_with_columns_are_the_five_that_sort_first`
is the guard: add a system flag ranked ahead of those five and it goes red.

**A whole-mailbox flag has to enqueue over the rows that *disagree*, not over
the selection.** `MessageSet::WithFlag` narrows a set by the denormalised
column, which is a comparison rather than a read, and `Actions::set_flag_set`
uses it for both the queue rows and the write. Enqueueing over the wider set
looks harmless — the drainer would send a redundant `STORE` — but the run of
queue rows *is* the undo set (`MessageSet::Queued`), so a message that already
carried the flag would land inside it and `u` would clear a flag the action
never set. The same reasoning applies to any future bulk verb whose effect is
conditional on the row's current state.

A toggle over a predicate means the same thing it means over a selection —
*make them agree* — and deciding which way it goes is two indexed `count(*)`s,
never a read. The two counts also separate "this mailbox is empty" from "these
already agree", which are different sentences.


**A draft exists twice, and the composer owns it.** Saving a draft appends it
to the account's Drafts mailbox, and the next sync pass over that folder
fetches it straight back — so the same unfinished message is both a `drafts`
row (the composer's live buffer, autosaved as the user types) and a `messages`
row (a read-only snapshot of that buffer as of the last append). #51.

`MessageRepository::upsert_batch` therefore drops from its batch any message
whose `(mailbox, UIDVALIDITY, UID)` a local draft row already claims. Three
things about the shape of that:

- **It takes `&mut Vec` and shortens it**, rather than skipping quietly. Both
  sync passes go on to `for message in &batch` for threading and for recording
  correspondents; `resync.rs` also pushes each into `arrived`, which is what
  notifies. A skip that left the message in the batch would thread a row that
  was never written and announce the user's own draft as new mail.
- **It is in the store, not in the two passes that call it.** A skip each
  caller had to remember is one a third caller would not, which is the same
  argument the `0003_mailbox_counts` triggers make about `recount`.
- **It matches on the mailbox too.** UIDs are per-mailbox, so the message that
  happens to be number 7 in the inbox has nothing to do with the draft that is
  number 7 in Drafts, and matching on the number alone hides mail.

The skip alone does not close the race where a pass fetched the appended copy
before `set_server_copy` recorded where it landed: the row is already there,
every later pass finds it and keeps it current, and the duplicate is permanent.
So claiming the copy is also what disowns the row — `DraftRepository::set_server_copy`
deletes any message row for the same server copy in the account's Drafts folder.

A draft whose append the server would not locate (no `UIDPLUS`) has no `uid`,
matches nothing here, and appears as an ordinary message. That is deliberate
and is `postio-sync::drafts`' standing rule: it flags the folder for a resync
rather than guessing which message in Drafts is the one it just wrote.

The consequence is that your own draft is in the composer and nowhere else —
the Drafts folder lists other clients' drafts only. That is #166, and it is the
deliberate other half of this decision rather than an oversight.


**A draft's place in the Drafts folder is a `messages` row the composer
writes.** #51 stopped the synced copy of a draft becoming a second message row;
what that left was a Drafts folder listing other clients' drafts and nothing
else, and a sidebar badge — which reads the mailbox's cached count of message
rows — saying 0 while the composer held one. #166.

`DraftRepository::save` therefore maintains a row in the account's Drafts
mailbox, linked by `drafts.message_id`, and `delete` removes it. `delete` is
the single exit both discard and send go through (`postio-sync::send` finishes
there), which is why it is the one place that has to remember.

Two designs were rejected, and both look cheaper:

- **Keeping the synced copy and routing its activation to the composer.** A
  draft has no server copy until an append has round-tripped, so the folder
  would list your draft only *after* a network exchange. `docs/PRODUCT.md` §18
  and the local-first rule both forbid exactly that. The mirror row is written
  in the same transaction as the draft, offline and always.
- **Making the list's row identity a sum type over `MessageId | DraftId`.**
  That reaches `ListCursor`, `MessageSummary`, the selection model and every
  `CommandId` target — the hottest path in the application — to solve a problem
  one nullable column solves.

`set_server_copy` attaches the UID to that row rather than creating one, and
its stray-row delete has to run **first**: `messages` is unique on
`(mailbox_id, uid_validity, uid)`, so attaching while a duplicate holds the
same identity is a constraint violation rather than a duplicate.

`load_body_or_reason` answers a draft's row from the draft's own inline body.
The row has no blob and never will — the composer's buffer is inline TEXT
precisely so a content-addressed store does not take one immutable blob per
keystroke — so reading the row would say "still downloading" about words the
user is looking at in another pane.

**`Composer::resume` is the exception to one-composition-at-a-time, and it is
allowed to be.** `open` refuses to replace a retained draft, because `c` a
second time means "show me the draft". `resume` takes a draft the user named
out of a folder, so it replaces — safely, because the draft it replaces is
autosaved and is itself a row in that folder now. It flushes the pending
autosave first: the debounce would otherwise fire against the draft that
replaced it, writing one draft's words onto another's row.

**`Return` on a list row cannot be tested through `Window::handle_key`.**
Activation reaches `connect_activated` through `GtkListView`'s own
`list.activate-item` action, which needs the view to hold the keyboard;
`handle_key` goes through the keymap and never touches the widget. That is why
`ListPane::test_activate_cursor` invokes the action rather than calling the
handlers — a wiring that came loose between the action and the signal still
has to show. `crates/postio-app/tests/resume_draft.rs` is the worked example.

Similarly, `Sidebar::select` is documented as selecting "without reporting it
back as a user action", so a test that uses it changes the sidebar and leaves
the list showing the previous folder. Click the row instead.


## Testing infrastructure

**`gtk_reader` hanging at 0% CPU with no output runs under a watchdog
now (#272).** The one test binary that talks to WebKit directly wedged at
least four times during gate runs — silent, 0% CPU, killed by hand each
time, twice while the box carried several sessions' concurrent builds. The
test's own waits are all deadline-bounded, so the block is inside a
toolkit or WebKit call; the standing suspect is WebKit's DMA-BUF renderer
negotiating GPU buffers with the nested headless mutter. Two changes in
`scripts/headless-runner.sh`: `WEBKIT_DISABLE_DMABUF_RENDERER=1` pins
WebKit to its software path under the test compositor (tests need no GPU
web rendering), and `gtk_reader-*` binaries run in their own process group
under a hard deadline — `POSTIO_TEST_WATCHDOG`, default 300s — that dumps
every thread's kernel `wchan` before killing the group, WebProcess
children included. So the next hang costs five minutes and leaves a
diagnosis in the log instead of an unbounded wait that only a human ends.
A 25-iteration loop under concurrent build load did not reproduce the
hang; if the wchan dump ever shows one, paste it into #272.

**A test that skips when there is no display reports success, and CI had no
display.** Sixty test files in this workspace open with some spelling of
`if adw::init().is_err() || gdk::Display::default().is_none() { return; }`.
That is correct for a contributor on a headless shell and wrong for a runner:
CI installed no display server, so every one of those sixty returned early and
the job went green having run none of them. The accessibility audit that
`docs/PRODUCT.md` §20 depends on is one of the sixty (#114).

The fix is in two halves, and the second is the one that lasts:

- `ci.yml` now gives the suites a display. It does that by handing
  `scripts/headless-runner.sh` what it already wants — `mutter` on PATH and an
  `XDG_RUNTIME_DIR` — rather than working around it, so CI exercises the same
  Wayland configuration developers do instead of a second one that behaves
  differently. Xvfb is started too, as a backstop: a runner has no logind seat
  or session bus, so `mutter --headless` may refuse to start, and the runner
  then fails open and execs the test binary unchanged with the `DISPLAY` set.
- `crates/postio-gtk/tests/gtk_display_required.rs` fails the build when `CI`
  is set and there is no display. One test rather than sixty edits: if a
  display is present none of the sixty skip, so "CI has a display" is the
  whole property and it is asserted once.

The general rule, which is worth applying to any skip: **a skip that is right
locally and wrong in CI has to know which one it is in.** A skip nobody can
distinguish from a pass is not a test.


**"database table is locked" on a line that is only a fixture** meant the
scratch database, not your test. Until #204, `test_support::memory()` was
`:memory:` with `cache=shared`, whose *table-level* locks return
`SQLITE_LOCKED` immediately — `busy_timeout` covers only the file lock, so
no pragma waited it out, and the failure rate tracked machine load. A read
transaction on one pooled connection (a list page mid-iteration) failed a
plain write on another, in a test about something else entirely. Fixed by
making `memory()` file-backed in a self-cleaning tempdir (`/dev/shm` where
present, so it still costs RAM); the tempdir rides inside the pool via a
guard slot, so clones of the `Database` keep it alive. If that error string
ever reappears, something reintroduced shared cache — start at
`Database::open_in_memory`'s doc comment, which now records the caveat.

**Tests that fail under load and pass alone are a family, and the fixes are
a doctrine** (#55, #80, #109, #122, #125, #210, #219 — the same lesson,
re-learned): *assert order and causality, never wall-clock overlap* (a
"still running when X finished" assertion goes vacuous exactly when the
machine is slow — record sequence in the mock and compare positions);
*liveness deadlines are minutes, not budgets* (a timeout exists to catch a
hang; performance claims live in the benches); *faults persist, not
positional* (`inject_after` schedules by absolute call count, and an
autonomous engine loop's own backend calls shift which call the fault
lands on — a test that means "the server refuses X, whoever asks" wants a
persistent fault); and *reproduce under `cargo test --workspace
--no-fail-fast` with something else compiling before fixing*, because a fix
verified on a quiet box proves nothing about the only condition that fails.
`tokio::time::pause` is not the escape hatch for any test doing real I/O
(engine + SQLite threads, in-process sockets): auto-advance misfires with
real blocking work in the loop.

**A tokio future awaited on the GTK main context type-checks, passes clippy,
and panics the first time the line is reached.** `spawn_future_local` runs on
the glib main loop, which has no reactor, so
`secrets.store(..).await` there gives "there is no reactor running" — which
shipped in 0.1.0 and made the app unable to add an account with every gate
green. The rule was already written down in `postio-app/src/feed.rs` and
followed everywhere except the one path no test could reach, which is why the
guard is now static: `scripts/check-runtime-crossings.py` refuses any `.await`
inside a `spawn_future_local` block that is not a channel receive. Nested
`runtime.spawn(..)` blocks are exempt — that is the crossing working. An await
that is safe without being a receive needs a `POSTIO-GLIB-SAFE:` comment
saying why.

Worth knowing what this class of bug looks like, because it is not obvious in
review: the suspend point is often several calls away. The check's first real
find was `part_bytes` in `reading.rs`, which returns without suspending when
the message body is local — so every seeded test passed — and reaches
`tokio::time::sleep` only when it has to wait for a download. **A test over
seeded fixtures cannot catch this**, which is the conclusion #66 reached
about onboarding before asking for the lint instead.


**GTK records no accessible properties unless an accessibility backend is
running, so an a11y test with no backend measures nothing.** GTK builds a
`GtkATContext` per widget lazily, and only when a backend is live. A headless
session has no a11y bus, so it gets `GTK_A11Y=none`, so there is no context,
so `gtk_test_accessible_has_property` answers "not set" for every widget on
screen no matter what the code did. On a maintainer's desktop at-spi *is*
running, which is the whole of the difference — and it is why
`gtk_accessibility.rs` failed headless and passed live for long enough that
the split was misread as a timing race (`postio-9112`). Verified against
plain GTK outside this codebase: the same list item reports `has_property=0`
under `GTK_A11Y=none` and `1` under `GTK_A11Y=test`. Any test asserting
accessibility must select a backend itself — `GTK_A11Y=test` needs no bus —
and should prove it has one before drawing conclusions, which
`require_an_accessibility_backend` does by setting a name on a throwaway
widget and reading it back.

**`gtk_test_accessible_has_property` asks whether a property was set, not
whether it says anything.** A widget labelled `""` reads as named. That is a
state this tree really reaches — the message list's unbind path sets `""`
deliberately to clear a recycled row — and it made the row-naming assertion
unable to fail at all: sabotaging `announce()` outright left the test green.
gtk-rs binds no getter for a property *value*, so the only way to ask is
`gtk_test_accessible_check_property` from `gtk4-sys`, which compares and
returns NULL on a match. Hence `gtk4-sys` as a dev-dependency of
`postio-gtk`.

**Cargo gives every integration test *binary* its own process — but not
every test *function*.** libtest runs each `#[test]` on a thread of its own
even at `--test-threads=1`, and GTK may be initialized from exactly one
thread, so the second test in a binary to reach `adw::init()` aborts with
`gdk_display_open_default() was called before gtk_init()`. Moving GTK tests
out of `#[cfg(test)] mod tests` into `tests/` is only half the fix
(`postio-yxfn` stopped there, and `gtk_toast` went on aborting). **One test
function per file** for anything that touches a display, the way
`gtk_style.rs`, `gtk_accessibility.rs` and now `gtk_toast.rs` are.
Deterministic under `--test-threads=1`, intermittent otherwise, so it reads
as flakiness. `gtk_composer_autosave.rs` and `gtk_finder.rs` still have this
shape — see #41.

**A scroll area is a tab stop, and an unnamed one announces nothing.**
`GtkScrolledWindow` takes the keyboard so it can be scrolled with one, which
puts it in the focus order *before* the widget inside it. Three of them —
settings' config view, the composer's body, the thread's message column —
each announced nothing when focus landed there. Give the region the name of
what it scrolls, from a constant shared with the widget inside so the two
cannot drift into disagreeing.

**Comparing rendered pixels to prove a focus ring is drawn does not work
reliably headless.** The technique itself is sound — `WidgetPaintable` +
`render_texture` + `download`, and a deliberately loud override does show up
— but against the real stylesheet it reports the ring about one run in five.
`pump()` drains the main context without guaranteeing a frame carrying the
new CSS state. Do not ship such a test without waiting on a real frame; the
full record of what was ruled out is in #90.

**GTK may be initialized once per process, so it can never be initialized
from a unit test.** `cargo test` runs a crate's unit tests on a thread pool
inside one binary. GTK's init is process-wide state guarded by a
one-thread-only assertion, so two unit tests that both call `adw::init()` are
two threads racing for it. The loser does not fail a test — it kills the
process, and every other test in that binary is never reported at all.

Found by #41: four unit tests in `crates/postio-gtk/src/toast.rs` did this.
CI reported `signal: 6, SIGABRT` and zero of postio-gtk's 305 passing tests.
It had survived every developer machine, because whether it aborts depends on
which thread wins and on whether a display exists. Reinstating the four tests
while working #41 reproduced it as **SIGSEGV on one run in three** under
`scripts/test-headless.sh`, and not at all display-less. That ratio is the
lesson: a crash this shape cannot be shown absent by running the suite again.

What to do instead: put anything needing a display in `crates/<crate>/tests/`,
where cargo gives each integration test file its own process.
`crates/postio-gtk/tests/gtk_toast.rs` is the worked example, and every other
`gtk_*.rs` beside it follows the same `if adw::init().is_err() || ...` guard.

The one legitimate exception is a crate with no lib target — an integration
test has nothing to link against. `postio-app` is a binary crate and keeps
exactly one GTK-touching unit test in `src/compose.rs` for that reason.

`scripts/check-no-gtk-init-in-unit-tests.py` enforces this in CI and in
`issue-land.sh`. It reads `#[cfg(test)]`/`#[test]` spans rather than grepping
for `adw::init`, so production code initializing GTK is untouched; the only
way past it is a `POSTIO-GTK-INIT:` line in the file arguing why the test
cannot move. Its own failure modes are exercised by
`scripts/test-check-no-gtk-init-in-unit-tests.py`, since the tree is clean and
a guard on a clean tree passes whether it works or not.

**A test that builds a `Window` reads the developer's own `$XDG_STATE_HOME`,
and that decides what the test sees.** #215 was reported as "`gtk_reading_pane`
is red on `main`" — consistently, on every commit and every branch — and the
report pointed at the headless runner. It was not the runner.
`Window::reader()` built its `Reader`
with `Reader::new`, which loads the standing remote-image allow list from
`$XDG_STATE_HOME/postio/remote-images.ini`. The test renders a body with a
remote `<img>` from `ada@example.com` and asserts the parts panel hears that
one reference was held back. On a machine where that sender had an "always
allow" exception, the body rendered with its images *permitted* — so nothing
was held back, `set_held_back(0, 0)` hid the badge, and the assertion failed.
The `connect_rendered` callback the reporter concluded "never arrives" arrived
every time, carrying `0`, which is why replacing the assertion with a
ten-second await did not help either.

The signature to recognise: **red for one person on every commit, green for
everyone else, and a bisect that finds nothing** means the cause is not in the
tree. Reproduce it by putting the state back rather than by re-running —
`XDG_STATE_HOME=<scratch> cargo test ...` with the file written by hand took
this from unexplained to proven in one run.

Two fixes, both on the branch for #215. `Window::set_allowlist_path` points a
window under test at a scratch file; it must be called before anything asks
for `reader()`, because the list loads once when the reader is built and stays
in memory for its life (deliberately — there is never meant to be a second
opinion about who is allow-listed), and a `debug_assert!` says so.
`scripts/run-isolated.sh` now exports `XDG_STATE_HOME` alongside
`XDG_DATA_HOME` and `XDG_CONFIG_HOME`; it had isolated the store and the
config but not the state, so looking at the demo mailbox and clicking "always
allow" once wrote a real exception into the real file — the most likely way
the poisoned entry got there in the first place.

The general rule: `$XDG_STATE_HOME` is not just window geometry. Anything a
test constructs that reaches it needs a seam, or the suite is asserting about
the machine.

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
- `scripts/issue-land.sh` used to share the target directory by default
  (`export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$MAIN_CHECKOUT/target}"`), so
  the landing gates — the one run a merge is staked on — were exposed to it
  too. #253 removed that default: the script now builds in the calling
  worktree's own `target/` and only honours `CARGO_TARGET_DIR` when a caller
  has genuinely set one. `scripts/test-issue-land-target-dir.py` holds it
  there, by building a real crate through `--gates-only` and looking at where
  the artifacts landed.

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

CLAUDE.md and the `/issue` skill used to recommend
`export CARGO_TARGET_DIR=~/src/postio/target` to keep the GTK and WebKit
builds warm. That advice was right about the cost it was avoiding — it just
was not free, and the above is the bill. #178 settled it the other way:
every worktree gets its own `target/`, and the ~400 third-party crates stay
warm through `export RUSTC_WRAPPER=sccache` instead, which keys on exact
compiler inputs and so cannot produce this confusion at all.

The lesson generalises past cargo: when a fix removes a shared resource,
grep for the places that *default* to it, not just the places that name it.
#178 changed both instruction documents and left `issue-land.sh`'s default
in place for #253 to find — and a default is worse than an instruction,
because nobody has to read it for it to fire.

**"A single cargo invocation is safe" (above) is about the target directory,
not the source tree — a long build run directly in the shared checkout can
still be torn by a concurrent `git pull`.** Observed 2026-08-25 verifying
`main` after the postio-session refactor: `cargo test --workspace
--no-fail-fast` was run against `~/src/postio` with its *own*
`CARGO_TARGET_DIR`, specifically to dodge the hazard above. Twenty minutes
into a cold build it failed anyway — `postio-imap` used
`Message::content_type`, but the `postio-model` rlib it linked against had
no such field. Both true at once only makes sense if the two crates were
compiled from different moments of the same tree, and `git reflog` said
so: `pull: Fast-forward` had landed five commits, including the one adding
`content_type`, while the build was still running. Cargo had already
compiled and cached `postio-model` from before the pull; `postio-imap`'s
source was read from disk after it, use-site and definition torn across
the same invocation.

Isolating `CARGO_TARGET_DIR` answers "whose *artifact* is this" (the entry
above); it says nothing about "whose *source tree* is this", because the
shared checkout is exactly one directory that every session's `git fetch`,
`git pull`, or `git checkout` can rewrite while somebody else's `rustc` is
mid-read of the same files. `scripts/run-isolated.sh` already avoids this
for the running app by pinning a worktree to a commit rather than reading
`~/src/postio` live. A verification run worth staking a report on needs the
same pinning: `git worktree add <path> <commit>` (or run it in an existing
issue worktree, which is already pinned to its own branch) rather than a
scratch target dir against the one checkout everyone else is still moving.

**A test that spawns a real `Engine` needs `test_support::temp()`, not
`test_support::memory()` — `:memory:` has no WAL.** #109 tracked
`reading::tests::a_part_nobody_has_is_fetched_before_it_is_saved` failing
about once in a dozen runs, load-correlated, always finishing in exactly the
engine's `POLL_INTERVAL` (5s) whether it passed or failed — which read as a
timing coincidence worth chasing but wasn't the mechanism. Reproduced under
sustained moderate CPU load (`yes > /dev/null` on half the cores, matching
"two other worktrees compiling"): `world()`'s own setup query panicked with
`SQLITE_LOCKED` ("database table is locked: messages"), and separately the
test's own follow-up read failed the instant after the awaited fetch had
already succeeded. Neither took anywhere near the 30s `BODY_WAIT` deadline —
both failed as fast as a query returns, which is what actually pointed away
from the poll interval and at SQLite.

The mechanism: `test_support::memory()` opens a shared-cache `:memory:`
database, which cannot use `journal_mode = WAL` — there is no file to write
a WAL against, so it falls back to `memory` journalling, where a writer and
a reader on the same table can collide as `SQLITE_LOCKED_SHAREDCACHE`. That
is a different error from `SQLITE_BUSY`, and critically, `busy_timeout`
does not retry it — recovering from `SQLITE_LOCKED` needs SQLite's
unlock-notify API, which this pool does not use, so the error comes back on
the first try, immediately. A test with only one connection never notices;
one that spawns a real `Engine` on its own thread — anything that touches
`MailBackend`, not the seeded-fixture kind — is running exactly the writer
that can collide with it. `world()` in `reading.rs` does; the fix was
switching it to `test_support::temp()`, which is file-backed and gets the
same WAL guarantees production reads run under. `test_support`'s own doc
comment already said as much ("`temp` when the test is *about* the
file — WAL behaviour... because an in-memory database has no journal"); the
part worth remembering is that "about the file" includes any test running a
concurrent writer, not only tests that reopen or inspect the file directly.

## Logging & privacy

**`Zeroizing<String>` protects the password; the buffers around it are where
it escapes.** `postio_imap::secret::Password` was always the right shape, and
#144's security review still found live copies that were freed without being
overwritten — all of them on the way *into* a `Password`, and the worst of
them on error paths where the secret never became one at all. Two rules came
out of it:

- **`String::from_utf8` is the trap.** On success it moves the buffer into a
  `String`, which is fine only if that `String` is itself zeroized; on failure
  it drops the bytes it was given, unprotected. `secret.rs` now goes
  `Zeroizing<Vec<u8>>` → `std::str::from_utf8` → `&str` → `Password`, which
  borrows instead of converting, so no second allocation exists on either
  path. The signature is the enforcement: `secret_text` accepts only
  `&Zeroizing<Vec<u8>>`, so a caller holding a bare buffer has to wrap it
  before it can get a password out at all, and the compiler is what checks
  that — nothing else could.
- **`SecretString::from(String)` reallocates, and reallocating frees a secret
  without overwriting it.** `secrecy::SecretString` is `SecretBox<str>` and
  does zeroize on drop, but it is built through `String::into_boxed_str`,
  which calls `shrink_to_fit` — so a `String` with spare capacity is copied to
  a fresh allocation and the old one is freed with the password still in it.
  The copies handed to io-sasl (`postio-imap/src/imap/mod.rs`'s
  `credential_copy`) and to io-smtp (`postio-sync/src/send.rs`) are single
  `str::to_owned` calls for exactly this reason: `to_owned` allocates `len`,
  so the buffer moves. A `String::with_capacity`, a `push_str` or a `format!`
  in either place reintroduces the leak silently;
  `the_handshake_copy_of_a_password_has_no_spare_capacity` is what catches it.

For the record, checked against io-sasl 0.1.0 and io-imap 0.6.0: the password
we hand over is protected the whole way down. `SaslPlainCreds::passwd` is a
`SecretString`, and io-imap makes one further copy inside
`ImapAuthPlain::new` which is also a `SecretString`. Two copies, both
zeroized.


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


## Toolchain

**The rustc version is pinned in `rust-toolchain.toml`, and
`RUSTUP_TOOLCHAIN` beats it.** rustup's precedence is: the `RUSTUP_TOOLCHAIN`
environment variable, then a `rustup override` for the directory, then
`rust-toolchain.toml`, then the default. So a machine that exports the
variable ignores the pin *while looking pinned* — every gate green, every
session confident it matches CI, and the same skew as before wearing the
fix's clothes.

This workstation exports it: `~/.config/mise/config.toml` has a `rust = "..."`
pin, and mise puts `RUSTUP_TOOLCHAIN` into every shell it starts. When the
repository pin moves, **that file has to move with it**, or nothing local
changes. This was found while fixing issue #38 — the pin was added, the check
passed, and `rustc --version` still printed the old compiler.

`scripts/check-toolchain-pinned.py` reports the skew rather than failing on
it, and `--strict` makes it fatal for anyone who wants that. It is deliberately
not fatal by default: a check whose exit status depends on a developer's shell
would make CI's verdict depend on the runner's environment, which is the
thing being fixed.

**A warning in a gate log is weaker than the pin was supposed to give.**
`scripts/issue-land.sh` and `scripts/test-headless.sh` are the two scripts
that run `cargo`/`rustc` on a session's behalf, so both capture
`RUSTUP_TOOLCHAIN` and `unset` it before invoking either — the gates run on
whatever `rust-toolchain.toml` names regardless of what the shell exports,
and `issue-land.sh` still prints the captured value afterward so the skew is
visible rather than silently corrected. This turns the warning into a
guarantee for the two paths that matter; it cannot reach a session's
interactive shell, where `rustc --version` still answers however
`RUSTUP_TOOLCHAIN` says to. `scripts/test-rustup-toolchain-cleared.py` proves
it with a `RUSTUP_TOOLCHAIN` naming a toolchain rustup has never installed —
that makes `rustc`/`cargo` refuse outright, so a regression here fails loudly
rather than silently drifting back. Issue #112.

**Why an exact version and not `stable`.** `channel = "stable"` in
`rust-toolchain.toml` floats exactly as hard as the `rustup default stable`
it replaced. The point of the pin is that the compiler changing under the
project looks like a commit somebody made rather than like weather: rustc
1.98.0 tightened `unused_imports` for redundant glob imports, flagged
`use adw::prelude::*` in `compose::tests`, and turned main red on a lint
nobody wrote — and it was unreproducible locally by construction. The check
refuses a channel name in that file for this reason.

**What the pin does not reach: the Flatpak release build.**
`flatpak/dev.postio.Postio.json` builds against
`org.freedesktop.Sdk.Extension.rust-stable`, which carries whatever rustc that
extension ships for `runtime-version: 50`. There is no `rust-1.98.0`
extension to name instead, so this one is pinned only indirectly, by the
runtime version. It is a weaker exposure than CI's was — a release build
either compiles or does not, and it is not a `-D warnings` gate that can turn
main red on a new lint — but it does mean the *shipped* binary and the tested
one may come from different compilers. `check-toolchain-pinned.py`
deliberately does not flag it: there is no alternative to flag it toward.

**Bumping it.** Change `rust-toolchain.toml`, change the mise pin to match,
and expect a cold rebuild: a different compiler shares no artifacts with the
old one, so the shared `target/` is dead weight the moment the pin moves.
Sweep it in the same change rather than letting both toolchains' output
accumulate — that directory reached 232 GB before anyone looked.


## Landing work

**`gh pr checks` cannot tell "nothing will run" from "nothing has run yet",
and `issue-land.sh` used to merge on the ambiguity.** It printed `no checks
reported` in both cases, and the script read that as "prose-only change,
nothing to wait for". Lost one way it cost a re-run — three consecutive
first attempts on #92, #106 and #118. Lost the other, on #135, it merged a
five-crate change before CI had started; CI passed afterwards, so nothing
broke, but that was luck rather than the guarantee the script exists to
provide. The whole reason it waits rather than using `gh pr merge --auto` is
that auto-merge lands a PR before CI registers, and this path did the same
thing (#139, #131).

The fix is that **the branch's own diff decides, not `gh`**. The workflows'
`on.pull_request` path filters are the authority on what a change schedules,
and `scripts/ci-expected-workflows.py` reads them — including the `&prose`
anchor/`*prose` alias that `ci.yml` uses to share one ignore list between its
`push` and `pull_request` triggers. `scripts/wait-for-checks.sh` then polls
for the checks it predicted and **refuses to merge** if one was due and never
appeared, while still watching briefly on a branch that should schedule
nothing, in case a rerun or `workflow_dispatch` produces one anyway.

Two things to respect if you touch this:

- **`gh pr checks` exit status cannot answer "is a check registered?"** It is
  non-zero both while nothing has registered and when a check has failed.
  Ask positively, with `gh pr checks --json name`, and treat `[]` as "no".
- **GitHub filter patterns are not shell globs.** `*` and `?` stop at a
  slash, `**` crosses them, and a later `!` pattern undoes an earlier match.
  `'*.md'` in `ci.yml` therefore ignores top-level prose only, which is why a
  hand-edit of the generated `docs/keybindings.md` still runs CI — the drift
  test in `postio-core/tests/keybindings_doc.rs` depends on that.

Both scripts have self-tests that CI runs: `test-ci-expected-workflows.py`,
and `test-wait-for-checks.py`, which drives the wait against a stubbed `gh`
so the registration race is reproducible instead of something you wait for.

**A script that rebases the tree it lives in runs its own pre-rebase self.**
That fix above landed, and then #50 merged a 1016-line, three-crate change
without waiting for CI anyway — printing a sentence (`no checks scheduled —
prose-only change, nothing to wait for`) that no longer existed anywhere in
the tree. Nothing was stale on disk. The order inside one run is what did it:
`issue-land.sh` runs its gates, then **rebases the worktree that contains
`issue-land.sh`**, and then keeps executing the copy bash already had open —
which is the version from before the rebase pulled the new machinery in. The
run that introduces a fix to landing is therefore the one run the fix cannot
protect, and it is the run whose author has least reason to expect the old
behaviour.

It is worse than merely stale. **bash reads a script by byte offset as it
goes**, so rewriting the file underneath a running shell can shift what it
parses next; the result is not reliably "the old version" of anything.

The fix is a **handover** (#160). Before rebasing, the script records
`git rev-parse HEAD:scripts` — one tree hash standing for the whole of
`scripts/`. If the rebase changes it, the run `exec`s
`$TREE/scripts/issue-land.sh` from the top with the same arguments, under
`POSTIO_LAND_REEXEC_DEPTH`, and gives up rather than merging past
`POSTIO_LAND_REEXEC_LIMIT` (2) handovers. Three things make that safe rather
than clever:

- **The whole decision sits inside the same `if [ "$BEHIND" -gt 0 ]` block as
  the rebase.** bash parses a compound command in full before executing any
  of it, so that block is already in memory when the rebase rewrites the
  file. Code placed *after* the block would be re-read at a byte offset into
  a file that has changed. Keep it there.
- **Nothing has been pushed yet at that point**, so a handover cannot double
  a push, a PR or a merge. If you move the push earlier, this stops being
  true.
- **The re-run needs no "skip what you did" flag.** The work is already
  committed so the tree is clean, and the branch is now zero behind, so the
  commit and rebase steps fall through on their own — and the gates run again
  against the combination CI will actually see, which is the only way the
  gates and the merge decision can be talking about the same tree.

`scripts/test-issue-land-rebase-handover.py` covers all four orderings
(machinery rewritten, a *called* check tightened, an ordinary rebase, and the
bound reached) against a real bare remote with only `gh` stubbed. Its case A
is the #50 incident verbatim in shape.

## Landing work

**`issue-land.sh`'s gates do not include `cargo doc`, and CI's do.** Moving
code between crates is the case where that bites: a doc comment carries its
intra-doc links with it, and a link that resolved in the crate it came from
does not necessarily resolve in the crate it lands in. #82 moved `Wiring` out
of `postio-app` and its `[`run`]` link went with it, pointing at a function
that stayed behind — every local gate passed and CI failed on:

```
error: unresolved link to `run`
  --> crates/postio-session/src/lib.rs:88:72
```

Worse, the link cannot simply be repointed: `postio-app` depends on
`postio-session`, so rustdoc cannot resolve *upward* from the dependency to
its dependent at all. The fix is to name the item in prose rather than link
it, and say why it is not a link.

So after moving code between crates, run CI's own doc gate before pushing:

```sh
RUSTDOCFLAGS="-D warnings -A rustdoc::private_intra_doc_links" \
    cargo doc --workspace --no-deps --document-private-items
```

## The shared cargo target directory

**sccache's server outlives the worktree that started it, and
`issue-release.sh` can leave it pointing at a directory that no longer
exists.** `.cargo/config.toml` sets `TMPDIR = { value = "target/tmp",
relative = true }`, which resolves against *the workspace root of whoever
started the sccache server*. The server is one machine-wide daemon, it keeps
the environment it was launched with, and it is the process that actually
creates the compiler's temporary files. So releasing the worktree that
happened to start it breaks every subsequent build on the box:

```
sccache: encountered fatal error
sccache: error: Failed to create temp dir
sccache: caused by: No such file or directory (os error 2)
   at path "/home/.../postio-worktrees/issue-176/target/tmp/sccache350w6t"
error: could not compile `unicode-ident` (lib)
```

The path in the message names a worktree you may never have worked in, and
`unicode-ident` is whatever happened to compile first — neither has anything
to do with the failure. **Check whether anyone else is mid-build
(`pgrep -af rustc`), then `sccache --stop-server`**; the next `cargo` starts a
fresh server with the current `TMPDIR`. Do not do it while another session is
compiling — the running build dies with it.


**It hands you other worktrees' artifacts, and the compile error then names a
file that is correct.** This is not contention and not a stale cache — it was
demonstrated end to end while landing #82.

`cargo test -p postio-app` in the `issue-82` worktree failed with:

```
error[E0308]: mismatched types
   --> crates/postio-gtk/src/reader/view.rs:438:13
    |
438 |             sanitized.remote_blocked,
    |             ^^^^^^^^^^^^^^^^^^^^^^^^ expected `bool`, found `u32`
```

That worktree's own `postio-body/src/sanitize.rs` declares
`pub remote_blocked: bool`, and its `postio-gtk` is right to expect a `bool`.
The `u32` exists in exactly one place on this machine: the `issue-58`
worktree, where another session is mid-refactor turning that flag into a
count. So `postio-gtk` from one worktree was compiled against `postio-body`
from another, through the shared `CARGO_TARGET_DIR` that CLAUDE.md tells every
session to set.

The same run had produced a second symptom earlier —
`no variant ... named DetachComposer found for enum postio_core::CommandId`,
against a `command.rs` that declares it four times — from worktrees still on
an older `main`. Both are the same fault wearing different clothes.

**The worst instance so far did not look like a build problem at all.** It
looked like a broken `main`. `postio-gtk`'s
`cheatsheet::tests::the_sections_are_the_ones_the_registry_actually_uses`
failed *deterministically* — every run, filtered to that one test, single
threaded, in a fresh worktree and in the shared checkout — reporting an extra
"Thread" section holding two commands, "Unread only" and "Toggle order".
Neither string existed anywhere in the worktree under test. Both existed in
the `issue-61` worktree, where a session was adding them. The test binary had
linked *that* `postio-core`.

Two things make this the dangerous shape. It was **repeatable**, so the usual
"run it again" tell was absent. And it presented as exactly the case
CLAUDE.md's **CI is paused** section says to respond to by pulling `ready`
off every open issue — a disruptive, repository-wide stop, triggered by a
regression that did not exist. Rebuilding in a private `CARGO_TARGET_DIR`
passed first time.

So add one step before believing a red `main`: **grep the sibling worktrees
for the symbol in the error.**

```sh
grep -rl "<symbol from the failure>" ~/src/postio-worktrees/*/crates/
```

If it turns up in a worktree that is not yours, the error is about the build.

Three things follow, and the third is the one that costs time:

- **`cargo build --workspace` succeeding proves nothing about the next run.**
  It depends on what the other sessions happened to have built by then.
- **Building the failing crate alone is often clean**, because a narrower
  build reuses less. `cargo clippy -p postio-gtk` passed while
  `cargo clippy -p postio-app` failed on `postio-gtk`, minutes apart.
- **Do not go looking for the bug.** Check `pgrep -c 'cargo|rustc'` and
  whether the type in the error message exists in a *sibling worktree*
  (`grep -r <symbol> ~/src/postio-worktrees/*/crates/`). If it does, the
  error is about the build, not the code.

The reliable fix is a `CARGO_TARGET_DIR` of your own for that run. It costs a
full duplicate build, which is why it is not the default — but see the next
entry before choosing where to put it. Tracked as #178.

**Do not put a cargo target directory under `/tmp`.** It is a 16 GB *tmpfs* on
this box — RAM, not disk. A debug build of this workspace fills it, and what
happens then is not an out-of-space message from cargo: every subsequent
command in the session fails, `git` exits 128, and even `echo` cannot write
its output, which reads like the machine has died rather than like a full
filesystem. `df -h /tmp` is the one-line diagnosis and `rm -rf` the fix. If
you need a private target directory, put it under `/home`, which has room,
and delete it when you are done -- it is a full duplicate of the build.

**Resolved 2026-08-25 (#178): worktrees stopped sharing a target
directory.** The mechanism was never pinned down, but the effect was proven
twice (a `bool`-vs-`u32` type error against a declaration that was correct;
`CommandId::DetachComposer` missing against a `command.rs` that declares it),
and every diagnosis of it cost the wrong kind of time. The replacement:
each worktree builds into its own `target/` and `RUSTC_WRAPPER=sccache`
carries the third-party compilation cost once per machine — sccache keys on
exact compiler inputs, so it cannot serve a sibling's artifact. Numbers that
shaped the choice: the shared directory had grown to ~157 GB (du,
hardlink-inflated) against 99 GB free, so nobody "migrates" by copying —
new claims simply start private, the cache warms as sessions build what
they touch, and the legacy directory is reclaimed when the last session
sharing it is gone. `issue-claim.sh` now also creates `target/tmp` in the
fresh worktree, because `.cargo/config.toml` points TMPDIR there and its
absence made every `tempfile::tempdir()` in a fresh worktree fail with
NotFound — three sessions hit that in one day. The interim tell above stays
true for anyone still on the shared directory.

## Working in a shared git tree

These matter regardless of where work is tracked
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
