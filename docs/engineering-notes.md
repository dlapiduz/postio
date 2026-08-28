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

**Saved searches (#10) landed as add/list/activate; rename, reorder and
delete did not.** ARCHITECTURE.md §6 already settled what a saved search
*is* — `postio-config::FilterConfig`'s `[filters.<name>]`, `pinned = true`
meaning "show it in the sidebar" — so #10 was wiring, not design: nothing
read `FilterConfig` at runtime and the sidebar had no third section. What
shipped is deliberately the acceptance criteria and no more: `Ctrl+S` names
a save from the query text itself (`Config::save_filter`, a slug with `-2`,
`-3` on a collision), the sidebar's "Saved searches" section renders every
pinned entry and reports the query when one is picked, and
`Window::run_search` opens the box with it and runs it immediately rather
than waiting out the debounce a keystroke would. A user who wants to rename
one, or stop pinning it, edits `config.toml` by hand — `Ctrl+E` already
opens it — same as any `[filters]` entry before this issue.

The write path is intentionally decoupled from the `ConfigService` /
`LiveConfig` handle `postio-gtk/src/config.rs::install_at` already owns:
`Ctrl+S` calls `Config::load_from_path` fresh, adds the filter, saves, and
repaints the sidebar directly, rather than mutating the cached `service`
and routing through `ConfigService::apply`. The file watcher reaches the
same state a moment later and repaints again — redundant, and harmless,
because `set_saved_searches` replaces the list rather than appending to it.
Routing the write through `service` instead would have needed either
`ConfigService` to grow a save method or the closure holding it to move
into two places at once; reading fresh avoids both for one extra disk read
per save, which is not a path anyone times. This is also what closes half of
§6's "Schema built, not wired" note — the sidebar now reads `[filters]` live,
including a hand-edit while the app is running, through the same
`ConfigChanged::filters` the watcher already computed and nothing consumed
before this.

Rename, reorder and delete are real gaps, not omissions nobody noticed —
they were in the issue's own "What", just not its "Acceptance". File them
as their own issue(s) before calling saved searches "done" in any
roadmap sense.

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
`scripts/checks/check-crate-boundaries.py` counts a crate's *own* dev-dependencies,
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

**The file-transfer portal carries *references*, not bytes — a dragged-out
file that is deleted after the drop leaves the receiver with nothing, and no
error anywhere.** Established 2026-08-25 on #121 by driving
`org.freedesktop.portal.FileTransfer` by hand, because the mechanism decides
whether `paths::export_dir` is allowed to point at a cache directory.

What the portal actually does, in the order it happens:

- `StartTransfer` returns a key. GDK writes that key, and nothing else, as
  the payload for `application/vnd.portal.filetransfer`.
- `AddFiles(key, fds)` takes **open file descriptors**. It is tempting to
  read that as "the portal now has the content" and it is not: the fds
  identify the files, and `RetrieveFiles` hands the receiver back *paths*.
  Between an unsandboxed sender and an unsandboxed receiver they are the
  original paths, unchanged — no document-portal indirection at all.
- Delete the files after `AddFiles` and `RetrieveFiles` still returns those
  paths, still reports success, and every one of them is now missing. The
  receiver gets nothing and Postio believes the drop worked.

Two consequences worth keeping:

- **The dangerous window is after serialisation, not before.** GDK opens the
  fds *during* serialisation, so a file missing at that moment fails loudly
  with `Failed to open …`. The silent case is only ever "produced,
  serialised, then reclaimed before the receiver read it".
- **`export_dir` is `$XDG_CACHE_HOME/postio/drag` on purpose, and that is a
  live trade-off rather than a settled one.** Nothing in Postio deletes it
  today, so in practice the window never fires; the day something does — a
  startup sweep, a size cap, a "clear cache" verb — it fires silently.
  `crates/postio-app/tests/drag_out_portal.rs` pins the mechanism down so
  that change fails a test instead of a user's drop.

That test file is also the sandbox check: run unchanged inside
`flatpak run dev.postio.Postio` it proves the sandboxed path, which is the
only part of #121 a session on the host cannot answer.

## Storage, sync & search internals

**The search executor has two SQL plans, and which one a statement gets is not
a preference.** #408, and every number here was measured against the 120,000-
message `search_budget` bench.

- A query narrow enough to rank is **driven by the match**: walk the postings
  of both FTS indexes, look each hit up in `messages` by primary key.
- One too broad to rank is **driven by `messages`**, ordered by its own
  `(account_id, received_at)` index, asking each row "did you match?" through a
  correlated `EXISTS` with `rowid = m.id AND … MATCH ?`. That shape is what
  makes FTS5 answer with a docid seek — the plan says
  `VIRTUAL TABLE INDEX 0:=M5`, and the `=` is the rowid.

Getting it wrong is expensive in both directions, and every wrong turn was
tried: letting SQLite choose cost **49 ms** on a word matching 1% of the
corpus (it drove from `messages` and probed a co-routine it could not size); a
`GROUP BY` over the union cost **297 ms** on a common word (an aggregate must
materialise every match before anything runs); probing the union per row cost
**570 ms**; and `count` driven by `messages` cost **2.8 s** on a rare word.

Two consequences worth knowing before editing that file:

- **Adding a column to the candidate-pool statement can lose its plan.** The
  file already recorded this for the hydrate columns; it is equally true of
  correlated subqueries in the select list, which is why the broad path
  carries no `bm25` at all. That is deliberate rather than missing — the path
  is only taken when the match is too wide to rank, where bm25 is near-uniform
  and recency is the intended fallback.
- **`hydrate` touches no FTS table.** The scores ride out with the candidate
  pool. Re-asking the indexes for the scores of ids you already have is the
  297 ms mistake wearing a different hat.

**Free text scores are summed, body at half** (`BODY_SCORE_WEIGHT`). Before
the body left `messages_fts`, one bm25 over six columns did this implicitly:
FTS5's length normalisation put a term in a short `subject` well above the
same term in a long `body`. Two indexes have no shared corpus statistics, so
it is stated. Summing rather than taking the better of the two, so a message
matching in *both* ranks first. The tests assert that ordering, never the
number.

**A negated term must be excluded outside the match, not inside it.**
`("report") NOT ("spam")` asked of `messages_fts` is *true* for a message
whose "spam" is in its body — the metadata genuinely does not contain it — so
the message comes back from a query that explicitly refused it. Exclusions are
about the message and belong in the `WHERE`.


**Only a *missing* keyring entry mints a store key.** ADR 0014 Q3, landed in
#299 as `postio_session::store_key`. The store is encrypted under the key of
its first open, so minting a second one does not produce a second key — it
produces a mailbox nobody can read. `SecretError::Locked`, `Timeout` and
`Backend` all mean "the keyring did not answer", never "there is no key", and
a service that treated any of them as a first run would silently destroy a
store the moment a keyring was slow. `NotFound` is the only first run. An
*empty* entry is minted over, and that is not an exception: nothing can have
been encrypted under an empty key.

A **corrupt** entry — text that is not 64 hex characters — is refused and left
alone. Corrupt is a store that cannot be opened; replaced is a store that can
never be opened again.

**The `derive_key` contexts in `postio_storage::key::Purpose` are on-disk
format.** `"postio db"`, `"postio blob content"`, `"postio blob id"`. Change
one and every existing store's subkey changes with it, which is to say every
existing store stops opening. `tests/store_key.rs` pins them for that reason
rather than for tidiness.

**Nothing renders a key.** `StoreKey` and `Subkey` have hand-written `Debug`
impls that print `<redacted>`, and the material is behind `expose()`/`to_hex()`
so every use is short and obvious in review. The failure mode this guards is
not somebody logging the key on purpose — it is a `#[derive(Debug)]` on a
struct that happens to hold one, which turns any `dbg!`, any `?err` and any
panic message into a full compromise of the store.


**One `TokenSource` per account, and never a second.** ADR 0006 Q5, made real
in #194. The composition root (`postio_session::engine::start`) builds one and
hands *that instance* to the account's IMAP pool and to `EngineParts::tokens`,
which is what `SmtpContext` sends with. A second source of the same type,
constructed anywhere downstream, compiles and looks identical and is the bug:

- a rejection seen while fetching is invisible while sending, so the two sides
  disagree about whether the credential is any good;
- on a provider that rotates its refresh token on every use — Google and
  Microsoft both do — two simultaneous refreshes each invalidate the other's
  result, and the account degrades to one working token lifetime;
- the single-flight coalescing is per source, so two sources means two flights
  and the stampede it exists to prevent.

`EngineParts` deliberately has no `secrets` field beside `tokens`: a struct
offering both is a struct where the wrong one gets used. A password account is
a `TokenSource` too (`StoredPasswordSource`), which is the whole point of the
seam — the composition root decides what kind of credential an account has,
and nothing downstream asks again.

**What a refused credential means is decided in exactly one place**,
`postio_imap::auth::with_credential`: invalidate, ask once more, and *do not
retry at all* if the source hands back the same bytes. That last clause is the
one that goes missing when the paragraph is written twice — and without it a
wrong password is an endless pair of round trips. The pool and the SMTP send
both call it; a third place that meets a server with a credential should too.


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

**Who wins between a queued local flag and the server's copy, and until when**
(#317). Until the operation carrying it settles, the **local flag wins**; after
that the server is authoritative again.

`MessageRepository::upsert_batch` writes the fetched message wholesale, flags
included. A `CHANGEDSINCE` pass that runs before the drainer has pushed a flag
therefore wrote the server's still-stale copy back over it: the dwell marked a
message read, the row went bold again a moment later, and the queued operation
eventually set a `\Seen` whose effect nobody could see. Reported as "reading a
message does not mark it read", which is not what was happening.

The fix is a **merge, not a skip**, and that distinction is the point.
`shadowed_by_pending_operation` answers the undrained *move* by dropping the
message from the batch — as far as the user is concerned it is not in that
mailbox, so nothing the server says about it there is news. A flag cannot be
handled that way: the row is still there, and the subject, the size and what
parts it has are all worth taking. So `unacknowledged_flag_changes` replays
just the queued `set_flags`/`clear_flags` over the flags the server reported,
in the order the user made them, and everything else lands unchanged. A flag
somebody set on their phone still arrives while a *different* flag of yours is
mid-flight.

The bound matters as much as the rule: only `pending` and `in_flight`
operations protect anything. Once one is `done` the server can mark that
message unread again, which is what has to happen when it is read and then
marked unread somewhere else. A protection that outlived the operation would
make the flag permanent.

Two traps if this is ever revisited:

* **A test here goes vacuous very easily.** The server must have a *reason* to
  report the message, or an incremental pass fetches nothing and there is no
  overwrite to survive. The first version of `resync.rs`'s test passed against
  the unfixed code for exactly that reason. Make another client change a
  *different* flag: that bumps `MODSEQ`, the message comes back carrying its
  whole flag set, and that set is missing the one in flight.
* **This is the same family as #289 and #368** — local-first intent lost across
  a gap that each half handles correctly on its own terms. When adding a new
  operation type, ask what an unacknowledged one of them should do to a resync
  that has not heard about it.

**`busy_timeout` is a retry loop, not a queue — interactive writes need a
permit** (#425). SQLite takes one writer at a time even under WAL, and
`PRAGMA busy_timeout` settles a collision by making the loser sleep and try
again, backing off up to 100 ms. There is no ordering in that and no
fairness: each retry is a fresh race. During a first sync the sync lanes
commit write units back to back with essentially no gap between one `COMMIT`
and the next `BEGIN IMMEDIATE`, so a keystroke's write loses that race over
and over. Measured: an archive took **1.8 seconds** to write one row, with the
connection pool almost idle the whole time (`Pool::get` returned in two
microseconds).

Two things that look like fixes are not. A **bigger pool** does nothing — the
pool was never the contended resource. **Shorter background transactions** do
almost nothing either: cut to an eighth of their size, the same write still
took half a second, because the number of races to lose grew as fast as each
one shrank. This is the trap worth not re-deriving; both were leading
hypotheses on #425 and both were wrong.

What works is `postio_storage::WriteGate`: an application-level queue in front
of SQLite's write lock, with two priorities. A background writer never
*begins* a write while an interactive one is waiting, so a person waits at
most for the background unit already in progress — and `initial::WRITE_UNIT`
(25 messages, ~8 ms) is what keeps that bound inside the interaction budget.
Both halves are load-bearing: the gate without a bounded unit would make a
keystroke wait out a whole 200-message batch, and a bounded unit without the
gate is the "shorter transactions" non-fix above.

Two rules for anything that writes. **Take the pooled connection first, then
the permit** — a permit-holder blocked in `Pool::get` can be waiting on a
connection held by a permit-waiter. And **one permit at a time per thread**:
the gate is not re-entrant, so nesting deadlocks against itself, which is why
`Actions` takes its permit in `connect()` (one per write unit) rather than
around `run`, where the verbs that resolve a target before acting on it would
nest.

A writer that takes no permit is invisible to the gate and is starved exactly
as before. That is not a safety bug, but it is the first thing to check if
"the UI froze during a sync" ever comes back. The interactive writers today
are `postio_session::actions` and `postio_app::compose`.

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

**A received attachment's bytes are in `Attachment::blob_id` once somebody
has opened it, and not before.** For the whole life of this project that
column was filled only on the way *out* — `postio_app::compose` putting a
file the user attached into the blob store — so it was `None` for every
message that had ever arrived from a server and `parts::Node::downloaded` was
correspondingly always false. [ADR
0017](decisions/0017-backfill-cost-attachments-memory-disk-encryption.md)'s
payload axis (#377) gave it its first receive-path writer.

What that means for anything reading a part:

- **`node.downloaded` is now true for received mail, and means it.** The
  attachment chip can honestly offer "download" versus "open", and inline
  `cid:` resolution has a field to read that is actually set. It is still
  false until the part is fetched, which for `AttachmentPolicy::OnOpen` — the
  default — is when the user opens or saves it.
- **`postio_app::reading::part_bytes` is the worked example**, and it has
  three cases rather than one: the part's own blob, the raw message when
  there is one, and a fetch when there is neither. Do not re-parse a raw blob
  without checking `blob_id` first; and do not assume a raw blob exists, because
  under the text axis the background lane never stores one.
- **The blob id is taken on the *decoded* payload**, not on the base64 that
  came off the wire. That is what makes two messages carrying the same file
  share one blob, and what makes a part fetched eagerly and the same part
  fetched on open land identically instead of twice. Anything that stores a
  payload must decode first (`postio_model::mime::decode_entity`).
- **`attachments.part_headers` is what makes a section decodable.** `BODY[2.1]`
  returns encoded bytes and none of the part's own headers; `BODYSTRUCTURE`
  reported the type and the transfer encoding at header-sync time and this
  column keeps them. A row without it — synced before migration 0010 — cannot
  be fetched by section and falls back to a whole-message fetch.

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

**The threaded list windows over messages, not over `threads` (#306, #307).**
ADR 0015 Q1 describes the folder list as "a window over `threads`... joined to
its newest message in the current folder". It is not, and the difference is
load-bearing rather than stylistic. The folder window is over **messages**,
keeping the row that is newest in its own thread within the folder:

```sql
FROM messages rep
WHERE rep.mailbox_id = ?1 AND rep.deleted_locally = 0
  AND NOT EXISTS (SELECT 1 FROM messages newer
                   WHERE newer.mailbox_id = ?1 AND newer.deleted_locally = 0
                     AND newer.thread_id IS NOT NULL
                     AND newer.thread_id = rep.thread_id
                     AND (newer.received_at, newer.id) > (rep.received_at, rep.id))
ORDER BY rep.received_at DESC, rep.id DESC
```

Two reasons, and the first is the one that matters:

- **A window over `threads` hides mail.** `messages.thread_id` is nullable and
  nothing guarantees it is set — `postio-sync`'s send path threads with
  `let _ = ...thread(&message)` and discards the failure. Every such message
  is simply *absent* from a list built over `threads`: no error, no empty
  state, mail in the store and not on screen. Under this shape an unthreaded
  message is a conversation of one and cannot disappear. `ThreadListRow::id`
  is `Option<ThreadId>` for exactly this.
- **It is flat by construction rather than by measurement.** The window walks
  `idx_messages_list (mailbox_id, received_at DESC, id DESC)` — the same index
  the message list uses — so "page k of threads costs what page k of messages
  costs" stops being a claim to benchmark and becomes the same query plan.
  Everything the conversation contributes (total size, unread here, flagged
  here) is a correlated subquery per row of the page, seeking
  `idx_messages_thread_mailbox` from migration 0012.

Every property the ADR actually decided is preserved: the collapse is
store-side, one row per conversation, flat paging, aggregates scoped to the
folder while the count stays the whole conversation. What changed is the table
the window walks. It also measures slightly faster (`store_reads`: 897us at
1k, 1.07ms at 100k, 1.09ms ten pages down).

**The account-scoped list still windows over `threads`**, because there is no
folder to be newest within and the ADR's shape is right there.

**Drafts does not thread**, which ADR 0015 did not have to say because it was
writing about reading mail. A draft is a document you are writing; two drafts
answering the same conversation would collapse into one row with no way to
open the other. `SqliteStore::lists_conversations` is the one place that is
decided, so the frontend never holds a second opinion about it.

**The tracker count is a size heuristic, and it under-counts on purpose
(#174).** `postio_body::sanitize` splits what it strips into ordinary remote
images and likely trackers. The rule the maintainer settled (2026-08-25) is
the whole rule: **an `<img>` whose own declared dimensions are ≤ 2px in
either axis, or which declares itself hidden (`display:none`,
`visibility:hidden`), is a likely tracker.** Nothing reads the host or the
path.

Two things follow, and both are deliberate:

- **A beacon that declares no size is counted as a picture.** Silence is the
  ordinary case — most senders declare nothing — so reading it as a beacon
  would label every plain image a tracker. Under-counting is the safe
  direction here in a way over-counting is not.
- **Never add a domain or path rule to "improve" it.** A list of known
  tracking vendors is exactly the provider hard-coding CLAUDE.md forbids, it
  rots from the day it is written, and it mislabels real pictures: the
  corpus fixture `html-tracking-pixel-remote-images.eml` serves *all three*
  of its images from hosts with `tracker` in the name, two of which are a
  product shot and a logo. That fixture exists to make the point.

The count only ever changes the parts panel's **wording** ("3 remote images
and 1 likely tracker"). Both kinds are blocked identically, so a beacon the
heuristic misses is still never fetched — being wrong here costs a noun, not
a request. That is why the panel says "likely", and why revisiting this
wants data about real mail rather than a cleverer rule.

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


**A pending operation shadows what the server says about its target, and
local-first is the reason.** `upsert_batch` drops a second set from its batch,
on the same argument and in the same place as the draft copies above: any
message with an undrained `Move` or `Delete` out of the mailbox being written.
#368.

Archiving is local-first — SQLite write, enqueue, emit, repaint — so between
the keystroke and the queue draining, the server still lists the message where
it was. A resync of that mailbox in that window fetches it, looks for a row
under `(mailbox, UIDVALIDITY, UID)`, finds none *because the row is in Archive
now*, and inserts a fresh one. The message the user just archived is back in
the inbox. It leaves again by itself once the queue drains, which on a link
that is down is indefinite, and nothing in the interface explains either
event. Measured on the e2e suite as the difference between two runs of
identical code at the same point: `store=[1@mb2 2@mb1 3@mb1]` when the drain
won the race, `store=[1@mb1 2@mb1 3@mb1]` when a resync did.

The end state was always right — a later resync removed it again — so this
reads as flakiness rather than as a bug, which is how it survived. #364 was
the test reacting to it.

**The rule it qualifies:** local-first is not only "the UI never awaits the
network". It is that *the local answer is the one the user sees until the
server agrees*. A background resync overwriting a local decision that has not
been carried out yet keeps the first half and breaks the second.

Two details are load-bearing:

- **It keys on the queue row's snapshot, not on the message row.** The local
  half of a move nulls the row's `uid`/`uid_validity` in the same transaction
  that enqueues, so by resync time the queue row is the only thing that still
  remembers the server coordinates. That snapshot exists because of #289 — a
  different bug with the same cause.
- **The shadow lifts the moment the operation settles**, on `done` *or*
  `failed`. One that outlived a move the server refused would hide the message
  for ever, which is worse than the resurrection it prevents. Only `pending`
  and `in_flight` shadow anything.

`move` and `delete` are the only operations that qualify, because they are the
only ones whose queue row names a mailbox the message is *leaving*
(`Operation::mailbox()` returns `from` for both). A flag change moves nothing,
and an `append` puts a message into a mailbox rather than taking it out.


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

**An integration test must assert the thing it names, not a number that
happens to move when it happens** (2026-08-26, #364). `e2e.rs`'s delivery
phase asserted `n_items() == shown_before + 1`, and failed about one run in
eight for the life of the suite. Because `issue-land.sh` runs it on the way to
every merge, that is somebody's landing rejected most days, and it looks
exactly like a regression in whatever they were working on — two of mine were,
and establishing otherwise took eight runs on `main` against eight on the
branch.

The count was never the claim. `shown_before` was snapshotted straight after a
phase that waits for the *server* to have the archived message, which says
nothing about the local row: the departure from INBOX arrives separately, and
can even be undone and redone while the queue drains (#368). So the baseline
was 3 on some runs and 2 on others, for identical correct behaviour, and only
one of those makes `shown_before + 1` reachable. It now finds the delivered
row by its `Message-ID` and asserts *that message* is in the list model — the
sentence the phase was always trying to say, and one no timing can make
unreachable.

The general rule: **when a test waits for a total, ask what would have to be
true for that total to be wrong while the software is right.** A count is
shared by every row, so any other row moving underneath it corrupts the
measurement silently. An identity is not. Prefer waiting for the specific
message, widget or row the phase is about, and reach for a count only when the
count really is the property — and then settle it against the store rather
than against whatever the list happens to be showing.

Two supporting habits this cost a day to learn. **Instrument before
theorising**: three plausible explanations were wrong, and a dump of
`(id, mailbox_id)` at fixed checkpoints settled it in one run. **And a
long-running integration test needs a way to see inside it** — `e2e.rs` had
no tracing subscriber at all, so `POSTIO_LOG` did nothing there; it has one
now, off unless the variable is set.

**A search-index column no trigger can compute needs an owner at the point the
data lands, and a pass that heals what that owner missed** (2026-08-25, #327).
`search_documents` is filled two ways: sender, recipients, subject and
attachment filenames come from SQL triggers on their own tables and were
always correct, while `body` can only ever be written by a caller, because the
text lives in the blob store. `index_body` was written, unit-tested and
benched — and no production code ever called it, so that one column was empty
on every message in every real store for the life of the project. It presented
as "search is inconsistent" rather than "search is broken", which is the
expensive part: one search box gave different answers to the same word
depending on whether it was in a subject or a body, with nothing on screen to
say which kind of question had been asked.

The shape the fix settled on generalises. The write goes where the data
lands and nowhere else — `postio_sync::backfill::fetch_body` is the single
funnel every body passes through, background backfill and the interactive
fetch of whatever the user just opened alike — and it goes *after* the
storage commit point, so a crash leaves a body that is local but unindexed
rather than an index entry for bytes that are not here. That residue is then
swept by `postio_session::index_local_bodies`, which asks
`messages_missing_body_text` for rows whose body is local and whose indexed
text is empty: it costs one query that finds nothing on a store that is caught
up, so it can run on every start, and it is spawned on the runtime rather than
called on the startup path because its first run over an existing archive
reads a blob per message.

**The backfill horizon is "all of it, in batches, newest first"** (2026-08-25,
#318). How far back Postio pulls bodies unprompted is a real product decision —
it is time, disk and somebody's data plan — and for the life of the project it
was made by accident. `postio-app` seeded 200 bodies per folder at startup and
nothing ever called `seed_backfill` again, so a *cap* was doing the work of a
*horizon*: when that first batch drained the background lane had nothing to do
for the rest of the process, every message below the newest 200 of its folder
waited to be opened, and the status line's denominator was the size of the
seed rather than the work outstanding.

The horizon chosen is the whole account, reached in batches: the engine tops
the queue up whenever it has genuinely drained, INBOX first by
`sync_priority`, and re-seeds a folder whose sync changed something. The
throttling is left to the policy that already existed for it —
`pause_on_metered`, `pause_when_active`, `max_body_bytes` — rather than to a
smaller number, because those are the knobs that know *why* they are pausing.
`BackfillPolicy::seed_batch` (200) is what a batch is; it bounds how much is
held in memory and how much can sit in front of an interactive fetch, and it
is no longer a horizon in disguise.

Three things make the walk terminate, and all three are load-bearing:

- **`body_state` is the cursor.** A body that lands becomes `full` and leaves
  `needing_backfill`, so each seed naturally asks for the *next* batch. Nothing
  remembers a position, which is why a restart resumes correctly rather than
  starting over.
- **`Backfill::set_aside`** holds what this session will not offer again: a
  message over `max_body_bytes`, and one whose fetch failed or found nothing.
  Both stay `body_state <> 'full'` for ever, so without this a drained queue
  would re-queue a failing message immediately and retry it at the speed of the
  engine's own loop. It is cleared on reconnection, and lost on restart, which
  is the retry a transient failure gets.
- **`seed` pages past a batch it could not use.** A folder whose newest
  `seed_batch` messages are *all* over the cap would otherwise answer the same
  unusable rows for ever and the walk would never start. `seed` reports what it
  actually queued, not what it read, which is what the top-up loop stops on.

`State::backfill_covered` is the latch that keeps a covered account from
re-asking every folder on every loop iteration; a sync that wrote something and
a link coming up are the two things that clear it.

**`Document::to_text` and `Document::to_search_text` have opposite rules about
link addresses, on purpose.** `to_text` spells a link as `label <href>`,
because a quoted reply that drops the address leaves "click here" pointing at
nothing. An index must not: a message that links to `tracker.example` does not
say "tracker.example" anywhere a reader can see, so indexing the address makes
that message a hit for a word it never contained — and one shortener would
answer for every campaign that used it. Same for the `[image]` placeholder,
which would make every message carrying a picture a hit for "image". Reach for
`to_search_text` whenever the destination is a haystack rather than a reader,
and go through `postio_index::index::index_body_of` rather than `index_body`
with text you extracted yourself — the rule "raw markup must never reach this
column" is kept by the crate that owns the column precisely so the next caller
cannot forget it.


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
guard is now static: `scripts/checks/check-runtime-crossings.py` refuses any `.await`
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
as flakiness — see #41.

The failure is usually quieter than an abort, which is why this note sat here
naming two files that "still have this shape" while nothing ever went red.
Every GTK test opens with the same guard — `if adw::init().is_err() ||
gdk::Display::default().is_none() { eprintln!("skipping: no display");
return; }` — written for a headless box, and it cannot tell that case apart
from "another thread in this process got GTK first". So the losing test
returns before asserting anything, and libtest calls that a pass. Measured on
`gtk_composer_autosave.rs` (#355): three consecutive runs took 1.88s, 1.89s
and 0.42s, the fast one being the run where the debounce test — the one with
real timing to prove — was the half that evaporated. *Which* of the two
vanishes is thread scheduling, so it is a fresh coin flip every run, and that
file had been reporting `ok` for both since the day it was written.

`gtk_composer_autosave.rs`, `gtk_finder.rs` and `gtk_settings.rs` (this note
missed the third) are now cases in `tests/gtk_suite/`, and
`scripts/checks/check-one-gtk-test-per-binary.py` refuses a new one — a rule
written down here plainly did not hold on its own. A file may still carry
several tests when only one needs a display: `gtk_shell.rs` builds a window
in one and parses the stylesheet as text in the other, and the check is
written to allow exactly that.

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

`scripts/checks/check-no-gtk-init-in-unit-tests.py` enforces this in CI and in
`issue-land.sh`. It reads `#[cfg(test)]`/`#[test]` spans rather than grepping
for `adw::init`, so production code initializing GTK is untouched; the only
way past it is a `POSTIO-GTK-INIT:` line in the file arguing why the test
cannot move. Its own failure modes are exercised by
`scripts/tests/test-check-no-gtk-init-in-unit-tests.py`, since the tree is clean and
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
  has genuinely set one. `scripts/tests/test-issue-land-target-dir.py` holds it
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

**A reader test with no allow-list override reads and writes the real
machine's remote-image allow list.** `Reader::new` calls
`RemoteImageAllowList::load()`/`::path()`, which resolve through
`glib::user_state_dir()` — the actual `$XDG_STATE_HOME` of whatever process
runs the test, not a scratch directory. Two sessions diagnosed this
independently from opposite ends; the full account, and the fix that landed,
are under "A test that builds a `Window` reads the developer's own
`$XDG_STATE_HOME`" above. In short, `gtk_reading_pane.rs` failed with "the
reader held a remote image back and the panel never heard about it" because
its sender was already on the real, stale allow list, so nothing was blocked
and the count was 0.

Two ways to give a test a list of its own, and they are not equivalent.
Setting `XDG_STATE_HOME` before `adw::init()`, the way
`gtk_composer_recipients.rs` does, moves *every* state file at once and is
right when a test touches several. `Window::set_allowlist_path` (#215) moves
only this one and takes no dependency on process-global environment or on
running before GLib has cached the state directory — which is why it is what
`gtk_reading_pane.rs` uses. Reach for the env override when you want the whole
state directory; reach for the seam when you want one file.

**A dropped `JoinHandle` detaches a `spawn_blocking` task; it does not abort
it — so cancel the *socket*, not the task.** `tokio::select!` losing a race
drops the losing future, and for `spawn_blocking` that means the blocking
thread keeps running with whatever socket it opened. Nothing in tokio can
interrupt a thread parked in `read`. The autoconfig probe raced a
`CancelToken` against its steps for a year and cancelled nothing but its own
waiting (#57, found by the `postio-iigq` audit).

The way out, where the blocking work is somebody else's crate: take the
stream. `io-pim-discovery`'s `DiscoveryStream` is `Read + Write` and nothing
more, and both of its std clients expose `with_factory(scheme, ..)` — so
`postio-imap`'s `discovery::transport` hands them a wrapper that checks the
token before every read and write and fails with
`io::ErrorKind::ConnectionAborted`. The detached task then unwinds through
the client's own error path and drops the socket. The protocol stays
upstream's; the stream becomes ours.

Three things that are easy to get wrong here:

- **Never report `ErrorKind::Interrupted` for a cancellation.** It means
  "retry me" throughout `std` — `read_to_end` and friends loop on it — so a
  cancelled stream reporting it spins forever instead of stopping. Exactly
  backwards, and it looks right. `ConnectionAborted` is the one nothing
  retries.
- **A check between reads does nothing while a read is parked**, so the
  token needs a deadline beside it. `pimalaya-stream`'s default `Retry` is
  60 seconds *per read*, and the DNS path armed no socket deadline at all —
  hence `DISCOVERY_IO_TIMEOUT`, and `TcpStream::connect_timeout` in place of
  `connect` on the DNS side. `Stream::connect_tcp`/`connect_tls` still take
  no connect deadline, so the HTTPS connect phase keeps the OS default.
- **A cancellable transport that nobody cancels changes nothing.** The
  composition root was passing `Probe::run` a `CancelToken::new()` and
  dropping it, so no probe in the shipping application was cancellable
  whatever the layers below could do. `ProbeCancellation` in
  `postio-app/src/onboarding.rs` is the half that does the cancelling.
## Fuzzing the hostile-input pipeline

Every message Postio parses is attacker-controlled, and the `.eml` corpus —
excellent as it is — only contains inputs somebody thought of. `fuzz/` is
three libFuzzer targets for the inputs nobody thought of: `parse_message`
(raw bytes through `postio_model::mime`), `sanitize_html` (bytes through
`postio_body`'s incoming sanitizer), and `parse_query` (a string through
`postio_search`'s parser). Added by #147.

**Running one.**

```bash
scripts/fuzz.sh parse_query                       # until you stop it
scripts/fuzz.sh parse_message -- -max_total_time=300
scripts/fuzz.sh --list
```

Use the script rather than `cargo fuzz` directly. Two things stand between a
shell on this workstation and a working fuzz run, and neither error names its
cause:

- **libFuzzer needs nightly, and `RUSTUP_TOOLCHAIN` beats a toolchain file.**
  `fuzz/rust-toolchain.toml` pins a dated nightly, but this machine exports
  `RUSTUP_TOOLCHAIN` from `~/.config/mise/config.toml`, and rustup reads the
  environment first — so the build gets 1.98.0 and fails with *"the option `Z`
  is only accepted on the nightly compiler"*, which reads like a missing
  toolchain and is a winning environment variable. Same trap as the one the
  landing gates clear; see the `RUSTUP_TOOLCHAIN` entry above.
- **rustup picks a toolchain file by the working directory, not by the
  manifest.** `cargo fuzz --fuzz-dir fuzz` from the repository root still gets
  the *root's* pin. The script `cd`s into `fuzz/` for exactly this reason.

**Why `fuzz/` is its own workspace.** Nightly and `-Z sanitizer=address` must
not leak into the pinned build, and the instrumented build is expensive enough
that `cargo test --workspace` must never touch it. `scripts/checks/check-toolchain-
pinned.py` was taught that a *dated* nightly (`nightly-2026-08-24`) is a pin
rather than a float — it names one compiler as exactly as `1.98.0` does, and
there is no stable spelling of what libFuzzer needs. A bare `nightly` is still
refused, and so is `stable`.

**The corpus is generated, not committed.** `scripts/fuzz-seed.sh` fills
`fuzz/corpus/<target>/` from `crates/postio-model/tests/corpus/*.eml` and from
`fuzz/seeds/`. The `.eml` fixtures stay in one place, where `/add-fixture`
maintains them, rather than being copied into the tree twice and drifting.
Seeding matters more than it sounds: from random bytes a fuzzer will never
generate a valid MIME boundary, so an unseeded `parse_message` run explores
almost nothing.

**What it found in its first hour**, which is the argument for having it:

- **Remote-image blocking was case-sensitive.** `postio_body`'s `is_remote`
  compared schemes with `starts_with("https://")`, but RFC 3986 §3.1 makes
  schemes case-insensitive and WebKit resolves them that way. A tracking pixel
  spelled `HTTPS://` was left in the document *and* reported as nothing held
  back — so the reader fetched it and the badge said zero. A privacy promise a
  sender defeats by holding shift. Fixed in #147.
- **`save_name` did not strip control characters.** A NUL reaches an
  attachment filename both from a literal `filename="a\0b.txt"` and from one
  base64'd inside an RFC 2047 encoded word; the name then goes to
  `FileDialog::initial_name`, which converts a `&str` to a C string. Fixed in
  #147.
- **`mime::parse` is not infallible.** `mail-parser` panics on a malformed
  multipart and the panic comes out of ingest. #277 — see below.
- **And one bug in the fix for the first one**, 71 executions after it was
  written: comparing a scheme with `value[..8]` panics when byte 8 lands
  inside a multi-byte character, and an attribute value is attacker-controlled
  text that can start with any character at all. `str::get(..n)` is the
  spelling that cannot. Worth noticing as a pattern — a hostile-input fix is
  itself hostile-input code, and the fuzzer is the thing that will tell you.

**Triaging a find.** The reproducer lands in
`fuzz/artifacts/<target>/crash-<hash>`. Shrink it first — `cd fuzz && cargo
fuzz tmin <target> artifacts/<target>/<file>` — then read it. **Do not paste
it into an issue.** For `parse_message` it is a whole message, mutated out of
the corpus but message-shaped, and this repository is public; the CI job
uploads it as an artifact and deliberately never prints it, for the same
reason `check-no-personal-data.py` redacts by default. Describe the shape and
add a fixture through `/add-fixture` if the input deserves to become a
permanent test.

**Then ask which layer the property belongs to.** All three of the first
findings were the checker being wrong rather than Postio, and all three were
still worth having:

- *`javascript:` survived the sanitizer.* It had not. `&#x6a;avascript&colon;`
  decodes to the literal text `javascript:` inside a `<p>` — visible, inert,
  not a URL. So the scheme check moved into the per-attribute URL pass, where
  a scheme name means something. Tag names stay a whole-document scan, and
  soundly: `<` is escaped to `&lt;` everywhere in text, so a literal `<script`
  in the output can only be an element. The same input also *confirmed*
  something worth knowing — an entity-encoded `&#104;ttps://` in a `src` is
  decoded before the attribute filter sees it, so blocking is not dodged by
  spelling.
- *A remote `src` survived a blocked render.* Also text. The mutation had
  deleted the `<` before `img`, leaving `img src="https://..."` sitting in a
  paragraph as escaped, inert characters. Same root cause as the first: a
  substring scan cannot tell an attribute from text that resembles one. The
  scanner now walks `<...>` interiors only, which is sound *because* of what
  the sanitizer guarantees — ammonia escapes `<` and `>` everywhere in text
  and in attribute values, so in sanitized output an unescaped `<` starts a
  tag. Note that this makes the scanner correct **only** on already-sanitized
  input, which is the only thing it is called on.
- *A path separator in an attachment filename.* Also correct behaviour:
  `mime::parse` reports the filename the sender wrote, and laundering it is
  `postio_gtk::parts::save_name`'s promise, not the parser's. Asserting it in
  the fuzz target demanded that the model launder data it exists to report
  faithfully. **The find still paid for itself**: it sent someone to read
  `save_name`, which stripped separators and dots but not control characters —
  and a NUL reaches a filename both from a literal `filename="a\0b.txt"` and
  from one base64'd inside an RFC 2047 encoded word. That name goes to
  `FileDialog::initial_name`, which converts a `&str` to a C string on the
  way. Fixed in #147, with the tests beside `save_name` where the promise is.

The general rule that came out of it: **a fuzz property must be a promise the
function under test actually makes.** When a find looks like a bug, the first
question is not "where is the bug" but "which layer promised this", and the
answer is often a layer the target does not call.

**`parse_message` is known red, and deliberately.** It finds #277 within
minutes: `mail-parser` 0.11.8 panics on a malformed multipart
(`Invalid part ID, could not find multipart`), and the panic comes straight
out of `postio_model::mime::parse`, which the module documents as infallible.
That is a remotely-triggerable client crash — ingest runs on bytes from the
server, during sync, before anyone opens anything — and it is exactly what
this target was built to find. It is **not** worked around in the target: a
fuzz target taught to ignore a real find is worth less than no target at all.
Containing it needs a decision about what the application shows for a message
that did not parse, which is why #277 carries the design options rather than a
patch. Until that lands, a red `parse_message` leg means #277, not a
regression you introduced.

**The scheduled job is paused.** `.github/workflows/fuzz.yml` is
`workflow_dispatch`-only, for the reason `ci.yml` and `bench.yml` are: a
weekly run spends this private repository's limited free minutes whether it
finds anything or not. Uncomment its `schedule` when the repo goes public.
Until then, running it by hand — or locally — is what happens.

**A panic your fuzzer finds in a dependency may be a `debug_assert!`, and
then it is not in the shipped product at all.** #277 was filed as a remote
denial of service: a malformed multipart panics `mail_parser` inside
`mime::parse`, which runs on bytes off the socket during sync, before anyone
opens anything. The panic is real. Measured both ways against
`mail-parser` 0.11.8, with a 144-byte reproducer:

| build | `debug_assertions` | `mime::parse` |
|---|---|---|
| dev, test, CI, fuzz | on | **panics** |
| release (what ships) | off | returns, recovering nothing usable |

The site is `debug_assert!(false, "Invalid part ID, could not find
multipart.")` at `parsers/message.rs:485`. `debug_assert!` compiles out
whenever `debug-assertions` is off, which is the default for
`[profile.release]` and not overridden here — so the shipped binary never had
the crash. **cargo-fuzz builds with debug assertions on**, which is why the
fuzzer found it and why the `parse_message` leg was red.

Two lessons, and the second is the expensive one:

- **Check the panic site before believing the severity.** The line number in
  the backtrace is enough: open the dependency's source. A `debug_assert!` and
  a `panic!` read identically in a fuzz report and mean completely different
  things about what users experience.
- **A signal that only exists in debug builds cannot drive a user-facing
  state.** The first plan for #277 was to catch the unwind and show the reader
  "this message could not be parsed". In release there is no unwind to catch —
  `mail_parser` returns a thin, ordinary-looking message — so that state would
  have been unreachable in the only build that ships. What release actually
  does is show `Absent::Empty`, "genuinely has no text or HTML part", which is
  false; saying otherwise needs a signal upstream does not give, so the fix
  stopped at containment.

`catch_unwind` is still right regardless: the module documents `parse` as
infallible, and it has to hold against the next such bug too. It moved out of
`parse_inner` to wrap the whole function as `try_parse`, so the outcome can
reach a caller that wants to log it — catching inside meant `parse_inner`
always returned a value and nothing downstream could tell a contained failure
from an ordinary empty message. `postio-sync`'s backfill is the caller that
cares; the reading pane deliberately is **not**, for the reason above.

**Fixing a crash uncovers the crash behind it, and the next one was ours.**
With the panic contained, `parse_message` ran further into the same inputs and
found a stack overflow — `postio_model::mime`'s own `part_paths` walked the
MIME tree by recursing once per level, and nesting costs a sender nothing:
`multipart/mixed` inside `multipart/mixed`, as deep as they care to type.

That one had none of the previous bug's mitigations. It is not a
`debug_assert!`, so it is in the shipped build; and a stack overflow is a
`SIGSEGV` rather than an unwind, so `catch_unwind` cannot contain it and
neither can any caller. It was the real remote denial of service the issue had
been filed about, hiding behind a bug that only looked like one.

The walk is an explicit worklist now, not a depth limit. A limit is a number
somebody has to be right about — too low and a legitimately baroque forwarded
thread loses its attachments, too high and the crash is still reachable —
whereas iteration has no such number, and the heap it uses is bounded by a
message already in memory. **When input decides how deep a recursion goes,
that is the input deciding how much stack you use**; prefer a worklist to a
`fn` that calls itself anywhere on the ingest path.

Two process points from this: **re-run a fuzz target after every fix** rather
than assuming the target is done, and note that the target could not see the
second bug until the first was gone.

**`fuzz_target!` aborts on *any* panic, including one you caught.**
libfuzzer-sys installs a panic hook that calls `process::abort()`, on purpose:
aborting before the stack unwinds is what lets libFuzzer tell one crash from
another. The side effect is that `std::panic::catch_unwind` never runs inside a
fuzz target — the hook fires first — so `parse_message` kept reporting a
contained panic as a crash after it had been fixed.
`postio_fuzz::allow_contained_panics` replaces the hook with one that does
nothing and lets the unwind proceed. **An uncaught panic is still a crash**,
which is the part to check rather than assume: it unwinds out of the target
closure into libFuzzer's `extern "C"` frame, and Rust aborts rather than unwind
across that boundary. Verified by injecting a failing assertion and confirming
libFuzzer still reported "deadly signal" — do that again if the hook handling
is ever touched, because the failure mode is a target that reports nothing
forever.

## Coverage and mutation testing

Two tools from the #103 quality survey, both wrapped rather than run
directly, both entirely local (no upload, no third party sees a number —
this project's privacy posture applies to its own tooling, not just the
product). Added by #98/#99.

**Coverage.** `cargo-llvm-cov`, gated per crate against
`scripts/coverage-floors.json`, never one workspace percentage:

```bash
scripts/coverage.sh                # every crate the floors file names
scripts/coverage.sh postio-model   # just one
```

A floor is a ratchet, seeded at whatever a crate measured the day this
landed — not an idealized target, the same reasoning `docs/keybindings.md`
and `docs/config.md` use for their own generated baselines. Raising one is a
deliberate, reviewed change; the file's own comment says why `postio-gtk`
gets no floor at all rather than a low one. This job runs in `ci.yml` and
does gate a PR — see the file's own comment for why coverage, unlike
mutation testing below, is cheap enough to run on every push once CI is
unpaused.

**Mutation testing.** `cargo-mutants` over `postio-model`, `postio-search`,
`postio-config` and `postio-sync` (not `postio-storage`, and not
`postio-gtk` — widget code produces mostly timeouts under mutation):

```bash
scripts/mutants.sh                            # every crate above
MUTANTS_UPDATE_BASELINE=1 scripts/mutants.sh  # reseed after triage
```

This is the automated form of CLAUDE.md's own instruction to verify your
tests can fail, run against everything at once rather than one test at a
time. It is also genuinely slow: **run it on
`mutants.yml`'s own CI runner, never on a shared workstation.** The first
attempt at a real baseline ran locally, found 1934 mutants across the four
crates, and drove this box's load average past 14 within two minutes of the
initial (unmutated) build alone — with other sessions' builds sharing the
same eight cores at the time. Killed before it produced a single result.
`cargo-mutants` copies the whole tree into its own `/tmp/cargo-mutants-*`
scratch directory before it starts, so killing it costs nothing but the
lost CPU-minutes; nothing in the working tree or its `target/` is at risk
either way. `scripts/mutants.sh`'s own comment carries this warning forward.

**No baseline is committed yet.** `docs/mutants-baseline.txt` does not
exist: seeding it honestly means running the real thing to completion and
reading what survived, which is exactly the run above that had to be
killed. `scripts/mutants.sh` reports this plainly (survivor count and the
`MUTANTS_UPDATE_BASELINE=1` command) rather than crashing on a missing
file, so the first dispatch of `mutants.yml` is expected to fail — that
failure *is* the first real run, on hardware built for exactly this,
sharing nothing with a session's own workstation.

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

**`postio_config::secrets::is_secret_key` matches a substring, and that bit
you the moment a new schema reused it (#191).** It normalizes a key (strips
`_`/`-`/space, lowercases) and checks whether the result *contains* one of
`SECRET_MARKERS` — `"password"`, `"token"`, `"secret"`, etc. — deliberately
generous, because in `config.toml` a false positive costs a renamed key and
a false negative writes a password to disk. That generosity assumes nothing
legitimate in the document ever needs those words as substrings, which held
for `config.toml` and stopped holding the moment `providers.toml`
(`postio-imap/src/discovery/providers_toml.rs`) needed fields named
`requires_app_password`, `password_help_url`, and — worse — the OAuth token
*endpoint*'s own field, `token`, all of which are perfectly ordinary,
non-secret data that the marker list matches anyway. Stripping the whole
parsed table, the way `config.toml` does, silently deleted three real
fields the first time this ran (`cargo build` failed with "contains what
looks like a secret at: provider.gmail.password_help_url" and two others —
not a subtle bug, but one that would resurface for anyone else who points
`strip_secrets` at a *whole* document without first checking whether the
document's own schema uses any of those eight words for something ordinary.

The fix is not to loosen the marker list — `config.toml` still needs it
generous — but to scope *where* it runs: only at an `#[serde(flatten)]
extra: toml::Table` catch-all for fields the schema does not name, checked
after typed deserialization rather than before it. A named, expected field
is never handed to `is_secret_key` at all, however many marker substrings
its name happens to contain; only a key nobody's schema recognizes — a
`client_secret` someone mistakenly pastes in — reaches the scan. Before
reusing `strip_secrets`/`is_secret_key` on a new document type, check
whether that schema's own legitimate field names collide with
`SECRET_MARKERS` first — `password`, `passwd`, `passphrase`, `secret`,
`token`, `apikey`, `credential`, `privatekey`, and the exact matches `pass`,
`pw` — rather than discovering it via a build failure.

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

`scripts/checks/check-toolchain-pinned.py` reports the skew rather than failing on
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
`RUSTUP_TOOLCHAIN` says to. `scripts/tests/test-rustup-toolchain-cleared.py` proves
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

**"Did it land" has to be asked more than once.** `gh pr merge --rebase`
returns as soon as GitHub *accepts* the merge; the `git fetch` on the next line
can still be answered before the new tip is visible. `issue-land.sh` asked
once, and on 2026-08-26 that turned a few seconds of replication lag into
`MERGE DID NOT LAND` for two landings out of three (#194, #299) — for work that
was on `main` already.

The wrong answer is the expensive one here. The message tells the session to
rename the branch and land again, which opens a **second PR for commits already
merged**, and to leave the worktree and the claim held. So the check now
retries for `POSTIO_LANDED_TIMEOUT` (30s) and, when it does give up, prints the
PR's own `state` beside its verdict: `MERGED` there means this check was wrong,
not that the work is gone.

The two directions have a test each and they are not the same test.
`test-issue-land-312.py` is about a merge that **never happened** and must
still fail; `test-issue-land-lagging-ref.py` is about one that happened
**late** and must not. A retry helps only the second, which is why the first
runs with a short `POSTIO_LANDED_TIMEOUT` rather than being relaxed.


**`gh pr merge` exits 0 when it merges nothing, and `gh pr view` finds a PR
that is already merged.** Put together, `issue-land.sh` announced `merged.`,
deleted the remote branch, and exited 0 while the commits never reached
`main` — twice in one session on #277, caught only by checking `origin/main`
by hand afterwards.

The sequence needs nothing unusual. `issue-claim.sh` generates the branch name
from the issue title, so two sessions on one issue produce the same name by
construction — which is the normal state of this repository. The second
session pushes, `gh pr view --json number` resolves the *first* session's
merged PR (it returns the most recent PR for the head branch whatever its
state), the script reads that as "PR already open; the push updated it",
`gh pr merge --rebase` prints `! Pull request #N was already merged` and exits
**0**, and the script believes it. The branch is then deleted from the remote
and the operator is told to run `issue-release.sh`, which removes the worktree
holding the only remaining copy.

Two things guard it now, and the second is the general one:

- The PR's **state** is what decides, not its existence. Only `OPEN` means
  "the push updated it"; a merged or closed PR on the same head branch means
  the name was reused, and the script opens a new one.
- **The merge is verified before it is believed.** Note that ancestry cannot
  answer this — the merge is a rebase, so every commit lands with a new hash
  and the local tip is never an ancestor of the base even on complete success.
  Commit *subjects* survive a rebase, so the check is that each subject being
  landed appears in `origin/<base>` afterwards. On failure the script exits
  non-zero and deliberately leaves the remote branch alone, because at that
  point it may be the only copy.

**A stub that lies passes forever.** Three of the `issue-land` self-tests had
a `gh pr merge` stub that printed `Merged` and moved nothing, so none of them
could ever have caught this; the new verification failed all three the moment
it landed, which is how the gap showed up. They now push the branch into the
bare test remote, as a real rebase-merge does.
`scripts/tests/test-issue-land-312.py` is the regression test: a merged PR on the
same head branch, and a merge that reports success while doing nothing.


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
and `scripts/checks/ci-expected-workflows.py` reads them — including the `&prose`
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

`scripts/tests/test-issue-land-rebase-handover.py` covers all four orderings
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
`issue-release.sh` could leave it pointing at a directory that no longer
exists.** Fixed in #359 — `scripts/rustc-wrapper.sh` now pins the daemon's
`TMPDIR` to `${SCCACHE_DIR:-~/.cache/sccache}/tmp`, which outlives every
worktree. The rest of this entry stays: it is still exactly what you will see
from a daemon started *before* that fix, and the mechanism explains a second
thing that was quietly wrong.

`.cargo/config.toml` sets `TMPDIR = { value = "target/tmp", relative = true }`,
which resolves against *the workspace root of whoever started the sccache
server*. The server is one machine-wide daemon, it keeps the environment it
was launched with, and it is the process that actually creates the compiler's
temporary files. So releasing the worktree that happened to start it broke
every subsequent build on the box:

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
fresh server, which under the fix takes the pinned directory and cannot go
stale again. Do not do it while another session is compiling — the running
build dies with it.

Two measured facts behind the fix, kept because neither is what the
configuration looks like it says:

- **A client's `TMPDIR` is ignored entirely.** Start the daemon with one
  `TMPDIR`, delete that directory, then compile from a worktree whose own
  `TMPDIR` is perfectly valid: it still fails naming the deleted path. Only
  the daemon's copy, taken at spawn, is ever consulted.
- **rustc and the linker inherit the daemon's `TMPDIR`, not cargo's.** So
  `TMPDIR = target/tmp` has governed the compiler's scratch only on boxes
  with no sccache. With sccache the tmpfs protection that setting exists to
  provide was *accidental* — it held because the donating worktree's
  `target/tmp` happened to be on disk, and a daemon spawned from a plain
  shell takes the real `/tmp`, a 6 GB tmpfs here, which is precisely the
  "Disk quota exceeded" failure the setting was written to prevent.

The pinned directory is re-`mkdir -p`'d on every wrapper invocation, so
clearing `~/.cache/sccache` no longer strands a running daemon either — the
next compile recreates the directory underneath it. That is the property the
old arrangement could not have: a released worktree is gone for good.


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


## The scripts directory and the gate (2026-08-25, #315)

**`scripts/` is a small command surface over two subdirectories.** Top level:
the commands a session actually types (`issue-claim.sh`, `issue-land.sh`,
`issue-release.sh`, `check.sh`, `run-isolated.sh`, `test-headless.sh`) plus
infrastructure invoked by config or other scripts. `scripts/checks/` holds
every repository invariant; `scripts/tests/` holds the self-tests.
`scripts/check.sh` runs every `checks/check-*.py` by glob, so **adding an
invariant is dropping a file into `checks/` with a self-test in `tests/`** —
nothing else to wire, and `issue-land.sh` and CI pick it up automatically.

**Self-tests rot while CI is paused, silently.** Two were red on `main` for
days before #315 tripped over them: `test-issue-claim-blocked-by.py`'s
fixture predated the claim script's base-exists guard (no `origin` in the
fixture, so the run died before reaching what it tests), and
`test-issue-base-branch.py`'s `gh` stub predated the #312 merge
verification (it said "Merged" without moving the base, failing the very
check that exists to catch that lie — the merge test's stub had been taught
this; the base-branch one had not). If you change the landing machinery, run
`scripts/tests/` yourself; nothing else will until CI is back.

**`scripts/` runs on BSD userland too, and GNU-only syntax fails there
loudly-but-misleadingly (2026-08-27, #559).** A session on macOS could not
claim an issue at all: `issue-claim.sh` built the branch slug with
`sed 's/[^a-z0-9]\+/-/g'`, and BSD sed has no `\+`, so the substitution
matched nothing, the title passed through with its spaces and colons, and git
refused the ref — *after* the claim lock had been taken, so the retry then
reported the issue as already claimed. `issue-land.sh` had the same `\+`
extracting the issue number; on BSD it yielded the empty string and the guard
below it reported "not an issue branch", which is true-sounding and about the
wrong thing entirely. `issue-release.sh` aged claims with GNU `date -d`.
**The rule: the issue-workflow scripts run wherever a session runs, so they are
POSIX or they are broken somewhere nobody is looking.** Use `[x][x]*` not
`[x]\+`, and `python3` rather than `date -d` — every check already needs
python3, so it costs no dependency. `scripts/tests/test-scripts-bsd-portable.py`
enforces this, and names the three scripts that are Linux-only *by nature*
(`headless-runner.sh`, `test-headless.sh`, `install-local.sh` — mutter, /proc,
the XDG hicolor layout) so that exemption is a decision on the record rather
than a script that happened to fail.

**A platform difference is a parameter, not a `#[cfg]` (2026-08-27, #556).**
The store, the config directory and the drag-out cache answer differently on
Apple — `~/Library/Application Support/Postio`, `~/Library/Caches/Postio` — and
the obvious way to write that is `#[cfg(target_os = "macos")]`. Don't. **With a
`cfg`, each machine can only ever prove half of it**, and the half nobody runs
is the half that rots — which here would be the macOS answer, the one most
sessions cannot check. So `Platform { Freedesktop, Apple }` is an argument:
`store_path_from(env, Platform::Apple)` is asserted on Linux and
`Platform::Freedesktop` on a Mac, and only the public wrapper calls
`Platform::host()`. The same argument applies to anything else that will differ
per platform; reach for the parameter first.

Two things that must stay true, because every fixture depends on them:
`$POSTIO_STORE`, `$POSTIO_CONFIG` and `$POSTIO_EXPORT_DIR` still win on either
platform, and so does a **deliberately set** `$XDG_*`. Someone who exported one
meant it, the platform default has no business overruling that, and it is what
lets a store be shared with a Linux VM on the same machine.

**A second claim queue is a label, not a convention (2026-08-27, #552).** The
macOS frontend initiative (#15) is the first work an ordinary Linux session
must not pick up — most of it cannot even be built there. Its issues carry
`ready-mac` and deliberately **not** `ready`, so a plain `issue-claim.sh` skips
them without knowing they exist; `--ready-label ready-mac` (or
`POSTIO_READY_LABEL`) asks for that queue instead. Two labels rather than one
label plus a rule, because **sessions run on several machines and the claim
locks under `$POSTIO_CLAIMS` are per-machine** — they are a lock between
sessions on one box, not between boxes. Across machines the only guards are the
label, the assignee, and the remote-branch check in `issue-claim.sh`. A
convention ("don't take the macOS ones") is enforced by whoever read CLAUDE.md
most recently; a label the queue query never returns is enforced by the query.

**A gate that cannot run has to say so (2026-08-27, #555).** `issue-land.sh`
runs clippy and the tests over the crates a branch changed. On a host missing
their system libraries that is not a weaker gate — it is *no* gate, and the
branch merges anyway. This is live rather than hypothetical: a macOS session
cannot build `postio-gtk` or `postio-app`, because gtk4 and libadwaita have
arm64 bottles but **webkitgtk has none**, and the reader and the composer are
both WebKit views. So the land script now asks the host what it can build:

- a changed crate the host cannot build is a **hard stop**, before anything is
  committed or pushed;
- a changed crate the unbuildable ones *depend on* still lands — refusing would
  leave such a session unable to do any work at all — but the PR gets
  `needs-linux-verify` and a warning in its body. `postio-app` depends on every
  other workspace crate directly or transitively, so when it is unbuildable,
  *any* changed crate is unproven against the frontend.

The probe is `pkg-config`, not `uname`: a Linux box without the `-dev` packages
is in exactly the same position, and a check keyed on the operating system
would wave it through. **The rule, which is the display rule aimed at the other
axis:** the existing one says a skip nobody can distinguish from a pass is not a
test; this one says **a crate the host never compiled is not a crate that
passed.** The symmetric `needs-macos-verify` direction is wired when `macos/`
exists — the label is created, the code path is not, because untested code that
guards something is worse than no guard.

**sccache is wired in through `.cargo/config.toml`**
(`build.rustc-wrapper = "scripts/rustc-wrapper.sh"`), not exported per shell.
The wrapper execs plain rustc when sccache is missing, so it cannot cause the
"RUSTC_WRAPPER names a binary that does not exist" hard failure an export
could; an explicit `RUSTC_WRAPPER` in the environment still beats the config.
The standing warning above about the sccache *server* keeping the `TMPDIR` of
whoever started it still applies.

**Dev-profile debug info is `line-tables-only`** (workspace `Cargo.toml`).
Backtraces keep file:line — what tests and `RUST_BACKTRACE` need — while the
heaviest part of compiling and linking the GTK/WebKit stack goes away. What
is lost is variable inspection in a debugger; delete the one line to get it
back. Changing it invalidates every cached compile once (sccache keys on
flags), so the first build after it lands pays full price.

**The headless runner keys on cargo's 16-hex metadata suffix** to decide what
runs on the private compositor: `deps/gtk_list-0123456789abcdef` goes
headless, `postio-app` and examples reach the real display — before #315 the
README's own `cargo run -p postio-app` launched the app invisibly.
`scripts/tests/test-headless-runner.py` pins the contract with a stubbed
mutter, so it runs anywhere, fast.

**The measured shape of a real mailbox: ~90% of the bytes are attachments,
carried by ~15% of the messages.** Every sizing argument in this project ends
up needing these numbers, so they are recorded once rather than re-derived.
Taken from the same reference account cited above (81,744 messages), whose
`BODYSTRUCTURE` metadata is fully synced — which is the useful part, because it
means all of this is knowable *before a single body byte is fetched*:

| | messages | bytes |
|---|---:|---:|
| the whole mailbox | 81,744 | 12.43 GB |
| … attachment payloads | 25,752 parts | 11.00 GB (88.5%) |
| … headers + `text/*` | all | 1.43 GB (11.5%) |
| carrying an attachment | 12,712 (15.5%) | 11.26 GB (90.6%) |
| carrying none | 69,032 (84.5%) | 1.17 GB (9.4%) |
| over the 5 MB `max_body_bytes` cap | 539 (0.66%) | 6.02 GB (48.4%) |
| distinct attachments by (filename, size) | 13,099 of 22,878 | 7.69 GB of 10.96 GB |

By MIME type the payloads are dominated by `application/pdf` (5.0 GB),
`image/jpeg` (2.6 GB), `application/zip` (0.76 GB) and
`application/octet-stream` (0.66 GB) — all already compressed, which is why
[ADR 0017](decisions/0017-backfill-cost-attachments-memory-disk-encryption.md)
skips compression for them and expects the whole saving to come from the text.
`disposition = 'inline'` is 2.64 GB of the total: CID images in HTML mail, which
is why small inline parts ride with the text axis rather than the payload axis.

Two consequences that keep catching people out. **The existing 5 MB cap is not
a rounding error — it is half the mailbox**, refused by declining 0.66% of
messages; any argument about raising or lowering it is an argument about
gigabytes. And **the last-30%-dedup is free**: content addressing collapses
22,878 attachment parts to 13,099 distinct ones, provided the id is taken on the
decoded payload rather than on its base64.

**The database's own weight, measured the same way** (`dbstat`, on a store with
81,744 messages and only 902 bodies fetched, so this is very close to a pure
metadata cost): 163 MB total, of which `recipients` and its four indexes are
**56 MB — 34%, larger than `messages` itself** (378,819 rows at 4.6 per message,
each storing an address and its lowercased near-duplicate). Two of those indexes,
`idx_recipients_draft` and `idx_attachments_draft`, are not partial and so index
a column that is NULL on every row in the table; `idx_recipients_draft` alone is
6 MB. Per message the metadata costs about 2 KB. Anyone projecting a store's
size should start from that number and add the text corpus, not from the
message count alone.

**Nothing reclaimed disk for the life of the project, because three sweeps
had no caller.** `BlobStore::collect_garbage`, `BlobStore::purge_temporary`
and (later) `BlobStore::evict_to_fit` were each written, tested, benched where
relevant and documented — and no production code called any of them (#416).
The consequence was not subtle: `MessageRepository::delete` removes a
message's row and deliberately does *not* touch its blobs, because the
schema delegates reclamation to the sweep, so **deleting mail freed nothing,
ever**. The worst case needs no user at all — a `UIDVALIDITY` reset wipes and
re-syncs a whole mailbox, orphaning every blob in it at once. They are wired
now from `postio_app::reclaim_disk`, beside the body-index catch-up.

This is the **third recorded instance** of the same shape, after
`MailBackend::list_mailboxes` (no production caller for the life of the
project, hidden because `MockBackend::new()` invented an INBOX) and
`index_body` (written, tested and uncalled until #327, so `search_documents.body`
was empty on every message in every real store). The pattern is now specific
enough to state: **a `pub fn` in a leaf crate, fully tested, is not evidence
that anything calls it** — and its own unit tests pass just as happily either
way, so the suite gives no signal at all. The tests that catch this class live
at the far end, in `crates/postio-app/tests/app_suite/`, and assert *"a store
this application opened has had X done to it"* rather than *"X works"*.

**The grace period is load-bearing, and a test that shortens it tests nothing.**
`GarbageCollection::min_age` (one hour, `postio_session::BLOB_GRACE_PERIOD`)
exists because a blob is written *before* the row that references it is
committed — inside that window a perfectly healthy blob is indistinguishable
from an orphan, and a sweep without the grace period deletes the body of a
message that is mid-fetch. `reclaim_wiring.rs` therefore back-dates the
orphan's mtime rather than passing a shorter period: the first version of that
test passed `Duration::ZERO`-adjacent timing, failed, and the failure *was* the
grace period working. Injecting a shorter period would have made it pass while
exercising a configuration that never ships.

## A slow query whose SQL is fast is measuring the machine (#500)

A search the readout timed at **3.8 s** replayed at **15 ms** — the same
three statements, the same term, on a copy of the same store. Nothing was
wrong with the plan; everything was wrong around it. The chain, longest
lever first:

1. **The body catch-up was an infinite loop.** `index_body` deliberately
   wrote no row for a textless body, so an attachment-only message (a DMARC
   report, an image) never left `messages_missing_body_text`'s candidate
   set. The store had 654 of them — more than one 200-message batch — so
   `index_local_bodies` re-selected the same batch for ever: a core at 100%
   for as long as the app ran, a stream of ungated autocommit writes, and a
   full-table candidate probe per pass evicting the page cache the search
   needed. Found not by any test but by `top -H` on the live process and
   `gdb -p <tid>` on the hot thread, which is the first thing to reach for
   when a *read* is slow while the SQL is provably fast.
2. **The replay lied because `cp` warms the cache.** Copying the store to
   probe it pages the whole file into the OS cache, so the replay measured
   warm reads while the app was reading cold, on a machine at full swap
   from parallel builds. A later replay under real load reproduced seconds.
3. **Benches on tmpfs cannot see any of this.** `test_support::memory()`
   lives on `/dev/shm` and plain `tempdir()` lands on `/tmp`, tmpfs on the
   reference platform — WAL exists but disk I/O does not, so no write
   pressure there can ever slow a read. `search_under_load.rs` builds its
   corpus under `CARGO_TARGET_TMPDIR` (inside `target/`, a real filesystem)
   for exactly this reason; anything measuring I/O contention must do the
   same.

The structural fixes, so the shape cannot come back: a textless body writes
an **empty index row** — "tried, nothing there" and "never tried" are now
different states; the catch-up **refuses a batch identical to the last one**,
so no future regression can spin it; batches commit **once, behind a
Background write-gate permit**, with the blob reads phased before the
transaction and a breather after it. On the read side the box runs **one
search in flight at a time** (`Live::settled` is the release valve a failed
run must call) and the debounce is sized to typing cadence, so a slow store
is never asked five questions for one word.

## Encrypting the store, and the things it made visible (2026-08-28, #610/#300)

Bodies moved out of the blob store into compressed `messages` columns
(ADR 0020) and the database became SQLCipher (ADR 0014) in one pass. The
encryption itself was uneventful. What it *exposed* was not, and most of it
had been latent for months.

### `exit()` does not stop threads, and now that matters

`Engine::spawn` started a thread and dropped its `JoinHandle`, so nothing
could wait for it even in principle. The application then leaked each engine
on the reasoning — written in the code — that "dropping it at exit would stop
the engine a moment before the process ends anyway".

That was true until the store was encrypted. `exit()` runs the process's exit
handlers and then kills it; every page the sync thread writes now goes through
libcrypto, and libcrypto is torn down by those handlers:

```
thread A: exit() -> __run_exit_handlers -> (libcrypto goes away)
thread B: sqlcipher_page_cipher -> walWriteOneFrame
          -> sqlite3PagerCommitPhaseOne -> SyncStateRepository::observe
```

A coredump, not a theory: `postio-app --test e2e` every run, the engine tests
about one in six, and the application whenever somebody quit mid-sync. No mail
is lost — a torn WAL frame is what recovery is for — but the process dies on
the way out.

**A detached thread that writes to the store is a bug now, whatever it looks
like locally.** `Engine` keeps its handle and is joined: `Drop` for the
ordinary case, `Engine::stop` for the handles the application holds for the
whole session, `stop_retained` called by `run` once the GTK loop returns. The
wait is bounded at five seconds and gives up saying so, because the last handle
usually goes on the main loop and a shutdown that blocks it on a stalled
network read is a worse bug than the one being fixed.

`postio-storage` also asks libcrypto not to register its `atexit` handler.
That is belt to those braces and **does not stand alone** — with a system
libcrypto the DSO is finalized regardless, which is how the remaining crashes
were traced back to the thread rather than the flag.

### `cipher_memory_security` is a correctness setting, not a tuning knob

ADR 0014 lists it as the second performance lever after `cache_size`. It is
not: with it on, Postio segfaults inside a WAL write. The feature `mprotect`s
SQLCipher's internal buffers `PROT_NONE` between uses, and this application
always has two connections writing at once — the sync engine committing a pass
while the UI writes a flag is the ordinary state, and the whole reason
`WriteGate` exists. One connection shields a page another is mid-cipher on.

Off, permanently, and issued before `PRAGMA key` because SQLCipher wants it
there.

### `PRAGMA key` cannot fail, so something must read a page

SQLCipher accepts any key and only discovers a wrong one when a page will not
decrypt — surfacing later, elsewhere, as `SQLITE_NOTADB`: *"file is not a
database"*. That sentence reaches a screen (#404), and it tells somebody their
mail is corrupt when it is intact and merely locked. `configure` reads page 1
immediately and turns the failure into `Error::WrongStoreKey`.

### mmap is gone, and the memory story improved

`PRAGMA mmap_size` is meaningless over encrypted pages — SQLCipher decrypts
each one into the page cache, so there is no version of "the file is the
buffer". Removing it moved memory out of the file-backed half: that row used
to grow 83 → 167 MiB with mailbox size and is now flat at ~121 MiB of shared
libraries, and resident total at 100k messages went from 215 MiB to 177 MiB.

### Measuring an encrypted store: three traps, all of which caught us

1. **A stale binary measures an error screen.** After isolating a cost by
   patching out `PRAGMA key`, `target/release/postio` was still the plaintext
   build; it could not open the freshly-encrypted stores, so the first memory
   run measured a store that never opened — and "flat memory" looked entirely
   plausible. **Verify the store opened before believing any number from it.**
2. **The startup passes are a transient.** Anonymous memory peaks well above
   the settled figure — 86 MiB against 55 MiB on a 400k store — while the
   body-index catch-up and the dictionary trainer run. Sampling at ten seconds
   measures that and calls it the baseline. Wait 45 s.
3. **Two data points cannot show a bound.** 1k → 100k rises; it takes a third
   point at 400k, where it does not move at all, to show the shape is the page
   cache filling rather than the mailbox loading.

**Attribute cost to the cipher by measuring, not by reasoning.** Patching out
`PRAGMA key` in `db::configure` and re-running the same bench against an
equivalent plaintext store takes two minutes and settles it. Done that way:
encryption costs ~5% on the unified page, ~22% on startup — and is *not* why
the unified page is over budget (#619) or why startup drifted (#636). Without
the isolation the cipher would have worn both.

### The gates that nothing runs

Four bench regressions (#619, #622, #636, #638) and a licence drift (#639)
were all found by hand in one session, and they share a cause: `cargo bench`
is not in the steward loop and `cargo deny` was in no gate at all. `deny.toml`
had been a policy in the sense that a sign is a policy, while three crates
declared `GPL-3.0-or-later` in an MIT workspace and stayed green.

`check.sh` now runs `deny.toml` (`check-dependency-policy.py`). Benches
deliberately did **not** get a `--no-run` gate: `cargo clippy --all-targets`,
which `issue-land.sh` already runs, compiles them — verified by breaking one
and watching `--all-targets` catch it and a plain `cargo check` miss it. None
of the four failures were compile errors anyway. Only *running* them catches
those, which CLAUDE.md already asks of the reconcile pass.

### The vendored OpenSSL costs more than the ADR priced

ADR 0014 prices `bundled-sqlcipher-vendored-openssl` as "the heaviest new
compile in the graph", absorbed by sccache. That covers compiling OpenSSL and
not *configuring* it: `Configure` is a perl program, Fedora splits the perl
standard library into packages, and getting it to run took six of them plus
twenty-two transitive, discovered one build failure at a time as
`Can't locate X.pm in @INC` inside a cargo build script. There is no
`perl-core` metapackage in the Fedora 44 repos; the definitive list comes from
grepping `use` statements out of the extracted OpenSSL source. They are in the
README's system deps now.

The `bundled-sqlcipher` + system-libcrypto variant the ADR records as its
alternative needs none of that and built first time. The one thing vendoring
genuinely buys is that a statically linked libcrypto has no DSO to finalize at
exit — belt to the engine join above, not load-bearing now that the join
exists.

## Cross-platform dependencies and what a Linux box can prove (2026-08-28, #642)

`main` spent a day unbuildable on Linux because a macOS dependency section
swallowed fifteen entries of `postio-imap`'s `[dependencies]` (#642). The
diff looked tidy — `security-framework` sorts between
`rustls-platform-verifier` and `secrecy`, so it read as an alphabetical
insert — and a TOML table runs until the next header, so everything below it
became macOS-only. On Linux the crate lost `postio-model`, `tokio`, `serde`
and twelve more, and produced 219 errors.

Nothing caught it because nothing built it: CI is `workflow_dispatch`-only and
the reconcile pass had not run since it landed.

Postio is one workspace targeting Linux and macOS (ADR 0019), so this class
recurs by construction. **A Linux box cannot build or test the macOS half**,
which is true and is also where the reasoning usually stops. It can do rather
more than nothing. Three layers, cheapest first:

### 1. Placement, enforced (every machine, instant)

`check-target-sections-last.py`: a `[target.'cfg(...)'...]` table must come
after every plain `[dependencies]`, `[build-dependencies]` and
`[dev-dependencies]` table. Platform sections live at the foot of the
manifest.

This is a placement rule, not a correctness proof, and the distinction is
worth keeping straight: TOML has no notion of a table somebody *meant* to keep
going, so the swallowing is not detectable. The position that makes it
possible is. **A platform section at the foot of the file cannot swallow
anything, because there is nothing below it to swallow.**

### 2. Cross type-checking, as far as the C dependencies allow

`scripts/cross-check.sh [triple]` runs `cargo check --target` over every
workspace member. Measured on this workstation against
`aarch64-apple-darwin`:

| | |
|---|---|
| **6 checked** | postio-model, postio-config, postio-core, postio-body, postio-search, postio-ui |
| **12 skipped** | everything else |

Every skip is a **C build script** wanting a cross-toolchain this machine has
not got — `ring` (via rustls), `zstd-sys` and `openssl-sys` (postio-storage),
the GTK sys crates. Never Rust. The script reports `skipped` separately from
`FAILED` for exactly that reason: a crate whose C dependency would not build
taught us nothing about its Rust, and saying "ok" there would be a lie.

The six are not a consolation prize. `postio-config` is where Apple's
directory layout lives, and `postio-ui` is ADR 0019's shared frontend logic —
the two crates most likely to carry macOS-only code that a Linux build never
compiles. Verified by planting `#[cfg(target_os = "macos")]` code that calls a
function that does not exist: `cargo check -p postio-config` reports **0
errors**, and `cross-check.sh` reports `FAILED postio-config` with the missing
function named.

Setup, once — it is a large download and deliberately not in `mise.toml`:

```sh
rustup target add --toolchain "$(rustup show active-toolchain | cut -d' ' -f1)" \
  aarch64-apple-darwin
```

Not wired into `check.sh`: it compiles a second copy of the dependency graph,
which is minutes on a cold target directory, and `check.sh` runs on every land
across every session. It belongs in CI and in the reconcile pass.

**The skipped twelve could shrink.** `cargo-zigbuild` supplies a cross
compiler that can build C for Apple targets, which would bring `ring`,
`zstd-sys` and `openssl-sys` into reach for `cargo check`. Not tried; worth it
if the macOS port grows and this layer starts feeling thin.

### 3. A macOS runner, for everything else

Linking, the Apple frameworks, `security-framework` actually resolving, the
Swift half, and **running any test at all**. There is no substitute and no
approximation. Whatever CI eventually looks like, a macOS job is what the
other twelve crates get.

### The rule of thumb

Each layer catches a strictly cheaper class than the one below it, and the top
two run on any developer's machine. When adding a platform-conditional
anything, the question is not "can I test this here" — usually no — but "which
of these three is the cheapest thing that would have caught me getting it
wrong". For #642 it was the first, and it costs milliseconds.

## Six types are called *Scope*, and they answer four questions (2026-08-28, #670)

Before adding a seventh, or before reading a `scope` field and assuming you
know what it holds: the word is heavily overloaded in this workspace, and the
overloads are all legitimate — they are genuinely different questions that
happen to want the same English word.

| Type | Home | Question |
|---|---|---|
| `AccountScope` (re-exported as `postio_core::state::Scope`) | `postio-model` | **Which accounts?** `Unified` or one account. #186 moved it down here from `postio_core::state` so search and the list could not disagree; its doc comment is the best short argument in the tree for moving a type down a crate. |
| `postio_search::facets::Scope` | `postio-search` | **Which slice does a search look at?** `AllMail` / `Inbox` / `Lists` — the canvas's standing, no-typing rescope. Not a mailbox id, deliberately. |
| `ListScope` | `postio-model` (was `postio-runtime::store`) | **Which messages is this view showing?** `Mailbox` / `Account` / `Flagged` / `Snoozed` / `Thread`. What the message list is paged over. |
| `ViewScope` | `postio-core::state` | **What is a whole-view selection relative to?** `Mailbox` / `Flagged` only, because `Ctrl+A` is not a gesture inside a thread and nothing needs a `Snoozed` predicate yet. |
| `FeedScope` | *deleted by #670* | Was `postio-gtk`'s own spelling of `ListScope`. |
| `ScopeFfi` | `postio-ffi` | Not a question — the uniffi ABI mirror of `ListScope`, with `i64` fields. A wire format, the way `ExtCommand` is the owned counterpart of `CommandSpec`. |

**The pair worth understanding is `ListScope` and `ViewScope`**, because they
look like one type spelled twice and are not. `ViewScope` is the *result of a
rule* applied to a `ListScope` — `postio_core::aim::view_scope` — and its
smaller variant set is the point: a `ViewScope` that cannot be constructed
from a thread drill-in is what makes "no whole-view gesture inside a
conversation" a compiler check rather than a conformance table two frontends
have to keep passing. Collapsing them into one type with a predicate would be
one type fewer and a strictly weaker guarantee.

**The rule of thumb.** A new `*Scope` is warranted when it answers a question
none of the above asks, and it belongs in the lowest crate that all its
readers share — which #186 and #670 both discovered the same way, by finding
a second crate that needed the same value and could not reach it.
