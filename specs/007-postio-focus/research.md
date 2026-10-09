# Research: Postio Focus

Phase 0 for [plan.md](plan.md). Each section is a finding that constrains
Focus's design and the decision it led to. A rejected alternative is kept
only where it stops the question being argued again. Paths are
repository-relative.

---

## R0. What Focus is built on

**The reading renderer.** Message bodies are drawn by `crates/postio-render`
(spec 006, ADR 0042): a Blitz-based engine with no toolkit, no network and
no C in its graph. It runs one render thread per reader and returns an
immutable snapshot: a display list, a `TextIndex`, and link, message, fold and
anchor boxes. `BodyView`, a `GtkScrollable` in `postio-widgets`, paints that
snapshot as tiles and supplies selection, find, links, accessibility and
zoom.

**The store migrates where it can, and starts over where it cannot.** A
schema change comes with a `Migration` from the stamp it replaces
(`postio_storage::schema::MIGRATIONS`); a store whose stamp no migration
reaches is refused with a fresh store as the way forward
(`postio_session::start_over`), which sets the old one aside and keeps the
accounts and `config.toml`
(`docs/notes/2026-10-01-store-migrations-and-starting-over.md`). What exists
only in the store is lost when it starts over. That decides R14: what the
user decides lives in `config.toml`, and the store holds what can be
recomputed or re-entered.

---

## R1. The shared GTK crate is `postio-widgets`

**Decision.** `crates/postio-widgets` holds the GTK Focus draws outside its
own window and the presenters that join it to `postio-client`; it depends on
no app. The rule, what lives there and the boundary are
[ADR 0043](../../docs/decisions/0043-focus-is-the-one-desktop-app.md).

It holds:

- **`body_view/`**: `BodyView`, tiles, interaction, find, zoom and
  accessibility;
- **the reader**: `Reader`, its banner, notices and attachment chips. The
  verb bars `Reader` draws are configuration, and Focus passes none: its
  open message has its own action row. `Reader::flow_in` makes the view as
  tall as its document inside an outer scroller (R2);
- **the composer**, behind a `ComposerHost` trait for the pane or dialog it
  lives in, the file dialog's parent, the keymap in force and autosave. It
  still edits in WebKit, because ADR 0039's native editor is decided and not
  built, so `postio-widgets` carries webkit6 for the composer alone;
- **the small widgets**: keyhint, keycap, action bar, buttons (the one close
  button, the one icon button), chip, notice, toast, label pills, the
  row-anchored pickers and the typed-date entry;
- **the list model**: a `GListModel` over `postio-ui`'s `ListWindow`,
  generic over its row type;
- **the presenters**: composing, reading (the blob source and the remote-image
  fetcher), configuration, credentials and adding an account, onboarding,
  settings, export (drag-out), and the editor launcher;
- **the settings window**, the window-state file and drag-out.

**Boundary.** It may depend on gtk4, libadwaita, webkit6, `postio-render`,
`postio-ui`, `postio-core`, `postio-body`, `postio-config`, `postio-model` and
`postio-client`. It may not depend on the store engine or the protocol, on
`postio-host`, `postio-session`, `postio-runtime`, `postio-storage` or
`postio-sync` (presenters reach mail through `postio-client`), or on any app.
`check-crate-boundaries.py` enforces it.

**The checks that scan GTK code** (`check-key-hints-are-derived.py`,
`check-buttons-have-a-kind.py`, `check-no-dead-css.py`,
`check-shadows-use-tokens.py`, `check-spacing-literals-ratchet.py`,
`check-reader-header-has-one-home.py`, `check-blocking-now-sites.py`,
`check-uncalled-pub-fn.py`) scan `postio-widgets` and `postio-gtk`.

**One view, reused.** `BodyView` drops a message's selection, focused link
and toggled folds when the next is set, because the open message reuses one
view for every message.

---

## R2. The message view

**Decision.** Focus's open message hosts the shared `Reader` in single-message
mode, and one `Reader` serves every open, in the dialog or the pane.

**Layout.** The open message owns its header and action row. Below them is
one scrolling column: thread marker, subject, labels, sender block, action
card, render-mode line, body, attachments and fold line scroll together.
`BodyView::flow_in` makes the view as tall as its document and reads its
visible window from the outer scroller's adjustment, so tiling stays
windowed. The cards are GTK widgets, not HTML chrome in the document, so
their buttons stay real buttons. The frame, column and treatments are
[screens.md](screens.md), "The open message".

**Highlighting a sentence (FR-035, screen 23).** `TextIndex::locate(Excerpt {
text, offset, source_len })`, in `postio-render` beside `find`, marks a
character range drawn as the find overlay is, and scrolls to it.

