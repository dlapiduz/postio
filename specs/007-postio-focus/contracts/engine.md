# Contract: the engine side of Focus

What the host, the client, the command vocabulary, sync and Focus's engine
crates promise each other. The reasons are in [research.md](../research.md)
(R3, R7–R10, R12–R14, R16), and the shapes in
[data-model.md](../data-model.md).

## Focus mode in the host (`crates/postio-host`)

```rust
impl Host {
    /// Turn on Focus's pipeline in this process. Called by the two Focus
    /// apps, postio-gtk and postio-tui (C29), at startup, after `start` and
    /// before `start_syncing`. macOS does not call it, so nothing below runs
    /// while it holds the store (spec Clarifications: "only while Focus
    /// runs").
    pub fn enable_focus(&self, setup: FocusSetup) -> FocusHandle;
}
```

`enable_focus` installs three things:

1. **A filing pass** in `postio-sync`, for incremental passes only (below).
2. **A body-stage task**, on the `spawn_body_indexer` pattern:
   - it subscribes to `Event::BodyLoaded` and debounces bursts;
   - it runs invitations and the needs-action question over the bodies that
     arrived;
   - at start, it catches up on rows with no `focus_classified` record for the
     current version, newest first, at background priority, on one core
     (FR-141).
3. **A due timer** on the engine's five-second tick, the one that wakes
   snoozes:
   - it creates digest deliveries whose time has come;
   - it fires reminders with no reply;
   - it makes RSVP answers final when their window closes.

`FocusSetup::default().with_config(focus)` carries the `[focus]` settings,
and a later `enable_focus` call replaces them. The pass sits in a
`FilingSlot` every engine shares (`Wiring.filing`), so enabling it later
takes effect from the next pass. `ClientKind::Focus` names Focus's
connection.

## The filing pass (`crates/postio-sync`)

```rust
/// Called inside an incremental pass's own write unit, once per write unit
/// that filed something new, with exactly the new messages.
#[async_trait]
pub trait FilingPass: Send + Sync + Debug {
    async fn file(&self, transaction: &Connection, filed: &[FiledMessage<'_>])
        -> Result<FilingEffects, SyncError>;
}
```

Focus's pass is `postio_sync::FocusFiling` (`crates/postio-sync/src/filing.rs`).

- **Where it is called.** In `resync::incremental`'s write unit. A server
  with no MODSEQ (Gmail, JMAP) never reaches it, because its steady-state
  pass is a full enumeration, so that pass hands the rows it inserted to the
  filing pass through `commit_batch_filing`. First syncs, rebuilds and
  re-enumerations pass nothing (FR-118). The pass is async and takes the
  store's `Connection`, because the store has no `Transaction` type.
- **`FiledMessage`** (`postio_model::filing::FiledMessage { message, thread,
  role }`, re-exported by `postio-sync` and `postio-classify`) carries only
  what is known at filing: the envelope and `References`; `list_id`,
  `unsubscribe_offered` and `automation`; the flags, `$Junk` among them;
  whether a `text/calendar` part is present; the mailbox's role and the
  thread.
- **Its writes go through the storage verbs that take the caller's
  transaction**: filter decisions, holds, reminder cancellations, and the
  archive of filtered mail with its queued server move.
- **Errors never lose mail** (ADR 0008, Q6). The pass runs in its own
  savepoint: a failure rolls back only what it wrote, leaves the message in
  the inbox, logs ids and outcome only, and the insert commits.
- **Its cost is bounded:** at most **4 statements per new message** plus one
  per write, with no scans. A counting test in the sync suite asserts both.

## The classifier (`crates/postio-classify`)

```rust
pub fn at_filing(message: &FiledMessage, facts: &dyn Facts, rules: &dyn Rules) -> Outcome;
pub fn at_body(message: &BodyMessage, text: &OwnText, facts: &dyn Facts, rules: &dyn Rules) -> Outcome;

pub struct Outcome {
    pub filter: Option<Reason>,
    pub hold:   Option<RuleName>,
    pub marker: Option<MarkerCandidate>,
}
```

- **`Facts`** answers the guards, each with at most one seek or lookup. It is
  implemented in `postio-sync`, over the store:
  - `wrote_to(address)` reads `correspondents`;
  - `took_part(thread)` is an `EXISTS` over `idx_messages_thread_mailbox`
    with Sent;
  - `own_domain(address)` reads every account's and identity's addresses in
    one statement over those two small tables, so an address at a shared
    provider guards every sender there, which is the safe direction;
  - `never_filter(address)` reads `[focus.filter] never`.
- **`Rules`** holds the automated-senders table (`Senders::shipped()`, from
  `crates/postio-classify/data/senders.toml`, which `build.rs` validates), the
  digest rules as plain `(name, queries)` through the matcher, the stopped
  marker kinds from `[focus.filter] stop_markers`, and the detector's rules.
- **`BodyMessage`** carries the filed message and the user's `identities` on
  its account: who "you" is, and which names a greeting can use.
