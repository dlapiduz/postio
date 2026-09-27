# Research: Postio Focus

Phase 0 for [plan.md](./plan.md). Each section is a decision, why it was taken,
and what it was chosen over. The tree is `origin/main` at `b76a5f00`
(2026-09-27), which includes spec 006 (`crates/postio-render`, ADR 0042).
Paths are repository-relative; line numbers are against that tree.
External versions were checked on 2026-09-27.

---

## R0. What this plan is built against

**The renderer has landed.** Spec 006 put message bodies on
`crates/postio-render`, a Blitz-based engine with no toolkit, no network and
no C in its graph. It runs one render thread per reader and returns an
immutable snapshot: a display list, a `TextIndex`, and link, message, fold and
anchor boxes (`crates/postio-render/src/lib.rs:31-105`). `BodyView`, a
`GtkScrollable` in `crates/postio-gtk/src/body_view/`, paints that snapshot as
tiles and supplies selection, find, links, accessibility and zoom. ADR 0042
records the rule. ADR 0032's decision stands, but its WebView mechanism is
superseded. The handoff's condition for planning, a merged renderer, is met.

**Who else is in these files** (`/lanes`, 2026-09-27):

- **`feature/contacts`** is 32 commits ahead, held for review since
  2026-09-24. It edits:
  - `crates/postio-core/src/{command,registry,context}.rs`;
  - the reader header (`crates/postio-gtk/src/reader/{message_header,view}.rs`);
  - `crates/postio-gtk/src/finder.rs`;
  - `crates/postio-ui/src/{paging,keymap,reader/header}.rs`;
  - `crates/postio-app/src/{recipients,compose,reading}.rs`;
  - the contacts tables.

  Every one of these is a file this plan moves or changes. So each move here
  is a pure `git mv` commit, which keeps rename detection working for
  whichever branch rebases second. The keymap change is one commit of its
  own. The branch that lands second rebases.
- **Stale claims.** The `in-progress` issues on read paths (#1609, #1612,
  #1602) and on the web processes (#1603, now largely overtaken by 006) have
  no worktree and no branch.

**The store has no migrations.** The schema is one declared batch, and its
hash is stamped into `user_version`. A store with any other fingerprint is
refused, and the user resyncs into a fresh one:

- `crates/postio-storage/src/schema.rs:62-88`;
- `crates/postio-storage/src/store.rs:458-480`.

Every schema change therefore clears what lives only in the store, snoozes
and labels included. That decides R14: what the user decides lives in
`config.toml`, and the store holds only what can be recomputed or re-entered.

`CLAUDE.md` and the constitution both still say "migrations are still
written". Recorded here for the maintainer, and not changed by this branch.

---

## R1. The shared GTK crate is `postio-widgets` (open decision 7)

**Decision.** A new crate, `crates/postio-widgets`, holds the GTK widgets and
presenters that more than one desktop app uses. Both desktop apps depend on
it, and neither depends on the other.

What moves out of `crates/postio-gtk`, in this order, one pure move commit
and one wiring commit each:

1. **`body_view/`**: `BodyView`, tiles, interaction, find, zoom and
   accessibility. It already has no `crate::` imports (`body_view/mod.rs`).
   Its only dependencies are gtk, adw's `StyleManager`, postio-render,
   postio-ui, `postio_config::ZOOM_STEPS` and `postio_body::Cap`.
2. **The small widgets Focus uses**: `widgets/{keyhint, keycap, action_bar,
   button, chip, notice, toast}.rs`, and their rules from `data/shell.css`.
   `widgets/screen.rs` stays, because it takes the classic `Window`.
3. **The reader:**
   - `reader/view.rs` (`Reader`), `message_header.rs`, `banner.rs` and
     `notices.rs`;
   - the attachment chips (`parts::Chips`, `parts.rs:1101`).

   The verb bars `Reader` draws (`reader/actions.rs:33-55`) become
   configuration: the classic app passes its three bars, and Focus passes
   none, because its dialog has its own toolbar. Several parts stay in
   postio-gtk as the classic reading pane: `conversation.rs`, `reader/rail.rs`,
   the parts panel and the search preview. `mark_html`
   (`crates/postio-gtk/src/search.rs:1568`) is plain string code, and moves to
   postio-ui.
4. **The composer**: `composer.rs` (4,687 lines), `editor.rs` and
   `data/editor.js`. They move behind a `ComposerHost` trait that replaces the
   composer's use of the classic `Window`. Today it uses the window for:
   - the pane and its occupant (`composer.rs:1888`, `:2443-2465`);
   - the context (`set_context`) and the command broadcast (`:1929`);
   - the keymap in force (`:1940`, `:3268`);
   - the file dialog's parent (`:2152`, `:2208`);
   - autosave, which returns early outside a classic window (`:2668`).

   `dispatch` (`:1961`) becomes public. The composer still edits in WebKit,
   because ADR 0039's native editor is decided and not built, so
   postio-widgets carries webkit6 for the composer alone.
5. **The list model adapter**: `MessageList`, a `GListModel` over
   `ListWindow` (`crates/postio-gtk/src/list.rs:283-376`), generalised over its
   row type.
6. **Presenters that would otherwise be written twice:**
   - the composer's seam wiring over postio-client
     (`crates/postio-app/src/compose.rs:77-98`);
   - the reader's wiring (`crates/postio-app/src/reading.rs`: the blob
     source and the remote-image fetcher);
   - the config service and watcher glue (`crates/postio-gtk/src/config.rs:111`);
   - the credential and add-account dialogs, which both apps need for
     screen 19 and a first run (`crates/postio-app/src/{add_account,settings_credential}.rs`).

**New in postio-widgets**, built for both apps and used first by Focus:

- the four row-anchored pickers and a typed-date entry (R6);
- label pills;
- an opt-in recipient chip entry (R15);
- the `AdwDialog` frame Focus's windows use.

**The crate's rule** is ADR 0043, kept to the rule. It gets a new entry in
`scripts/checks/check-crate-boundaries.py`:

- **It may depend on** gtk4, libadwaita, webkit6 (for the editor),
  postio-render, postio-ui, postio-core, postio-body, postio-config,
  postio-model and postio-client.