- The range is found from the stored excerpt, not the stored offsets: the
  index's text is in laid-out reading order, with whitespace collapsed,
  `alt` text included and closed folds left out.
- Matching uses `find`'s folding (case and diacritics), with any run of
  whitespace matching any other on both sides: blocks and table cells are
  line breaks and tabs in the index, where a flattened source has spaces.
- When the excerpt appears more than once, `offset` and `source_len` pick
  the occurrence nearest its proportional position.
- **Measured** over the render corpus (69 fixtures, about 1,000 sentences a
  detector could store): read from what is drawn, 99.5% of sentences are
  found; read from the text part first, 97.3%. The tiebreak picks the right
  copy 1,979 times in 1,980; the miss is a line of emoji.
  `crates/postio-render/tests/excerpt_locate.rs` holds these floors on the
  nightly profile, and they are measured again when the corpus grows.

So the marker's writer stores a plain prefix of the sentence, at most 200
characters, with no ellipsis (which would stop it being found), and
`source_len` is the length of the own text it was cut from.

**Own text.** `postio_body::own_text(&MessageBody)` is what the detector reads
and what offsets point into:

1. it sanitises the HTML;
2. drops the quoted stretches, with the detector the reader folds by;
3. leaves out text hidden inline and `alt` text;
4. flattens what remains, reading what is drawn, not a `text/plain`
   alternative that may say something else;
5. splits off the signature.

It also stops at Outlook's "Original Message" line and underscore rule, a
forward banner, a `From:` over `Sent:` or `Date:` block, "Sent from my", and
an attribution over unquoted text, and ends at a bare closing line. Text
hidden by a class in the sender's own stylesheet is not seen.

**Quote folds.** Single-message documents give their folds ids (`q0`, `q1`,
…), and every fold is labelled "N quoted lines", counting lines that hold
words.

**Raw source (`v`).** `ViewSource` shows the stored raw message
(`messages.raw_blob_id`). When the raw blob is not local, it is fetched on
that deliberate key press.

**Images and links.** Remote images need the sender's consent, once (`i i`)
or always (`i a`); the remote-image allowlist is loaded once per app and
shared by its readers, and also keeps each sender's chosen treatment. Links
open only on activation, with the target shown first. `o` offers the
snapshot's links and the message's parts in a chooser, which also saves
attachments.

---

## R3. Focus as a frontend: one crate, on the terminal's pattern

**Decision.** One crate, `crates/postio-gtk`, holds the binary, the window
and its rows and dialogs, over `postio-host` and `postio-client`, as the
terminal is. It reads through the client in-process, so it needs no split
between a view crate and a store-reading crate.

**Startup.**

- The store opens on its own thread behind a window that already exists,
  saying what it waits for.
- `Host::start`, then `connect(ClientKind::Focus)`.
- If another app holds the store, Focus shows the shared sentence with "Try
  again". A store no migration reaches offers "Start a fresh store" (R0).
  Every page before the inbox can be closed (its close button, `mod+q`,
  `mod+w`).

**Events.** There is exactly one reader of `client.events()`, pumped into the
GTK main context: clones of the receiver compete for events.

**The list.**

- It is built from `postio-ui`'s `ListWindow`, `Paging` and `SelectionState`
  (pages of 50, eight cached), over the shared list model (R1).
- Rows are Focus's own widgets, each one custom `snapshot()`.
- There are exactly two heights: **40 px** for one line and **72 px** for
  two. A row's kind, marker or no marker, decides its height; its content
  never does.
- **Measured** (a two-height `gtk::ListView` over 100,000 rows plus 50
  spliced, headless, at 1440×900): scrolling binds at most 11 rows a frame at
  40 px a frame and 27 at 800; a jump binds up to 205. GTK estimates the
  list's height 5% short. So a bind is cheap, a row draws a skeleton while
  its page lands, and pages are cached.

**Surfaced rows.** Digest deliveries (R13) and fired reminders (R7) are rows
that are not conversations. There are few of them. Each is spliced into the
window at the position given by the number of conversations newer than it:
one bounded count per surfaced row, cached against the list's witness. A SQL
`UNION` of conversations and surfaced rows would touch every place the
list's membership test appears.

**Config.** Focus's settings are `[focus]` (contracts/config.md).
`ConfigChanged` carries a `focus` flag, and Focus runs the shared config
service and watcher, so every section it follows reloads live.

**Packaging.** Focus is the desktop Flatpak's app. Until the package switch
(T253) it is a second launcher in it: app id `dev.postio.Postio.Focus`,
desktop file `dev.postio.Postio.Focus.desktop`, binary `postio-gtk`. At the
switch it takes the name "Postio", the binary `postio`, the app id
`dev.postio.Postio`, the icon and the `mailto:` handler (ADR 0043). It
already draws the package's one icon, `dev.postio.Postio`.