- **Layers, in order,** where an earlier layer's decision stands (FR-130):
  1. guards;
  2. corrections;
  3. structure and rules;
  4. the user's model, through a `ModelLayer` trait that `postio-ai`
     implements.
- **The needs-action question is asked once**, after layers 1–3, by
  `at_body`:
  1. **FR-106's gate** (`considered`) decides whether to ask: sent directly to
     the user; no list, bulk or automation signal (automation bits,
     `List-Id`, `List-Unsubscribe`, `$Junk`); not in Junk, Sent, Drafts,
     Outbox or Trash; not from the user, nor from a sender in the table. An
     unknown header fact counts as no evidence either way.
  2. **Text that speaks to a machine** ("previous instructions", "system
     prompt", "AI assistant", `tool_call`, …) gets no marker from either
     detector, at a small cost in recall on ordinary mail using those
     phrases.
  3. **The user's model** answers when one is connected; otherwise the
     built-in detector does. Never both (FR-107).
- **Guards** close only the filter question. A message with no From address,
  or no `thread_id`, is guarded and never filtered, so filing sets
  `thread_id`. Mail in a conversation the user took part in is never held.
- **The output is the fixed schema** in the data model, with no free text
  (FR-132).
- **Boundary rule.** No `postio-smtp`, `io-smtp`, `postio-account`,
  `postio-sync`, `postio-runtime`, `postio-transport`, `io-imap`, toolkit,
  network crate or inference engine. It depends on `postio-model` and
  `postio-search` (the matcher and `parse_when`) and nothing else of
  Postio's: own text comes from the caller, and it cannot take
  `postio-config`, whose file watcher brings `mio`, a network crate.

## Invitations (`crates/postio-calendar`)

```rust
pub fn parse(ics: &[u8]) -> Result<Invitation, CalendarError>;
pub fn reply(invitation: &Invitation, attendee: &EmailAddress, answer: Answer) -> Vec<u8>; // METHOD:REPLY
pub fn supersedes(newer: &Invitation, older: &Invitation) -> bool;                     // SEQUENCE, then DTSTAMP
```

It wraps calcard behind this adapter (R9). It is a pure leaf: no store engine,
no toolkit, no network, no async runtime.

## Sync and sending

**The promoted headers (R8).**

| Backend | How they arrive |
|---|---|
| IMAP | A separate `BODY.PEEK[HEADER.FIELDS (LIST-UNSUBSCRIBE PRECEDENCE AUTO-SUBMITTED)]` item in the same FETCH, on incremental fetches only (`MailBackend::fetch_headers_for_filing`), so still one round trip |
| JMAP | From the body. io-jmap 0.3's `JmapEmailProperty` has no `header:<name>` form, only every header, which is the cost ADR 0025 refused. io-jmap is surveyed again before landing |
| Gmail | The metadata it already fetches |

`postio_model::promoted::PromotedHeaders` is the one reader of the three
headers. An unknown value is `NULL`, and writing an unknown value never
erases a known one. For everything else, the indexer fills the two columns
when the body arrives. Search has `is:bulk` and `is:automated`, which answer
from those columns (constitution III; ADR 0025's promotion path).

**Backfill.** Text parts are fetched with `text/calendar` parts of 256 KiB or
less, stored beside them.

**Outgoing.** `outgoing::build` takes `calendar: Option<CalendarPart { method,
ics }>` and places it last in the `multipart/alternative`.

**Correspondents.** Local send and the Sent sync increment `correspondents`
for every To, Cc and Bcc address.

## Commands (`crates/postio-core`)

The keys are in [keymap.md](keymap.md). Each reversible command is one
`CommandId` with a direction or value, as `add_label` is, so its undo inverse
is the same command and no inverse needs a registry row and key of its own.

| Command | Payload | Recovery | `UndoKind` |
|---|---|---|---|
| `snooze`, `unsnooze` | `target, until: Option<DateTime>` (`None`: the preset default) / `target` | Undo | Snooze / Unsnooze |
| `remind_if_no_reply` | `target, at: Option<DateTime>` (`None`: clear) | Undo | Remind / Unremind |
| `toggle_read` | `target` | Undo | MarkRead / MarkUnread |
| `flag` | `target` | Undo | Flag / Unflag |
| `accept_invite`, `decline_invite` | `message` | **Window** (about 10 s) | Accept / Decline, an entry that expires with the window (R9) |
| `digest_rule` | `target` or `rule` | None (it opens the rule dialog; Create writes the rule to `config.toml`, removable at `g d`) | none |
| `stop_digesting_sender` | `target, stopped: bool` | Undo | StopDigesting / ResumeDigesting |
| `archive_thread` in `Digest` | `delivery` | Undo | Archive (one unit) |
| `restore_filtered` | `target, restored: bool` | Undo | Restore / Refilter |
| `dismiss_marker` | `target, dismissed: bool` | Undo | DismissMarker / UndismissMarker |
| `sweep_inbox` | none | Undo | Sweep (one unit, as a predicate) |
| `view_source`, `open_attachment_or_link` | `message` | None | none |
| `go_to_*`, `saved_search_*`, `toggle_has_action`, `toggle_reading_pane`, `back_to_words`, `picker_*` | none | None | none |

- **Availability.** `Availability.frontend` names the frontend asking, and
  `Requirement::Focus` marks what only Focus offers.
- **Contexts.** `Context` has `Picker`, `Digest`, `Filtered` and `Capture`.
- **Undo and the RSVP window.** The host's undo stack has an entry kind for
  an open RSVP window, removed when the window closes, so `mod+z` after that
  reaches the action beneath (R9).

## Reads (`crates/postio-client`)

`Client` serves `MailStore`, so `ListScope::Focus(_)` pages through
`list_page`, `list_count` and `note_removed` with no new request. Focus's
typed calls, one `Req` each:

| Call | Answers |
|---|---|
| `focus_counts()` | `FocusCounts`: the header strip |
| `surfaced()` | The open digest deliveries and fired reminders, each with its `at` and position |
| `thread_labels(threads)` | A page's labels, in one request |
| `filtered(…)` / `filtered_tabs()` | A page of the Filtered view, newest first, and the count for each reason |
| `held(messages)` | Which of these messages a rule holds, and which rule |
| `digest_waiting(rules)` | What each rule holds now (`g d`) |
| `delivery_messages(delivery)` | A digest delivery's messages |
| `digest_preview(…)` | Screen 24's count and first four rows, through the executor |
| `save_digest_rule(…)` / `delete_digest_rule(name)` | Write a rule to `config.toml`, refused when the matcher cannot answer it; remove one, releasing what it held |
| `digest_summary(delivery)` / `digest_like_this(message)` | The summary, with the model; the candidate rule for "Digest mail like this" |
| `sweep_preview()` | How many inbox messages the sweep would file away |
| `raw_source(message)` | The raw RFC 822 bytes, fetched on demand if not local |
| `move_recent()` / `note_move(folder)` | The move picker's Recent, and remembering a move |
| `label_counts(account)` / `create_label(…)` | The label picker's counts, which count conversations, and "Create label" |
| `vault(subject)` / `capture_task(…)` / `capture_note(…)` | The capture sheet's projects and suggestion, and the writes |

**Events.** `Event::SurfacedChanged` says the surfaced rows changed. The list
learns about held, filtered and restored mail through `MessageListChanged`
and `Paging::plan`.

## Store (`crates/postio-storage`, `crates/postio-runtime`)

**Focus scopes** are built in `crates/postio-runtime/src/store/local.rs`
beside the thread pages. Every place the list's membership test appears
takes the Focus scope's extra predicate (`focus_excludes`), or totals, seek
marks and rows would disagree: the window, the representative's `NOT
EXISTS`, the slice, the counts, the boundaries, the rows for changed messages
and the unified count.

**Markers on a page** come from one batched statement per page, on the
`participants_for` pattern, for Focus scopes only.

**Counts** are counted and cached against the list's witness.

**Counting assertions** (`postio_storage::test_support::counting`):

- a Focus inbox page is at most 3 statements at the storage layer (the
  window, participants and markers), with rows equal to the rows returned
  and no scans. The window merges every inbox in one statement: with several
  inboxes it sorts at most inboxes × page rows; with one it does not sort.
  The runtime adds the inbox witness it caches against. With more than one
  account enabled, folding a conversation that reached two inboxes adds the
  partner statements the unified inbox pays;
- `focus_counts` is at most 5 statements, with no scans;
- a surfaced row's position is 1 statement.

**`sort_at`** is the folder and conversation lists' `ORDER BY`, seek mark and
index key (R7); the query views (Account, Flagged, Snoozed, Outbox, the flat
Unified read, Thread) and search stay on `received_at`. `write_update` keeps
`sort_at` at least `received_at`, so a re-saved draft still rises to the top
of Drafts. The list's counting tests hold for it unchanged.

## Boundary rules (`scripts/checks/check-crate-boundaries.py`)

| Crate | Must not depend on |
|---|---|
| postio-widgets | rusqlite, libsqlite3-sys, turso, turso_core, io-imap; postio-host, postio-session, postio-runtime, postio-storage, postio-sync; postio-gtk, postio-app, postio-gtk |
| postio-gtk | postio-gtk, postio-app; any inference engine |
| postio-gtk | postio-gtk |
| postio-classify | postio-smtp, io-smtp, postio-account, postio-sync, postio-runtime, postio-transport, io-imap; gtk4, libadwaita, webkit6; the network crates; any inference engine |
| postio-calendar | turso, turso_core, rusqlite, libsqlite3-sys, gtk4, libadwaita, tokio, async-std; the network crates |
| postio-ai | postio-smtp, io-smtp, postio-account, postio-sync, postio-runtime, postio-transport, io-imap; postio-storage and the store engine; gtk4, libadwaita, webkit6; every network crate but `io-http`'s; any inference engine |
| postio-vault | the network crates, tokio; gtk4, libadwaita, webkit6; postio-storage and the store engine |

No app binary links a language model or an inference engine (FR-165).