- **It may not depend on:**
  - rusqlite, libsqlite3-sys, turso, turso_core or io-imap, as postio-gtk
    may not;
  - postio-host, postio-session, postio-runtime, postio-storage or
    postio-sync, because presenters reach mail through postio-client;
  - postio-gtk, postio-app or postio-focus.
- **The desktop crates:** postio-gtk gains "not postio-focus", and
  postio-focus gains "not postio-gtk, not postio-app".

**Checks that must widen with the move.** Several scan `crates/postio-gtk`
only, so code that moves would escape them:

- `check-key-hints-are-derived.py:39`
- `check-buttons-have-a-kind.py`
- `check-no-dead-css.py`: stylesheets under `crates/postio-gtk/data` only.
- `check-shadows-use-tokens.py`
- `check-spacing-literals-ratchet.py`: `composer.rs`'s baseline line moves
  with the file.
- `check-reader-header-has-one-home.py`
- `check-blocking-now-sites.py:37`
- `check-uncalled-pub-fn.py`'s `FRONTENDS` (`:109`), which exempts the
  composer's 31 `test_*` hooks only inside the frontends.

Each gains postio-widgets and postio-focus.

**Found while reading, and fixed in the move** (each is small, and each hits
Focus's one-view-reused dialog directly):

- **The detached composer's window never calls `style::track`.** It stays
  light in dark mode (inferred from `composer.rs:1697` and
  `window.rs:3524-3530`).
- **`BodyView` keeps state from the previous message.** Its selection,
  focused link, toggled folds and darkened flag survive
  `set_content_from_top`. In single-message mode the darkened key is `""`, so
  darkening one message darkens the next.
- **The plain-text fallback is never filled.** `Place.plain` is never written
  (`reader/view.rs:308`, `:336`, `:1882`), so a render that times out shows
  only its notice.

**Rationale.** The handoff's reuse table and FR-006/FR-007. The reader,
composer and widgets are what both apps draw. The reports show `BodyView` is
ready to move, `Reader` needs only the widgets and `mark_html` beside it, and
the composer needs one interface.

**Alternatives:**

- *Focus depends on postio-gtk.* Forbidden by the handoff.
- *postio-gtk becomes the shared crate, and the classic shell moves into
  postio-app.* A move of every classic file, and a much larger collision with
  `feature/contacts`.
- *Copy what Focus needs.* Forbidden by FR-006.

Names considered were `postio-gtk-kit`, `postio-components` and
`postio-surfaces`. `postio-widgets` says what the crate holds, and greps
cleanly.

---

## R2. The message view, on the 006 renderer

**Decision.** Focus's open-email dialog hosts the shared `Reader` in
single-message mode. One `Reader` serves every open. Tests already pin that
reuse:

- `crates/postio-gtk/tests/gtk_suite/gtk_reader.rs:839`, `:1361`, `:1437`;
- `crates/postio-app/tests/app_suite/reader_spawns_no_web_process.rs:150-200`.

**Layout.** The dialog owns its own header (Close, subject and position,
`k`/`j`) and its toolbar. Inside it, in order:

1. `Reader`'s native header card, with a Focus presentation option;
2. the marker card, in `Reader`'s notice slot;
3. the body (`BodyView`);
4. attachment cards;
5. the quoted-history fold line.

`BodyView` is its own scroller (`reader/view.rs:376-385`), so the header and
marker cards stay put while the body scrolls. Screen 04 draws one scrolling
column; the screen comparison records this difference.

The alternatives were both worse:

- *The cards as HTML chrome in the document*, as the conversation document
  draws them. Their buttons would become verb-scheme links, and the header
  card would need a second implementation in the reader's CSS.
- *`BodyView` inside an outer scroller.* It tiles only its own visible
  rectangle, through its own adjustments.

**Highlighting a sentence (FR-035; screen 23 in milestone 2).** `BodyView`
gains a public way to mark a character range of the snapshot's `TextIndex`,
drawn as the find overlay is (`body_view/interact.rs:189-217`), and to scroll
to it. `scroll_into_view` (`mod.rs:604`) is `pub(super)` today.

The range is found from the stored excerpt with `TextIndex::find`
(`crates/postio-render/src/text_index.rs:513`, insensitive to case and
diacritics). When the excerpt appears more than once, the stored offset picks
the occurrence nearest its proportional position. Stored offsets can't be
used directly: the index's text is in laid-out reading order, with whitespace
collapsed, `alt` text included and closed folds left out.

Rejected: `mark_html` with offsets. It matches terms, and a sentence can span
inline markup.

**Measured (spike S5, T011, 2026-09-27).** The spike ran over the render
corpus: 61 fixtures and about 980 sentences a detector could store. Each
sentence was read two ways:

| Excerpts read from | As read | Whitespace collapsed | Present at all |
|---|---|---|---|
| The text part first, as search reads it | 91.7% | 97.7% | 98.2% |
| What is drawn: the HTML, flattened, when there is HTML | 92.8% | 98.8% | 99.5% |

The plan holds, with three changes:

- **The locator collapses the excerpt's whitespace** (T066). The index writes
  every run of whitespace as one space, a `<pre>` line break included, and
  `find` compares whitespace as it is written. Collapsing recovers six points.
- **The locator matches any whitespace to any other, on both sides.** Blocks
  and table cells are line breaks and tabs in the index, where a flattened
  source has spaces. This recovers most of what is left of "present at all".
- **The detector reads what is drawn** (T115). Text that is never drawn
  cannot be found: a `<title>`, a hidden preheader, a blocked image's `alt`.
  A `text/plain` alternative can also say something the HTML does not; read
  that way, 13 more sentences are lost.

The tiebreak was measured on every body sent twice over, because no sentence
occurs twice within its own message anywhere in the corpus. It picks the right
occurrence 1,907 times in 1,908 (text first) and 1,933 times in 1,934 (what is
drawn). The one miss is a line of emoji.

The test is `crates/postio-render/tests/excerpt_locate.rs`, on the nightly
profile. Its floors are these numbers rounded down.

**Quoted history (FR-034).** Single-message documents give their quote folds
ids and a line count, as thread documents do:

- ids: `crates/postio-ui/src/reader/thread.rs:84-100`;
- today `crates/postio-body/src/quote.rs:34-112` emits `<details>` with no
  count.

This is a shared change: the classic single-message reader gains folds that
open.

**Raw source (`v`, FR-033).** A new `ViewSource` command shows the stored
raw message (`messages.raw_blob_id`, set with the body at
`crates/postio-storage/src/repository/messages.rs:1743-1755`). When the raw
blob is not local, it is fetched on that deliberate key press. The existing
`view-source` signal shows the decoded body, not RFC 822, and nothing invokes
it (`reader/view.rs:1961`).

**Images and links** come with the presenter:

- per-sender consent, and the runtime's fetcher on consent
  (`reader/view.rs:686-757`, `:1899`);
- links opened only on activation, with the target on hover
  (`body_view/mod.rs:498-601`).

The remote-image allowlist is loaded once per app and shared by that app's
readers; today each `Reader` loads its own copy. `o` offers the snapshot's
`links` and the message's parts in a chooser.

---

## R3. Focus as a frontend: one crate, on the terminal's pattern

**Decision.** One crate, `crates/postio-focus`, holds the binary, the view and
the presenters. This is the terminal's shape, one crate over `postio-host` and
`postio-client`, not the classic split into postio-gtk and postio-app. That
split exists so a store-reading example can live outside a crate that is
banned from the store engine. Focus reads through the client in-process, so it
needs no such split.

**Startup.** It follows the desktop, not the terminal:

- The store opens on its own thread
  (`crates/postio-app/src/lib.rs:1109-1165`) behind a window that already
  exists, saying what it waits for.
- `Host::start`, then `connect(ClientKind::Focus)`. `ClientKind` gains
  `Focus` (`crates/postio-client/src/protocol.rs:26-35`).
- If another app holds the store, Focus shows the shared sentence with "Try
  again", as the desktop does (`lib.rs:1376-1390`). The sentence is
  `crates/postio-storage/src/error.rs:171-172`.

**Events.** There is exactly one reader of `client.events()`, pumped into the
GTK main context. Clones of the receiver compete for events
(`crates/postio-host/src/lib.rs:1558-1560`).

**The list:**

- It is built from postio-ui's `ListWindow`, `Paging` and `SelectionState`
  (`list.rs:75-81` pages of 50, eight cached; `selection.rs`), over the shared
  adapter (R1).
- Rows are Focus's own widgets, each one custom `snapshot()`, as
  `/gtk-design` requires.
- There are exactly two heights, measured from screen 01: **40 px** for one
  line (the classic row height) and **72 px** for two. A row's kind, marker
  or no marker, decides its height; its content never does.

**Surfaced rows.** Digest deliveries (R13) and fired reminders (R7) are rows
that are not conversations. There are few of them. Each is spliced into the
window at the position given by the number of conversations newer than it:
one bounded count per surfaced row, cached against the list's witness
(`crates/postio-runtime/src/store/local.rs:77-110`).

Rejected: a SQL `UNION` of conversations and surfaced rows. It would touch
every place the list query's membership test appears (R13).

**Config.** Focus's settings go in a `[focus]` section. `ConfigChanged`
(`crates/postio-config/src/change.rs:41-62`) gains a `focus` flag. Focus runs
the shared config service and watcher, so `[keys]` and `[focus]` reload
live. The terminal has no live reload today (`crates/postio-tui/src/run.rs:82`),
and it is not in this plan's scope.

**Packaging.** Focus is a second launcher inside the desktop Flatpak:

- app id `dev.postio.Postio.Focus` (a name inside the app's own namespace,
  which a sandboxed app may own);
- desktop file `dev.postio.Postio.Focus.desktop`;
- binary `postio-focus`, built by `flatpak/dev.postio.Postio.json` beside
  `postio`.

The release workflow's `flatpak` job builds both. There is no separate
Flatpak: the two apps share every library, and only one runs at a time.

**Screens against PNGs.** `cargo run -p postio-focus --example shot -- <png>
[dark] [WxH] <screen>` renders a named screen from a seeded demo store. The
store is `postio_storage::seed`, plus Focus's markers, digests and filter
decisions, written through the host. This is the classic `shot` loop
(`/gtk-design` §6) applied to the Focus screens.

---

## R4. One keymap for every app

**Decision.** `KEYS.md` becomes the registry's defaults, the maintainer's
answer in the spec's Clarifications. The registry keeps one row per command.
Defaults change, and a command only another app has gets a key that does not
collide.

**Frontend availability is generalised.** `Availability.terminal: bool`
becomes `Availability.frontend: Frontend {Classic, Terminal, Focus, Macos}`
(`crates/postio-core/src/registry.rs:193-230`):

- `Requirement::Terminal` and `Requirement::Graphical` keep their meaning;
- `Requirement::Focus` marks commands only Focus offers: invitations,
  has-action, Filtered, digests and remind.

**New contexts:** `Picker`, `Digest` and `Filtered`. The open-email dialog is
`Context::Reader`. Its fallback chain (Reader → Conversation → List → Global,
`crates/postio-ui/src/keymap.rs:561-589`) is what lets `j`/`k` step the list
from inside the dialog.

**Collisions, and where each key goes.** Collisions were found from the
registry on this branch.

| Key | Today | After |
|---|---|---|
| `s` | Flag | Snooze. Flag moves to `*` |
| `d` | Delete | Digest rule. Delete moves to `Delete` |
| `h` | Previous view | Remind if no reply. Previous view moves to `Left` |
| `l` | Open message (alternate) | Label. The alternate is dropped; Label moves off `L` |
| `U` | Mark unread | Unsubscribe. Mark unread becomes Toggle read on `r` |
| `X` | Unsubscribe | Select all; `mod+a` stays as its alternate |
| `R` | Refresh (alternate) | Restore from Filtered. Refresh keeps `F5` |
| `u` | Undo | Free. Undo moves to `mod+z` |
| `J`/`K` in a conversation | Next/previous in conversation | `]`/`[`. `J`/`K` extend the selection in the list |
| `g f` | Focus the folder list | Filtered. The classic command folds into "Go to folders" (`g o`): the classic app focuses its folder list, and Focus opens its folders popover |
| `g d` | Drafts | Digest rules. Drafts moves to `g t` |
| `g t` | Sent | Drafts. Sent moves to `g s` |
| `g s` | Flagged | Sent. Flagged moves to `g *` |
| `o` (Search) | Toggle result order | Open attachment or link. Result order moves to `alt+o` |
| `D` | Darken message | Stop digesting sender. Darken moves to `alt+d` |
| `A` | Archive thread | Unchanged. In a digest it archives the whole digest: "archive everything this row stands for" |
| `b`/`B` | Snooze / Unsnooze | `b` is free. Unsnooze keeps `B` |
| `L` | Add label | Free |

The enumeration test (SC-015) is the arbiter. A proposed key it rejects is
changed there, not argued.

**Tests that pin today's keys change in the same commit:**

- `crates/postio-core/tests/core_suite/command_registry.rs:75-164`
- `registry.rs:2256-2275` and `:2327-2340`
- `crates/postio-gtk/tests/logic_suite/keymap_defaults.rs:144-175`
- the golden `linux-bindings.txt`
- `docs/keybindings.md` (`POSTIO_UPDATE_DOCS=1 cargo test -p postio-ui`)
- the terminal's `app.rs` tests that press `s`, `d` and `X`
- the terminal's parity test (`crates/postio-tui/tests/registry_parity.rs:91-144`)

**The terminal.** Raw mode already delivers `Ctrl+Z` as a key
(`crates/postio-tui/src/term.rs:184`), and nothing handles it, so under the one
keymap it is Undo. Spec 005 still claims `Ctrl+Z` suspends the terminal:

- `specs/005-tui-frontend/contracts/tui-surface.md:103-105`;
- `spec.md:360`;
- task T028, marked done.

No SIGTSTP code exists. The same commit corrects those three claims.

**Grouping the key map (screen 20).** A table in postio-ui maps command ids to
Focus's key-map groups. An enumeration test proves every Focus command has a
group. The table holds no bindings, so the one-binding-table check does not
apply.

Rejected: a `group` field on every `CommandSpec`. It would edit every row of
`registry.rs`, the file `feature/contacts` also rewrites.

**Smaller points:**

- `!` needs a punctuation alias (`keymap.rs:196-216`). `[`, `]`, `Delete` and
  `*` are already named keys.
- Shifted keys render the way the shared hint code renders them (spec C22).
- The picker keys are registry commands in `Context::Picker`, so they can be
  rebound and they appear in the key map. They are `1`–`4`, `Tab`, `Space`,
  `Return` and `Escape`.

---

## R5. The command bar

**Decision.** The bar is postio-ui's finder (`crates/postio-ui/src/finder.rs`)
with a new blended mode. Typed text yields three groups, shown in the order
screen 09 draws them:

- commands, from `palette::entries`;
- places: mailboxes, folders, labels and saved searches;
- one "Search mail for …" row.

Each group is ranked within itself. `>` narrows to commands, which is the
finder's existing prefix. The classic finder keeps its unblended modes ("never
blended", `finder.rs:43-44`) unless `/ux-architect` adopts the blend there.

**Plain English, lowered locally.**
`postio_search::natural::lower(text, today, names)` is new, pure and
deterministic:

- A correspondent's name, looked up through a closure the caller supplies over
  the address book, becomes `from:` after "from" or "by", or before "sent" or
  "wrote". After "to", it becomes `to:`.
- A date phrase becomes an `after:`/`before:` pair: last month, yesterday,
  this week, "since Monday", "in August".
- "with attachment(s)" becomes `has:attachment`; "unread" and "flagged" become
  `is:` operators; "in <folder>" becomes `in:`.
- Stop words are dropped, and every other word stays free text.

The output is tokens of the one language, shown as chips through
`postio_ui::search::chips`. The chips are the query (constitution III).
Screen 07 lowers "invoice" to `subject:`. The rules leave it free text, which
already searches subjects; the comparison records the difference.

**Saved searches** are the `[filters]` entries with `pinned = true`, in their
`order` (`crates/postio-config/src/filters.rs:20-47`). Four registry commands
bind them to `Alt+1`–`Alt+4`; nothing binds Alt+N today. `Ctrl+S` is the
existing `SaveSearch`.

**`in:` completion** uses the finder's existing folder data
(`crates/postio-gtk/src/finder.rs:245-268`, moving to postio-ui). A command
acts on the aim the bar opened over; the finder already carries it.

**Built (T084).** `natural::lower` refines three of the rules above:

- A name is a name only when the caller's address book knows it, or when it
  is an address. The longest run of up to three words is tried first.
- "With attachments" becomes `has:attach`, the language's own spelling.
- "In" becomes `in:` only before a mailbox's role ("in archive", "in spam").
  "In Receipts" stays free text, because `in:` naming a folder that does not
  exist selects nothing, and a wrong guess would hide every result. The bar's
  `in:` completion is how a folder gets named.

A bare name, month or weekday stays free text as well: with no word marking it
as a sender or a date, it is as likely to be a subject.

---

## R6. Pickers and dates

**Decision.** postio-widgets gains four popovers anchored to the row
(snooze, remind, label and move) and a shared date entry.

**Presets** come from one table in `crates/postio-ui/src/schedule.rs`, computed
against the clock and the local zone. They reuse `at_local_time`, which is
safe across daylight-saving changes (`schedule.rs:19-24`). The table gains:

| Picker | Presets |
|---|---|
| Snooze | Later today 18:00, Tomorrow morning 08:00, Monday morning 08:00, Next week 08:00 |
| Remind | Tomorrow 09:00, In 2 working days 09:00, End of the week (Friday 09:00), In a week 09:00 |

The existing schedule-send preset "This evening" and Snooze's "Later today"
become one wording for both apps (spec C14; ADR 0029). `/ux-architect`
chooses it.

**Built (T037).** When two pickers mean the same moment (this evening, tomorrow
morning, Monday morning), they call the same function. `schedule_presets` is
rewritten over those functions and returns what it returned before.

C14 is not settled yet. A test holds "Later today" and "This evening" to one
instant at every time of day, so the choice is a change of words; T091 makes
it.

Remind's presets:

- "In 2 working days" counts Monday to Friday, starting the day after today.
- "End of the week" is the first Friday 09:00 still ahead, with the same
  five-minute lead "This evening" has.

The presets step whole days as 24-hour durations, as `main`'s do. Near
midnight in a daylight-saving week, that lands on the wrong day (#1700).

**Typed dates.** `postio_search::date::parse_when(text, now) ->
Option<DateTime<Local>>` is new and public. It looks forward and understands a
time of day: "tue 9am", "thu 2pm", "tomorrow 8", "in 2 days", "oct 3 14:00".
The existing `parse_date` (`crates/postio-search/src/date.rs:24`, crate-private)
resolves only past dates with no time, for queries, and it stays as it is. The
detector's due dates (R10) use `parse_when` too.

**Built (T036).** The function is generic over the zone:
`parse_when<Tz: TimeZone>(text, now: DateTime<Tz>) -> Option<DateTime<Tz>>`.
That lets its tests run in a zone with daylight saving, whatever zone the
machine is in. Its rules:

- A day with no time is 08:00.
- A bare number is an hour on the 24-hour clock.
- A numeric date is month first.
- A time the clocks skip is pushed forward by the gap. A time they repeat is
  its first occurrence still ahead.

The picker shows the instant it read, so a misreading shows before it is used
(T091).

**Snooze takes the chosen time.** `Command::Snooze` gains `until`. Today
`Actions::snooze` always uses three hours (`crates/postio-session/src/actions.rs:84-92`).

**Label and Move:**

- Label is the existing `AddLabel`, which adds or removes
  (`crates/postio-core/src/command.rs:534-547`). `Space` toggles, and a name
  that doesn't exist is created through the host.
- Move is the existing `Move`. Its "Recent" list is the last few destinations,
  kept by the host in the `settings` table. Losing it on a resync is harmless.

The classic app keeps its finder-based label and move, and its fixed snooze,
until `/ux-architect` adopts the pickers there.

---

## R7. Snooze comes back at the top; reminders

**Decision: snooze.** `messages.sort_at` becomes the list's sort key. It
equals `received_at` at insert, and it is set to the wake time when a snooze
wakes (`crates/postio-storage/src/repository/messages.rs:1236`). It replaces
`received_at` in the list's order, its seek marks and its indexes
(`crates/postio-storage/src/schema.rs:693-731`). A woken snooze then comes
back at the top in every app, as screen 11 says; today it returns to its old
place.

**The cheaper alternative the maintainer may choose.** Leave snooze returning
in place, and change screen 11's copy to "comes back to the inbox at that
time". This is the plan's riskiest change to a shared hot path. Spike S6
measures it against the list's counting tests before anything depends on it.

**Decision: reminders.** A `reminders` table records the conversation,
`set_at`, `due_at`, `fired_at` and `cancelled_at`.

- **Set** by `Command::Remind{target, at}`, from the picker or from the
  draft's `remind_at` when it is sent.
- **Cancelled** by the Focus filing pass (R8), when a message from someone
  other than the user arrives in the conversation.
- **Fired** on the engine's five-second tick, the one that wakes snoozes
  (`crates/postio-runtime/src/engine.rs:1229`).

A fired reminder is a surfaced row (R3) at the top of Focus's inbox, marked
"No reply since <date>". While it stands, Focus's inbox scope leaves out that
conversation's ordinary row, so nothing is listed twice. For undo,
`UndoKind::Remind` carries its inverse, `Unremind`, following the snooze
template in `crates/postio-core/src/undo.rs:45-74`.

---

## R8. Classification: where, when, and in what crate

**Decision.** A new crate, `crates/postio-classify`, computes one
fixed-schema answer per message. It works in layers, and an earlier layer's
decision stands:

1. guards;
2. corrections;
3. rules and the built-in detector;
4. the model, in milestone 2.

```text
Outcome {
    filter:  Option<Reason { kind, source }>,
    hold:    Option<DigestRuleName>,
    markers: Vec<MarkerCandidate { kind, span, excerpt, when }>,
}
```

It has no send path. Its boundary rule bans:

- postio-smtp, io-smtp, postio-account, postio-sync, postio-runtime,
  postio-transport, io-imap and io-http;
- every network crate the postio-render rule bans.

It returns only this schema, as ADR 0009 requires.

**Only while Focus runs** (the spec's Clarifications). At startup,
postio-focus switches the host into a Focus mode, which installs three things:

1. **A filing pass for new mail.** `commit_batch`
   (`crates/postio-sync/src/initial.rs:630-713`) calls an optional
   `FilingPass` inside its transaction.
   - It runs only for incremental passes, the ones that emit
     `Event::NewMail`. First syncs never auto-filter (FR-118), and filtering
     years of inbox at first sync is the failure it would otherwise produce.
   - It runs the guards and header rules, writes filter decisions and holds,
     and archives filtered mail through the storage verbs that take the
     caller's transaction (`crates/postio-storage/src/actions.rs:1-35`). It
     enqueues the server move.
   - Its cost per new message is counted and bounded.
2. **A body-stage task.** It follows `spawn_body_indexer`
   (`crates/postio-session/src/lib.rs:1108-1160`): it subscribes to
   `BodyLoaded`, debounces, and runs invitations (R9) and the needs-action
   detector (R10) over bodies that have arrived. At Focus's start it catches
   up over rows with no classification record: newest first, on one core, at
   background priority (FR-141).
3. **A due timer** for digest deliveries and reminders, on the engine's
   five-second tick.

When the classic app or the terminal runs, the host has no Focus mode, and
none of this happens.

**Headers Focus needs before the body.** At filing, only the envelope,
`References` and `List-Id` are known
(`crates/postio-account/src/imap/fetch.rs:213-248`). Every other header
arrives with the body (`crates/postio-index/src/index.rs:565`).

ADR 0025 rejects a header allowlist at header-sync time, and names the way
out: "A header that genuinely must be matchable before the body arrives earns
a dedicated operator, a column, and its own `HEADER.FIELDS` fetch"
(`docs/decisions/0025-arbitrary-headers-are-indexed-rows.md:275-296`).
Focus takes that path for `List-Unsubscribe`, `Precedence` and
`Auto-Submitted`:

- **Fetched** in the header sync's existing `HEADER.FIELDS` item, and only on
  incremental syncs. ADR 0025's objection was the cost to every first sync,
  and a first sync pays nothing here.
- **Stored** as two columns, `messages.unsubscribe_offered` and
  `messages.automation`. For older mail they are filled from the body's own
  headers when it arrives.
- **Searchable** through dedicated operators, `is:bulk` and `is:automated`,
  so search can ask the same question (constitution III).
- **The other backends** ask for the same fields in their header requests:
  `crates/postio-jmap` and `crates/postio-gmail`.

**Guards.** Each is one seek or one lookup:

| Guard | How it is answered |
|---|---|
| The user wrote to the sender | A `correspondents(address_id, sent_count, last_sent_at)` row, maintained at local send (`crates/postio-sync/src/send.rs:525-575`) and when Sent syncs. Without it, a seek on `idx_recipients_address` with kind in (to, cc, bcc) joined to Sent (`schema.rs:562-574`, `:738`). The same row gives completion its "wrote N times" (R15) |
| The user took part in the conversation | `EXISTS` over `idx_messages_thread_mailbox` with the Sent mailbox |
| The user's own domain | The identities' domains, held in memory |
| A pinned sender | `[focus.filter]` in config |

**Automated senders are data** (constitution VII). A TOML table ships with
Postio: patterns on local part and domain, each with a reason and a source
name. The user's corrections override it. None of it is a constant in code.

**The server's own verdict.** `$Junk` in a message's flags
(`crates/postio-model/src/flag.rs:41-44`) gives the reason "spam".

**Rejected:**

- *Classifying in the frontend.* Filing happens in the host.
- *Fetching the whole header block at header sync.* That is ADR 0025's
  rejected allowlist.
- *Filtering at first sync.* It would archive years of inbox.

---

## R9. Invitations

**Decision.** A new crate, `crates/postio-calendar`, is a pure leaf that wraps
**calcard** behind a thin adapter, with default features off.

- `parse(text/calendar bytes) -> Invitation`. The invitation holds the UID,
  SEQUENCE and DTSTAMP, the method, summary, start and end with their zones
  resolved, location, organiser and attendees, and a summary of the
  recurrence.
- `reply(invitation, attendee, partstat) -> ics bytes`, with `METHOD:REPLY`.

Above the adapter sits a small RFC 5546 layer:

- A REQUEST with a higher SEQUENCE replaces the marker, as does one with the
  same SEQUENCE and a later DTSTAMP.
- A CANCEL matching the UID removes the marker's actions.
- An event already over has no actions.

**Pimalaya first** (constitution VII), surveyed 2026-09-27.

- **Pimalaya's `ical-rs` 0.5.1** (2026-09-02, MIT OR Apache-2.0):
  - parses, and expands recurrence;
  - resolves zones from the calendar's own `VTIMEZONE`;
  - but has no IANA or Windows fallback: a TZID sent without its `VTIMEZONE`
    does not resolve;
  - has no iTIP semantics;
  - shipped five breaking releases in its first four weeks.
- **calcard 0.3.14** (Stalwart, 2026-09-15, Apache-2.0 OR MIT):
  - its lenient parser runs in production inside a mail server;
  - it maps Windows and Exchange zone names to IANA;
  - it types `METHOD` and `PARTSTAT`, and has a builder and a writer.
- **Others:** `icalendar` has no `VTIMEZONE` model. `ical` is archived.
  `caldata` enforces the spec too strictly for real mail.

calcard is chosen behind the adapter, and ical-rs is surveyed again before the
branch lands. Caveats:

- Pin the version, and expect 0.4.0 to move to jiff (already in `Cargo.lock`).
- 0.3.14 pulls chrono-tz and mail-builder 1.x, where Postio has 0.5.0.
  Spike S1 checks the graph against `check-dependency-policy.py`.

**The calendar part.** At filing it is only an attachment row, and its
`method=` parameter is dropped (`crates/postio-account/src/backend/message.rs:334-344`).
The body backfill fetches `text/calendar` parts under 256 KiB together with
the text parts (`crates/postio-sync/src/backfill.rs:1509`). Invitations
therefore appear when the body does, and nothing is fetched just to classify.

**RSVP.** `Command::Rsvp{message, answer}`:

- builds a reply to the organiser from the identity that matches an
  `ATTENDEE`; with no match there is no Accept or Decline;
- the reply has a text part and a `text/calendar; method=REPLY` part, which
  `outgoing::build` gains (`crates/postio-model/src/outgoing.rs:88`,
  `:320-340`);
- queues it with `queue_send_at(now + 10 s)`
  (`crates/postio-storage/src/repository/drafts.rs:330`);
- records the pending answer.

**The ten-second window:**

- While the reply is `Queued`, the toast's Undo and `Ctrl+Z` issue
  `CancelSend` (`drafts.rs:469`).
- Once the drainer has taken it, the result is `AlreadyInFlight` and the
  answer stands.
- The window lasts ten to fifteen seconds, because the drainer polls every
  five (`crates/postio-runtime/src/engine.rs:823`).
- The host's undo stack holds an entry for the window that expires with it.
  After that, `Ctrl+Z` reaches the action beneath instead of answering "too
  late", which is the concern `registry.rs:51-70` raises about putting sends
  on the stack.

This is the first real implementation of `Recovery::Window`, which today is
only metadata.

---

## R10. The built-in needs-action detector

**Decision.** The detector is rules in postio-classify. It reads the newest
message's own text: postio-body's extraction with quoted history and the
signature removed, the same boundaries the reader folds.

**It considers only** mail sent directly to the user, with their address in
`To`, and ignores anything that is:

- list or bulk mail (`List-Id`, `is:bulk`);
- automated (`is:automated`, or the automated-senders table);
- from the user.

**What it marks.** Sentences are split first, and clauses are split at `;` and
`—`.

- **A Question** ends in `?` and addresses the reader: in the second person,
  or with an interrogative opening whose subject is "you".
- **A To-do** asks the reader to act: "please", "can/could/would you", "let me
  know", "I need you to" followed by a verb, or an imperative opening.
- **A deadline phrase** ("by Friday", "by Monday, 28 September") becomes a due
  date through `parse_when` (R6).

At most one marker is made per message: the first To-do with a deadline, else
the first Question, else the first To-do.

**Precision over recall.** Pleasantries and rhetorical questions ("How are
you?", "Hope you're well?") are excluded, and when two rules disagree nothing
is marked. It reads English first (spec, Assumptions).

**The gate** is a labelled corpus: fixtures in `crates/postio-model/tests/corpus`
added through `/add-fixture`, with reserved domains and fictional names. It
requires precision of at least 0.9 (SC-013); recall is reported but not gated.
Spike S4 measures rules alone. Only if they miss the bar does the detector
gain a small compiled-in table of weights, which FR-165 allows because it
needs no inference engine.

**What it stores.** Offsets into the extracted text, and an excerpt of the
sentence capped at 200 characters. Dismissals are stored per message. Three
dismissals of the same kind for one sender stop that kind for that sender, and
that stop is written to config as a correction (FR-108).

---

## R11. Colour and type, for two apps with one widget set

**Decision.** The shared widget CSS reads `--postio-*` variables, and each app
defines them.

- **The classic app** keeps defining them from its tokens: `tokens.css`,
  generated from the design system, with its steel accent over libadwaita's
  (`crates/postio-gtk/data/tokens.css:153`, `:247`, `:281`, `:300`).
- **Focus** defines the colour variables from libadwaita's own: accent, view,
  window and card backgrounds, borders and dim labels. The system accent and
  light and dark then arrive through `AdwStyleManager`, as Focus's GTK mapping
  asks (libadwaita 1.6's accent API is inside the `v1_7` features already
  enabled).
- **Metrics tokens** are shared: spacing, radii and chip sizes.

Type: Focus uses the system's Adwaita Sans and Adwaita Mono. The renderer
keeps its bundled fonts for message bodies.

**Rejected:**

- *Focus loads the classic `tokens.css`.* The steel accent is not the system's,
  and screens 01–03 use the system's.
- *Retyping hex values.* ARCHITECTURE §10 forbids it.

**Label colours (T042).** The function is
`postio_ui::label_colour::label_colour(name, stored, accent_hue)`.

- A label with a stored colour, set by the user or by their server, is drawn
  in it.
- Any other label gets one of twelve hues, chosen by a hash of its name, so it
  has the same colour in both apps.
- A hue within 30° of the accent steps round the wheel to the nearest hue
  outside that band. Only those labels move when the accent changes.

FR-091 covers the colours Postio chooses. A colour someone chose is drawn as
they chose it, even inside the band.

---

## R12. Filtering and the Filtered view

**Decision.** Reasons form a fixed vocabulary: spam, promotion, notification,
receipt, shipping and social. Each has an optional source, such as the sender's
name or the list's, and records the layer that decided it: header, sender
table, server verdict, correction or, in milestone 2, the model.

**The view.** Filtered is a list scope over filter decisions joined to their
archived messages, newest first. It counts by reason for the tabs.

**Restore (`R`)** is one undoable unit. It:

- moves the message back to the inbox with the existing `Move`;
- deletes the decision;
- adds the sender to `[focus.filter] keep` in `config.toml`.

Its inverse archives again, restores the decision and removes the config
entry.

**Other rules:**

- **"Filtered today"** counts decisions since local midnight.
- **The sweep** runs the header rules over the current inbox. It shows the
  count first, then archives as one undo unit.
- **Automatic filtering is not on the user's undo stack**, because the user
  did not do it. Its undo is `R`.
- **Filtered mail is archived and never deleted** (Clarifications).

---

## R13. Digests by sender

**Decision.** Rules live in `config.toml` as `[[focus.digests]]`, each with a
name, a query, a cadence, a day and a time. A sender rule's query is
`from:<address>`, in the one language (ADR 0008).

**Matching at filing.** A rule is matched in memory by a matcher for the part
of the language that rules use:

- `from:` in milestone 1;
- `list:` in milestone 2.

ADR 0008's differential test holds that matcher equal to the executor over the
corpus (ADR 0008, Q1). The general matcher on `feature/rules` is not on `main`,
and this plan does not wait for it.

**Holding.** A `digest_holds` table records the message, its rule, when it was
held, and the delivery it belongs to (none yet). Focus's inbox scope leaves
held messages out until their delivery is archived or they are released.
That scope's membership test must change everywhere the list's membership test
(`MEMBER`) appears:

- the window, the representative's `NOT EXISTS`, and the slice;
- the folder count, the boundaries and the rows for changed messages;
- the unified count.

These are in `crates/postio-storage/src/repository/threads.rs:291`,
`:297-307`, `:1220` and `:1443`. Focus's header counts come from counts over
the same scope, not from the per-mailbox triggers, because those count held
mail.

**Delivery:**

- At a rule's due time, the timer creates a `digest_deliveries` row and
  attaches everything held since the last delivery. A delivery with nothing
  in it is not created.
- Each open delivery shows as one surfaced row in the inbox (R3).
- A due time missed while Focus was closed delivers once, at its next start.

**The digest's actions:**

- `⇧A` archives the delivery's messages as one undo unit.
- `D` removes the sender from the rule in config; that sender's future mail
  goes to the inbox.
- Removing a rule releases what it held.

**Preview (screen 24):** the rule's query through the executor over the last
90 days, counted, with its first four rows.

**`g d`:** the rules list, a full view in screen 21's frame (spec C15),
designed with `/ux-architect` before its task.

**Built (T131).** The matcher is `postio_search::matcher::Matcher::new(&ParsedQuery)`,
which returns `Unsupported` for anything beyond `from:` and `list:`.
`crates/postio-index/tests/index_suite/digest_matcher.rs` holds it equal to
the executor on 35 queries over the corpus.

The executor has a bug: `from:<address>` also finds mail *sent to* that
address (#1699). The matcher deliberately repeats it, because ADR 0008 makes
agreement the first test. So a fix for #1699 changes both in one commit.

---

## R14. Store and config: what lives where

A schema change makes the user resync into a fresh store (R0). What the user
decides lives in `config.toml`. The store holds what can be recomputed or
re-entered.

**Store:**

- `messages.sort_at`, `messages.unsubscribe_offered` and `messages.automation`;
- `markers`;
- `filter_decisions`;
- `digest_holds` and `digest_deliveries`;
- `reminders`, which are lost on a resync, as snoozes are;
- `correspondents`;
- `focus_classified`: what the catch-up has done, by stage and classifier
  version;
- `egress_log.subsystem` gains `'model'` in milestone 2.

**Config:**

- `[focus]`: `filtering`;
- `[focus.filter]`: the `keep` and `pinned` senders;
- `[[focus.digests]]`;
- the marker kinds stopped per sender;
- `[focus.model]` in milestone 2, and `[focus.vault]` in milestone 3.

The list reads markers with one extra batched statement per page, on the
pattern of `participants_for` (`threads.rs:1561`), and only for Focus scopes.
The classic list's statement counts do not change. The full shapes are in
[data-model.md](./data-model.md).

---

## R15. Compose in a dialog

**Decision.** A `DialogHost` implements `ComposerHost` (R1):

- the context is Composer while the dialog is open;
- the dialog is the parent of file dialogs;
- autosave is on;
- the window's resolver serves keys.

**The draft gains two fields,** `labels` and `remind_at`
(`crates/postio-model/src/draft.rs:122-197`). When the draft is sent, the host
applies the labels to the Sent copy's conversation and creates the reminder.

**Recipient chips** are an opt-in presentation of the shared composer's fields
(spec FR-052). Focus turns them on. Whether the classic app does is a
`/ux-architect` call.

**"wrote N times"** is `correspondents.sent_count` (R8), carried in the
`RecipientDirectory` rows (`crates/postio-client/src/protocol.rs:600`).
Completion ranks by it, one rule for both apps: sent count, then last seen,
then times seen. Today it ranks by the store's order, and `times_seen` counts
any header, not letters written (`crates/postio-sync/src/contacts.rs:36-62`).

**Unchanged or dropped:**

- The Markdown toggle on screen 05 is dropped (spec C7).
- "Send later" is the existing schedule path (`composer.rs:1461`, `:3154-3193`).

---

## R16. Milestones 2 and 3, designed so milestone 1 leaves room

**postio-ai**, the crate ADR 0009 named, is a client for the user's local
runtime.

- **Interface:** the OpenAI-compatible chat completions both runtimes serve,
  Ollama at `127.0.0.1:11434/v1` and llama.cpp's server at
  `127.0.0.1:8080/v1`. Requests use
  `response_format: {type: "json_schema", json_schema: {name, schema}}`.
- **Schemas** stay flat, and responses are validated on the client.
- **Safety:** endpoints must be loopback addresses or local sockets. Every
  call goes to the egress log. The crate has no send path, which the boundary
  check enforces.
- **Role:** it implements postio-classify's model layer and the digest
  summariser.

**The digest summary** is made of statements, each with references, and each
reference carries a message and an excerpt.

- A reference must resolve in its message when the summary is written (a byte
  search of the extracted text) and when it is shown (`TextIndex::find`).
- A statement whose reference does not resolve is dropped.
- The summary is plain text.

**postio-vault** writes Obsidian Tasks lines:
`- [ ] <text> [✉](postio://message/<id>) 📅 YYYY-MM-DD`.

- The link goes before the date: the Tasks plugin reads its fields from the
  end of the line, and allows only tags and block ids after them (Tasks
  8.4.0, `DefaultTaskSerializer`).
- 📅 is U+1F4C5. A finished task reads back as `- [x] … ✅ YYYY-MM-DD`.

**`postio://`** is registered in Focus's desktop file as
`x-scheme-handler/postio`.

- The classic app already sets `HANDLES_OPEN` for `mailto:`
  (`crates/postio-gtk/src/app.rs:130`).
- A `postio:` URI arrives as a GFile whose `uri()` carries it.
- It navigates and never acts, because any page or app can fire one.

---

## R17. Testing

**Fast (`--lib`):**

- postio-classify: rules, the detector, and guards over fixtures;
- postio-calendar: invitation fixtures;
- postio-search: `natural` and `parse_when`;
- postio-ui: key-map groups, presets, splice positions and label colours.

**Counting** (`crates/postio-storage/src/test_support/counting.rs`):

- a Focus scope page is one statement, plus one for markers, with no scans;
- the filing pass's statements per new message are bounded;
- the Focus counts are counted.

**Differential:** the digest matcher against the executor, over the corpus.

**Registry:**

- enumeration across frontends (SC-015);
- Focus parity: every Focus command has a key, a command-bar row and a
  visible control. The pattern is `crates/postio-tui/tests/registry_parity.rs`.

**Integration:** `crates/postio-focus/tests/focus_suite/` is one binary on the
`app_suite` custom harness, with `CASES`, `IGNORED` and the list contract, on
the headless compositor. Each user story's acceptance scenarios are cases that
assert on the widget tree.

**Screens:** `shot` renders every screen from 01 to 20 from the demo store, in
light and dark. Each comparison with its PNG is recorded in
`specs/007-postio-focus/screens.md`, with its differences and their reasons.

**Nightly:** SC-011's first pass, at 100,000 messages, under a
`POSTIO-MEASUREMENT:` marker in `.config/nextest.toml`.

---

## R18. Risks, and the spikes that settle them first

| Spike | Question | Decides |
|---|---|---|
| S1 | calcard on invitation fixtures (Outlook, Google, Apple, Zoom; zones; updates; cancellations; recurrence), its graph and its licence | calcard, or ical-rs behind the same adapter |
| S2 | The promoted headers' bytes per new message, on a real account | R8's header promotion |
| S3 | A two-height `GtkListView` with spliced rows at 100,000 conversations: scrolling, jumping, rows built per frame | R3's list |
| S4 | The detector's precision on the labelled corpus, rules alone | R10's rules, or rules with a small table of weights |
| S5 | Highlighting by excerpt across the render corpus | R2's highlight |
| S6 | `sort_at` against the list's counting tests | R7's snooze, or its cheaper alternative |