**Screens against PNGs.** `cargo run -p postio-gtk --example shot -- <png>
<screen>` renders a named screen from a seeded demo store: `postio_storage::seed`
plus Focus's markers, digests and filter decisions, written through the host.

---

## R4. One keymap for every app

**Decision.** `KEYS.md` is the registry's defaults. The registry keeps one row
per command; a command only another app offers keeps a key that does not
collide. The table is [contracts/keymap.md](contracts/keymap.md).

**Frontend availability.** `Availability.frontend: Frontend {Classic,
Terminal, Focus, Macos}`:

- `Requirement::Terminal` and `Requirement::Graphical` keep their meaning;
- `Requirement::Focus` marks commands only Focus offers: invitations,
  has-action, Filtered, digests, remind, the treatments and the reading pane;
- `Requirement::ThreePane` means "not Focus": the sidebar, pane cycling, the
  conversation rail and the parts panel, which the terminal and macOS keep.

Each app builds its resolver with `Resolver::from_commands_for(keymap,
Frontend)`, so a key kept for Focus does nothing elsewhere.

**Contexts.** `Picker`, `Digest`, `Filtered` and `Capture` join the set. The
open message is `Context::Reader`; its fallback chain (Reader → Conversation
→ List → Global) is what lets `j`/`k` step the list from inside it.

**The terminal.** Raw mode delivers `ctrl+z` as a key, so under the one
keymap it is Undo. The terminal never suspended on it.

**Grouping the key map (screen 20).** A table in `postio-ui` maps command ids
to Focus's key-map groups, and an enumeration test proves every Focus command
has one. The table holds no bindings, so the one-binding-table check does not
apply. A `group` field on every `CommandSpec` was the alternative; the table
keeps a presentation concern out of the registry's rows.

**Smaller points:**

- `!` has a punctuation alias in `postio-ui`'s keymap; `[`, `]`, `Delete` and
  `*` are named keys.
- The picker keys are registry commands in `Context::Picker`, so they can be
  rebound and they appear in the key map: `1`–`4`, `Tab`, `Space`, `Return`
  and `Escape`.

The enumeration test (SC-015) is the arbiter: a key it rejects is changed
there, not argued.

---

## R5. The command bar

**Decision.** The bar is `postio_ui::finder::blend(text, places, keymap,
context, availability)`: typed text yields, in the order screen 09 draws
them, the commands it matches (from `palette::entries`), places (mailboxes,
folders, labels and saved searches, each with the key that goes there), and
one "Search mail for …" row. Each group is ranked within itself, and `>`
narrows to commands. A command acts on `finder::Held { scope, selection,
cursor }`: the rows marked when the bar opened, else the cursor's row.

**Plain English, lowered locally.**
`postio_search::natural::lower(text, today, names)` is pure and
deterministic:

- A name is a correspondent only when the caller's address book knows it, or
  when it is an address; the longest run of up to three words is tried first.
  It becomes `from:` after "from" or "by" or before "sent" or "wrote", and
  `to:` after "to".
- A date phrase becomes an `after:`/`before:` pair: last month, yesterday,
  this week, "since Monday", "in August".
- "With attachment(s)" becomes `has:attach`; "unread" and "flagged" become
  `is:` operators.
- "In" becomes `in:` only before a mailbox's role ("in archive", "in spam").
  A folder name stays free text, because `in:` naming a folder that does not
  exist selects nothing and a wrong guess would hide every result; the bar's
  `in:` completion is how a folder gets named.
- A bare name, month or weekday stays free text: with no word marking it as
  a sender or a date, it is as likely to be a subject. Stop words are dropped.

The output is tokens of the one language, shown as chips; the chips are the
query (constitution III). "Invoice" stays free text, which already searches
subjects.

**Saved searches** are the `[saved_searches]` entries with `pinned = true`, in their
`order`, bound to `alt+1`–`alt+4` by four registry commands. `mod+s` is
`SaveSearch`.

**`in:` completion** uses the finder's folder data. A misspelling is
answered with "Search instead for …" (ADR 0037), and `O` switches the results
between relevance and date.

---

## R6. Pickers and dates

**Decision.** `postio-widgets` has four popovers anchored to the row (snooze,
remind, label and move) and a shared date entry.

**Presets** come from one table in `postio-ui/src/schedule.rs`, computed
against the clock and the local zone with `at_local_time`, which is safe
across daylight-saving changes. When two pickers mean the same moment (this
evening, tomorrow morning, Monday morning), they call the same function, so
Snooze and Send later say "Later today", and "Tomorrow evening" after 6pm
(C14).

| Picker | Presets |
|---|---|
| Snooze | Later today 18:00, Tomorrow morning 08:00, Monday morning 08:00, Next week 08:00 |
| Remind | Tomorrow 09:00, In 2 working days 09:00, End of the week (Friday 09:00), In a week 09:00 |

"In 2 working days" counts Monday to Friday, starting the day after today.
"End of the week" is the first Friday 09:00 still ahead, with the five-minute
lead "Later today" has. The presets step whole days as 24-hour durations, so
near midnight in a daylight-saving week one can land on the wrong day
(#1700).

**Typed dates.** `postio_search::date::parse_when<Tz: TimeZone>(text, now:
DateTime<Tz>) -> Option<DateTime<Tz>>` looks forward and understands a time
of day: "tue 9am", "thu 2pm", "tomorrow 8", "in 2 days", "oct 3 14:00". It is
generic over the zone so its tests run in a zone with daylight saving. A day
with no time is 08:00; a bare number is an hour on the 24-hour clock; a
numeric date is month first; a time the clocks skip is pushed forward by the
gap, and one they repeat is its first occurrence still ahead. The picker
shows the instant it read, so a misreading shows before it is used. The
query parser's `parse_date`, which resolves only past dates with no time,
is a different function for a different job.

**Snooze takes the chosen time**: `Command::Snooze` carries `until`.

**Label and Move.** Label is `AddLabel`, which adds or removes; `Space`
toggles, and a name that does not exist is created through the host. Move is
`Move`; its "Recent" list is the last few destinations, kept by the host in
the `settings` table (`focus.move_recent`).

---

## R7. Snooze comes back at the top; reminders

**Snooze.** `messages.sort_at` is the list's sort key. It equals
`received_at` at insert and becomes the wake time when a snooze wakes, so a
woken snooze comes back at the top in every app, as screen 11 says.

- **What sorts by it:** the folder and conversation lists, Focus's window
  and its cursor, their seek marks, and the indexes `idx_messages_list` and
  `idx_messages_thread_mailbox`.
- **What stays on `received_at`:** the query views (Account, Flagged,
  Snoozed, Outbox, the flat Unified read, Thread) and search.
- **Drafts keep rising:** `write_update` keeps `sort_at` at least
  `received_at`, so a re-saved draft still rises in Drafts.
- **Raw test inserts must name the column**, which is `NOT NULL`.

Measured before it was adopted, the list's counting tests stayed green
unchanged, which is why returning in place with changed copy was not chosen.

**Reminders.** A `reminders` table records the conversation, the anchor
message, `set_at`, `due_at`, `fired_at`, `cancelled_at` and `settled_at`.

- **Set** by `Command::Remind{target, at}`, from the picker or from the
  draft's `remind_at` when it is sent.
- **Cancelled** by Focus's filing pass (R8), when a message from someone
  other than the user arrives in the conversation.
- **Fired** on the engine's five-second tick, the one that wakes snoozes.

A fired reminder is a surfaced row (R3) at the top of Focus's inbox, marked
"No reply since <date>". While it stands, Focus's inbox scope leaves out that
conversation's ordinary row, so nothing is listed twice. `UndoKind::Remind`
carries its inverse.

---

## R8. Classification: where, when, and in what crate

**Decision.** `crates/postio-classify` computes one fixed-schema answer per
message, in layers where an earlier layer's decision stands: guards,
corrections, rules and the built-in detector, then the user's model. It has
no send path; its boundary bans the mail transports, sync, the runtime,
`io-imap`, `io-http` and every network crate. The output schema is in
[data-model.md](data-model.md) and the interface in
[contracts/engine.md](contracts/engine.md).

**Only while Focus runs.** At startup Focus calls `Host::enable_focus`, which
installs:

1. **A filing pass** for new mail, run inside an incremental pass's write
   unit (`resync::incremental`), and for servers with no MODSEQ inside the
   enumeration pass that inserted the rows. It runs the guards and header
   rules, writes filter decisions and holds, and archives filtered mail with
   its server move, in its own savepoint, so a failure rolls back only what
   it wrote. **First syncs never auto-filter**: filtering years of inbox at a
   first sync is the failure it would otherwise produce (FR-118).
2. **A body-stage task** on the `spawn_body_indexer` pattern: it follows
   `BodyLoaded`, debounces, and runs invitations (R9) and the needs-action
   question (R10) over bodies that have arrived. At Focus's start it catches
   up over rows with no classification record, newest first, on one core, at
   background priority (FR-141).
3. **A due timer** for digest deliveries, reminders and RSVP windows, on the
   engine's five-second tick.

When another app runs, the host has no Focus mode, and none of this happens.
Classifying in the frontend was the alternative; filing happens in the host.

**Headers Focus needs before the body.** At filing, only the envelope,
`References` and `List-Id` are known; every other header arrives with the
body. ADR 0025 names the way out for a header that must be matchable before
the body: a dedicated operator, a column, and its own `HEADER.FIELDS` fetch.
Focus takes it for `List-Unsubscribe`, `Precedence` and `Auto-Submitted`:

- **Fetched** only on incremental syncs, in a `HEADER.FIELDS` item of their
  own in the same FETCH, so a first sync pays nothing (ADR 0025's objection
  was the cost to every first sync). Gmail reads them from the metadata it
  already fetches. JMAP learns them from the body, because io-jmap 0.3 cannot
  ask for a single header.
- **Stored** as `messages.unsubscribe_offered` and `messages.automation`, and
  filled from the body's own headers for older mail.
- **Searchable** as `is:bulk` and `is:automated` (constitution III).

**Guards.** Each is one seek or one lookup:

| Guard | How it is answered |
|---|---|
| The user wrote to the sender | A `correspondents` row, maintained at local send and when Sent syncs. The same row gives completion its "wrote N times" (R15) |
| The user took part in the conversation | `EXISTS` over `idx_messages_thread_mailbox` with the Sent mailbox |
| The user's own domain | Every account's and identity's addresses, read in one statement |
| A pinned or restored sender | `[focus.filter] never` |

A message with no From address or no `thread_id` is guarded and never
filtered.

**Automated senders are data** (constitution VII):
`crates/postio-classify/data/senders.toml`, patterns on local part and domain,
each with a reason and a source name, validated by `build.rs`. The user's
corrections override it.

**The server's own verdict.** `$Junk` in a message's flags gives the reason
"spam".

---

## R9. Invitations

**Decision.** `crates/postio-calendar` is a pure leaf that wraps **calcard**
(pinned, default features off) behind a thin adapter:

- `parse(text/calendar bytes) -> Invitation`: UID, SEQUENCE and DTSTAMP, the
  method, summary, start and end with their zones resolved, location,
  organiser and attendees, and the recurrence;
- `reply(invitation, attendee, answer) -> ics bytes`, a `METHOD:REPLY` with
  only the answering attendee;
- `supersedes(newer, older)`: the same UID and occurrence, then SEQUENCE,
  then DTSTAMP; false when it cannot tell.

A REQUEST that supersedes replaces the marker, a CANCEL matching the UID
removes its actions, and an event already over has none.

**Why calcard.** Pimalaya was surveyed first (constitution VII). Its
`ical-rs` resolves zones only from the calendar's own `VTIMEZONE`, so a TZID
sent without one does not resolve, and it has no iTIP semantics; calcard maps
Windows and Exchange zone names to IANA, types `METHOD` and `PARTSTAT`, and
runs in production inside a mail server. calcard read all nine corpus
invitations (Outlook, Google, Apple, Zoom and Thunderbird styles; Windows
zones, a zone with no `VTIMEZONE`, an update, a cancellation, a weekly rule
with EXDATEs across a DST change) with the right instants. `ical-rs` is
surveyed again before the branch lands.

**The calendar part.** At filing it is only an attachment row. The body
backfill fetches `text/calendar` parts of 256 KiB or less with the text
parts, so invitations appear when the body does, and nothing is fetched just
to classify.

**RSVP.** `Command::Rsvp{message, answer}`:

- builds a reply to the organiser from the identity that matches an
  `ATTENDEE`; with no match there is no Accept or Decline;
- gives it a text part and a `text/calendar; method=REPLY` part, through
  `outgoing::build`'s `calendar: Option<CalendarPart { method, ics }>`, which
  goes last in the `multipart/alternative` (with `None`, the output is
  byte-identical to a message without it);
- queues it with a not-before time ten seconds ahead; an RSVP is never filed
  as a draft;
- records the pending answer.

**The ten-second window.** While the reply is queued, the toast's Undo and
`mod+z` issue `CancelSend`. Once the drainer has taken it, the answer stands.
The window lasts ten to fifteen seconds, because the drainer polls every five.
The host's undo stack holds an entry for the window that expires with it, so
after it `mod+z` reaches the action beneath. This is `Recovery::Window`.

---

## R10. The built-in needs-action detector

**Decision.** The detector is rules in `postio-classify`, over the newest
message's own text (R2).

**It considers only** mail sent directly to the user, with their address in
`To`, and ignores list and bulk mail (`List-Id`, `List-Unsubscribe`,
`is:bulk`), automated mail (`is:automated`, the senders table), `$Junk`, mail
from the user, and mail in Junk, Sent, Drafts, Outbox or Trash. An unknown
header fact counts as no evidence either way.

**What it marks.** Sentences are split, and clauses at `;` and `—`.

- **A Question** ends in `?` and addresses the reader: in the second person,
  or with an interrogative opening whose subject is "you". A sentence ending
  in `?` is a Question even when it asks the reader to act, and carries no
  due date.
- **A To-do** asks the reader to act: "please", "can/could/would you", "let me
  know", "I need you to" followed by a verb, or an imperative opening.
- **A deadline phrase** ("by Friday", "by Monday, 28 September") becomes a due
  date through `parse_when` (R6), read in the local zone from the Date header
  (else `received_at`). "End of day" and "today" are 18:00 that day, "end of
  the week" Friday 18:00, "end of the month" its last day at 18:00. "Until" is
  not a deadline, and "on Monday" says when, not by when.

At most one marker is made per message: the first To-do with a deadline, else
the first Question, else the first To-do.

**Precision over recall.** It does not mark pleasantries and rhetorical
questions, boilerplate that reads as a request ("Let me know if you have any
questions"), an ask put to somebody else by name, "Check out …", a "please"
inside a signature, or text addressed to an assistant. A greeting in front of
an imperative does not hide it. When two rules disagree, nothing is marked.
It reads English first.

**The gate.** `crates/postio-classify/tests/data/needs_action.toml`, invented
items with reserved domains, requires precision of at least 0.9 (SC-013);
recall is reported. On its 201 items (44 questions, 35 to-dos, 122 with no
ask) the rules reach precision 0.985 and recall 0.823 with no table of
weights, a marker counting as right only when its kind and its quoted
sentence both match. The rules and the dataset share authors, so that is a
best case. Most misses are questions with no "you", which the second-person
rule declines by design.

**Instructions aimed at a machine.** The `untrusted-instructions` corpus
fixture (category `prompt-injection`, ADR 0009 Q4) holds one honest question
followed by instruction-shaped and tool-shaped text. Neither detector marks
such a message, and the outcome holds none of its words.

**How markers are written.**

- The detector returns offsets into the own text; whoever writes the marker
  cuts the excerpt (at most 200 characters) from the own text by them.
- A detector's marker is never overwritten when the message is classified
  again.
- A question or to-do in held mail releases a hold still waiting, never a
  delivered one.
- Every message carrying the same invitation UID and occurrence shows the
  newest word, decided by `supersedes`; a marker already showing it is left
  alone, so an answer survives.
- Dismissals are stored per message. Three dismissals of one kind for one
  sender stop that kind for that sender, written to `[focus.filter]
  stop_markers` (FR-108).

---

## R11. Colour and type

**Decision.** The shared widget CSS reads `--postio-*` variables, and Focus
defines them from libadwaita's own (accent, view, window and card
backgrounds, borders, dim labels), so the system accent and the light and
dark schemes arrive through `AdwStyleManager`. Metrics (spacing, radii, chip
sizes) are shared tokens; no hex value is retyped (ARCHITECTURE §10). The
open message adds the handoff's surface, ink, hairline and scrim values for
those roles, scoped to `.focus-open`, and the accent's soft fill (C26).

**Type.** The chrome is Adwaita Sans and Adwaita Mono (C25). The renderer
keeps its bundled fonts for message bodies, and a body in app colours is set
in Barlow.

**Label colours.** `postio_ui::label_colour::label_colour(name, stored,
accent_hue)`:

- a label with a stored colour, set by the user or by their server, is drawn
  in it, even near the accent;
- any other label gets one of twelve hues, chosen by a hash of its name;
- a hue within 30° of the accent steps round the wheel to the nearest hue
  outside that band, so only those labels move when the accent changes.

---

## R12. Filtering and the Filtered view

**Decision.** Reasons form a fixed vocabulary: spam, promotion, notification,
receipt, shipping and social. Each has an optional source, such as the
sender's name or the list's, and records the layer that decided it: header,
sender table, server verdict or model.

**The filing pass** (`postio_sync::FocusFiling`) gives a message its reason in
this order:

1. the server's `$Junk`: spam, from the server layer;
2. the automated-senders table: the table's own reason;
3. the headers: `Auto-Submitted` is a notification; `Precedence: bulk` or
   `junk` is a promotion; `List-Unsubscribe` without `List-Id` is a
   promotion.

Everything else stays in the inbox, discussion lists included (`List-Id` with
`Precedence: list`). Only mail arriving in the inbox is filed away. A hold
beats a filter: held mail is never also filed away, and the first matching
rule holds it. An invitation is never held, and is filtered only by `$Junk`.
A domain in `[focus.filter] never` (`@example.com`) matches that exact domain,
not its subdomains. A message no rule acts on costs no reads: the store's
facts are asked only when a rule would file something away.

**The view.** Filtered is a list scope over standing filter decisions joined
to their archived messages, newest first, counted by reason for the tabs.

**Restore (`R`)** is one undoable unit: it moves the message back to the
inbox, marks the decision `restored_at`, and adds the sender to
`[focus.filter] never`. Its inverse archives again, clears `restored_at` and
removes the sender from `never` unless another restore from that sender still
stands.

**Other rules:**

- **"Filtered today"** counts decisions since local midnight.
- **The sweep** (`F`) runs the header rules over the current inbox, shows
  the count first, then archives as one undo unit.
- **Automatic filtering is not on the user's undo stack**, because the user
  did not do it. Its undo is `R`.
- **Filtered mail is archived and never deleted.**
- **Catching up on open** sorts inbox mail past `focus.filed_through` that
  has no filing record, newest first. The row under any app's cursor stays
  where it is and is recorded as seen. Focus's first-ever open only sets the
  mark (FR-118).

---

## R13. Digests

**Decision.** Rules live in `config.toml` as `[[focus.digests]]`, each with a
name, a list of queries, a cadence, a day and a time. A sender rule's query is
`from:<address>`, in the one language (ADR 0008).

**Matching at filing.** `postio_search::matcher::Matcher::new(&ParsedQuery)`
matches the part of the language rules use (`from:`, `list:`, `to:`,
`subject:`, `filename:`) in memory, and returns `Unsupported` for anything
else, so a rule the filing pass cannot answer is refused when saved. ADR
0008's differential test holds it equal to the executor over the corpus
(`crates/postio-index/tests/index_suite/digest_matcher.rs`). The executor's
`from:<address>` also finds mail sent to that address (#1699), and the matcher
deliberately agrees with it, so a fix changes both in one commit.

**Holding.** `digest_holds` records the message, its rule, when it was held,
and its delivery (none yet). Focus's inbox scope leaves out held messages
until their delivery is archived or they are released. That scope's
membership test is applied everywhere the list's is: the window, the
representative's `NOT EXISTS`, the slice, the counts, the boundaries, the
rows for changed messages and the unified count. Focus's header counts come
from counts over the same scope, not from the per-mailbox triggers, which
count held mail.

**Delivery.** At a rule's due time, the timer creates a `digest_deliveries`
row and attaches everything held since the last delivery; a delivery with
nothing in it is not created. Each open delivery is one surfaced row (R3). A
due time missed while Focus was closed delivers once, at its next start.

**Actions.** `A` archives the delivery's messages as one undo unit. `D`
removes the sender from the rule in config, and that sender's future mail
goes to the inbox. Removing a rule releases what it held.

**Preview (screen 24):** the rule's queries through the executor over the
last 90 days, counted, with the first four rows.

---

## R14. Store and config: what lives where

What the user decides lives in `config.toml`, which no store change touches.
The store holds what can be recomputed or re-entered, and what is lost when a
store starts over (R0) is short-lived.

**Store:**

- `messages.sort_at`, `messages.unsubscribe_offered` and `messages.automation`;
- `markers`;
- `filter_decisions`;
- `digest_holds` and `digest_deliveries`;
- `reminders`, which are lost when a store starts over, as snoozes are;
- `correspondents`;
- `focus_classified`: what the catch-up has done, by stage and classifier
  version;
- the `settings` keys `focus.move_recent` and `focus.filed_through`;
- `egress_log.subsystem` includes `'model'`.

**Config:**

- `[focus]`: `filtering` and `reading`;
- `[focus.filter]`: `never` and `stop_markers`;
- `[[focus.digests]]`;
- `[focus.model]` and `[focus.vault]`.

The list reads markers with one extra batched statement per page, for Focus
scopes only. The full shapes are in [data-model.md](data-model.md).

---

## R15. Compose in Focus's frame

**Decision.** Focus's composer host implements `ComposerHost` (R1): the
context is Composer while it is open, the dialog (or the window it is
detached to) is the parent of file dialogs, autosave is on, and the window's
resolver serves keys. Its frame is [screens.md](screens.md), "The
composer".

**The draft carries `labels` and `remind_at`.** When it is sent, the host
applies the labels to the Sent copy's conversation and creates the reminder.

**Recipient chips** are a presentation of the shared composer's fields, which
Focus turns on.

**Completion ranking** is `postio_ui::recipients::suggest`, over
`Correspondent { contact, sent_count }` rows that `RecipientDirectory`
carries. The order:

1. sent count (`correspondents.sent_count`, R8);
2. ADR 0007 Q6's band: contacts the user made or imported before those seen
   only in mail;
3. last seen;
4. times seen.

While every count is 0, the order is the store's. Suggestions open at four
characters (C23). A suggestion row does not show the count yet, because the
composer's `RecipientCandidate` carries only the address.

**Send later** is the existing schedule path. There is no Markdown toggle
(C7).

---

## R16. The local model and Obsidian

**`postio-ai`**, the crate ADR 0009 named, is a client for the user's local
runtime:

- **Interface:** the OpenAI-compatible chat completions both Ollama
  (`127.0.0.1:11434/v1`) and llama.cpp's server (`127.0.0.1:8080/v1`) serve,
  with `response_format: {type: "json_schema", …}`. Each question's schema is
  flat and checked again on the client.
- **Transport:** `io-http` over std sockets; loopback needs no TLS. It can
  connect only through a `ModelEndpoint`, which can only be built from a
  loopback address or a local socket, so `localhost` is never looked up.
- **Safety:** message text is fenced, and no tools are offered. After a
  failure the runtime is left alone for 60 s. Every call is recorded in the
  egress log under `model`. The crate has no send path, which the boundary
  check enforces.
- **The needs-action question.** `ModelLayer` answers
  `Result<Option<_>, Unavailable>`, and `Unavailable` hands the question to
  the built-in detector. A quote that is not verbatim in the text is dropped.
- **Summaries.** A summary is statements, each with references, each
  reference a message and an excerpt. A statement stays only if its excerpt is
  found byte for byte in the own text, when it is written and again when it is
  read (`TextIndex::find`). The summary is plain text, stored with the
  delivery, and written after the delivery; the row shows its senders until it
  lands, so nothing waits.
- **"More like this".** Postio builds the candidate queries (the list, the
  sender, the sender's domain), and the model answers with one number, so a
  saved rule is never text the model wrote (FR-132).

**`postio-vault`** writes Obsidian Tasks lines:
`- [ ] <text> [✉](postio://message/<id>) 📅 YYYY-MM-DD`.

- The link goes before the date: the Tasks plugin reads its fields from the
  end of the line, and allows only tags and block ids after them (Tasks
  8.4.0, `DefaultTaskSerializer`).
- 📅 is U+1F4C5. A finished task reads back as `- [x] … ✅ YYYY-MM-DD`.
- A project is suggested from the subject's words; a capture is appended to
  the end of its note.

**`postio://`** is registered in Focus's desktop file as
`x-scheme-handler/postio`. A `postio:` URI arrives through `HANDLES_OPEN` as a
GFile whose `uri()` carries it, as `mailto:` does. It navigates and never
acts, because any page or app can fire one.

---

## R17. Testing

**Fast (`--lib`):**

- `postio-classify`: rules, the detector, and guards over fixtures;
- `postio-calendar`: invitation fixtures;
- `postio-search`: `natural` and `parse_when`;
- `postio-ui`: key-map groups, presets, splice positions, label colours,
  `focus_dialog`, `focus_row`, `focus_state` and `dwell`.

**Counting** (`postio_storage::test_support::counting`):

- a Focus scope page is a bounded number of statements, plus one for
  markers, with no scans;
- the filing pass's statements per new message are bounded;
- the Focus counts are counted.

**Differential:** the digest matcher against the executor, over the corpus.

**Registry:**

- enumeration across frontends (SC-015);
- Focus parity: every Focus command has a key, a command-bar row and a
  visible control, and every command Focus is offered reaches a handler
  (`registry_parity`).

**Integration:** `crates/postio-gtk/tests/focus_suite/` is one binary on the
custom harness, with `CASES`, `IGNORED` and the list contract, on the headless
compositor. Each user story's acceptance scenarios are cases that assert on
the widget tree, and keys and clicks are delivered through GTK's own
controllers, never by calling a handler.

**Screens:** `shot` renders each screen from the demo store, in light and
dark; [screens.md](screens.md) records each comparison.

**Nightly:** SC-011's first pass, at 100,000 messages, and the excerpt
locator's floors, under `POSTIO-MEASUREMENT:` markers in
`.config/nextest.toml`.

---

## R18. What the spikes settled

| Spike | Settled |
|---|---|
| S1 | calcard reads the corpus invitations, its graph passes the dependency policy (R9) |
| S2 | Not run: the promoted headers' bytes per new message on a real account (R8) |
| S3 | A two-height list with spliced rows holds at 100,000 conversations, binding at most 205 rows a jump (R3) |
| S4 | Rules alone clear the detector's precision bar; no table of weights (R10) |
| S5 | Highlighting by excerpt finds 99.5% of sentences read from what is drawn (R2) |
| S6 | `sort_at` leaves the list's counting tests unchanged (R7) |
