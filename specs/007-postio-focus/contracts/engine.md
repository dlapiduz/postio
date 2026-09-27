# Contract: the engine side of Focus

What the host, the client, the command vocabulary, sync and the new crates
promise each other. The reasons are in [research.md](../research.md) (R3,
R7–R10, R12–R14), and the shapes in [data-model.md](../data-model.md).

## Focus mode in the host (`crates/postio-host`)

```rust
impl Host {
    /// Turn on Focus's pipeline in this process. Called by postio-focus at
    /// startup, after `start` and before `start_syncing`. Never called by the
    /// classic app or the terminal, so nothing below runs while they hold the
    /// store (spec Clarifications: "only while Focus runs").
    pub fn enable_focus(&self, setup: FocusSetup) -> FocusHandle;
}
```

`enable_focus` installs three things.

1. **A filing pass** in postio-sync, for incremental passes only (below).
2. **A body-stage task**, on the `spawn_body_indexer` pattern
   (`crates/postio-session/src/lib.rs:1108-1160`):
   - it subscribes to `Event::BodyLoaded` and debounces bursts;
   - it runs invitations and the needs-action detector over the bodies
     that arrived;
   - at start, it catches up on rows with no `focus_classified` record for the
     current version, newest first, at background priority, on one core
     (FR-141).
3. **A due timer** on the engine's five-second tick, the one that wakes
   snoozes (`crates/postio-runtime/src/engine.rs:1229`):
   - it creates digest deliveries whose time has come;
   - it fires reminders with no reply;
   - it expires open RSVP windows.

`ClientKind` gains `Focus` (`crates/postio-client/src/protocol.rs:26-35`).

## The filing pass (`crates/postio-sync`)

```rust
/// Called inside an incremental pass's own write transaction, once per
/// write unit that filed something new, with exactly the new messages.
#[async_trait]
pub trait FilingPass: Send + Sync + Debug {
    async fn file(&self, transaction: &Connection, filed: &[FiledMessage<'_>])
        -> Result<FilingEffects, SyncError>;
}
```

As built by T034 (`crates/postio-sync/src/filing.rs`):

- **Where it is called.** The pass runs in `resync::incremental`'s write unit,
  not in `commit_batch`. Every pass that reaches `commit_batch` is a first
  sync, a rebuild or a re-enumeration, so a call there would never see an
  arrival. `resync_mailbox` calls `resync_mailbox_filing` with no pass.
- **The signature.** It is async and takes the store's `Connection`, because
  the store has no `Transaction` type.
- **How Focus supplies it.** `Host::enable_focus(FocusSetup)` puts it in a
  `FilingSlot` shared by every engine (`Wiring.filing`), so enabling it later
  takes effect from the next pass. `FocusSetup::default()` files nothing.

- **It is called only on incremental passes.** First syncs, whose mail never
  produces `Event::NewMail` (`crates/postio-runtime/src/engine.rs:3372-3461`),
  pass nothing (FR-118).
- **`FiledMessage`** carries only what is known at filing:
  - the envelope and `References`;
  - `list_id`, `unsubscribe_offered` and `automation`;
  - the flags, with `$Junk` among them (`crates/postio-model/src/flag.rs:41-44`);
  - the structure: whether a `text/calendar` part is present;
  - the mailbox's role, and the thread.
- **Its writes go through the storage verbs that take the caller's
  transaction** (`crates/postio-storage/src/actions.rs:1-35`): filter
  decisions, holds, reminder cancellations, and the archive of filtered mail
  with its queued server move.
- **Errors never lose mail** (ADR 0008, Q6). A failure leaves the message where
  it was, in the inbox, logs ids and outcome only, and lets the transaction
  commit the insert. As T034 built it, an error still rolls back the write
  unit; `NoFiling` cannot fail. T102 makes a failure commit the insert.
- **Its cost is bounded:** at most **4 statements per new message** plus one
  per write, with no scans. A counting test in the sync suite asserts both.

## The classifier (`crates/postio-classify`, new)

```rust
pub fn at_filing(message: &FiledMessage, facts: &Facts, rules: &Rules) -> Outcome;
pub fn at_body(message: &BodyMessage, text: &OwnText, facts: &Facts, rules: &Rules) -> Outcome;

pub struct Outcome {
    pub filter: Option<Reason>,
    pub hold:   Option<RuleName>,
    pub marker: Option<MarkerCandidate>,
}
```

- **`Facts`** answers the guards, and each answer is at most one seek or
  lookup:
  - `wrote_to(address)` reads `correspondents`;
  - `took_part(thread)` is an `EXISTS` over `idx_messages_thread_mailbox`
    with Sent;
  - `own_domain(address)` checks the identities;
  - `never_filter(address)` reads `[focus.filter] never`.
- **`Rules`** holds four things:
  - the automated-senders table (shipped TOML, which is data);
  - the digest rules, from `[[focus.digests]]` through the matcher;
  - the stopped marker kinds, from `[focus.filter] stop_markers`;
  - the detector's rules.
- **Layers, in order,** where an earlier layer's decision stands (FR-130):
  1. guards;
  2. corrections;
  3. structure and rules;
  4. the user's model, in milestone 2, through a `ModelLayer` trait that
     postio-ai implements.
- **The needs-action question is not a layer** (T116, T119). After layers 1–3,
  `at_body` asks it at most once:
  1. **FR-106's gate** (`considered`) decides whether to ask. The mail must
     be sent directly to the user. It must carry no list, bulk or automation
     signal (automation bits, `List-Id`, `List-Unsubscribe`, `$Junk`). It
     must not be in Junk, Sent, Drafts, Outbox or Trash, not from the user,
     and not from a sender in the table. An unknown header fact (`None`)
     counts as no evidence either way.
  2. **Text that speaks to a machine** gets no marker from either detector
     ("previous instructions", "system prompt", "AI assistant",
     `tool_call`, …). This costs recall only on ordinary mail that happens to
     use those phrases.
  3. **The user's model** answers when one is connected; otherwise the
     built-in detector does. Never both (FR-107).
- **As built:**
  - `at_filing` and `at_body` take `&dyn Facts` and `&dyn Rules`.
  - `Rules::senders()` returns the automated-senders table:
    `Senders::shipped()` in production, loaded from
    `crates/postio-classify/data/senders.toml`, which `build.rs` validates.
  - `BodyMessage` carries the filed message and the user's `identities` on
    its account. They say who "you" is, and which names a greeting can use.
- **Guards** close only the filter question. A message with no From address,
  or no `thread_id`, is guarded and never filtered, so filing must set
  `thread_id` (T102, T122). Holding a conversation the user took part in is
  T133's rule.
- **The output is the fixed schema** in the data model, with no free text
  (FR-132).
- **Boundary rule.** No postio-smtp, io-smtp, postio-account, postio-sync,
  postio-runtime, postio-transport, io-imap or io-http, and no network crate:
  the list is postio-render's. It may depend on postio-model, postio-storage
  (for `Facts`' reads), postio-search (the matcher and `parse_when`),
  postio-body (own-text extraction), postio-calendar and postio-config.

## Invitations (`crates/postio-calendar`, new)

```rust
pub fn parse(ics: &[u8]) -> Result<Invitation, CalendarError>;
pub fn reply(invitation: &Invitation, attendee: &EmailAddress, answer: Answer) -> Vec<u8>; // METHOD:REPLY
pub fn supersedes(newer: &Invitation, older: &Invitation) -> bool;                     // SEQUENCE, then DTSTAMP
```

It wraps calcard behind this adapter (R9). It is a pure leaf: no store engine,
no toolkit, no network, no tokio.

## Sync and sending

**The promoted headers (R8).**

| Backend | Change |
|---|---|
| IMAP | The existing `HEADER.FIELDS` item gains `LIST-UNSUBSCRIBE PRECEDENCE AUTO-SUBMITTED` on incremental fetches only (`crates/postio-account/src/imap/fetch.rs:213-248`) |
| JMAP | The header request asks for the same three fields |
| Gmail | The metadata headers include the same three fields |

For everything else, `index_headers` fills the two columns when the body
arrives (`crates/postio-index/src/index.rs:565`). Search gains `is:bulk` and
`is:automated`, which answer from those columns (constitution III; ADR 0025's
promotion path).

**Backfill.** `fetch_text_parts` (`crates/postio-sync/src/backfill.rs:1509`)
also fetches `text/calendar` parts of 256 KiB or less, and stores them beside
the text parts.

**Outgoing.** `outgoing::build` (`crates/postio-model/src/outgoing.rs:88`)
accepts `calendar: Option<CalendarPart { method, ics }>` and places it in the
`multipart/alternative`.

**Correspondents.** Local send (`crates/postio-sync/src/send.rs:525-575`) and
the Sent sync increment `correspondents` for every To, Cc and Bcc address.

## Commands (`crates/postio-core`)

The keys are in [keymap.md](./keymap.md). Each reversible command is one
`CommandId` with a direction or value, as `add_label` already is
(`crates/postio-core/src/command.rs:534-547`). Its undo inverse is therefore the
same command, and no inverse needs a registry row and key of its own.

| Command | Payload | Recovery | `UndoKind` |
|---|---|---|---|
| `snooze` | `target, until: Option<DateTime>` (`None`: the preset default) | Undo | Snooze (existing) |
| `remind_if_no_reply` | `target, at: Option<DateTime>` (`None`: clear) | Undo | Remind (new) |
| `toggle_read` | `target` | Undo | MarkRead / MarkUnread (existing) |
| `accept_invite`, `decline_invite` | `message` | **Window** (about 10 s) | none: the window entry expires with the send (R9) |
| `digest_rule` | `target` or `rule` | None (it opens the rule dialog; its Create is the dialog's default action, which writes the rule to `config.toml`, removable at `g d`) | none |
| `stop_digesting_sender` | `target, stopped: bool` | Undo | DigestSender (new) |
| `archive_thread` in `Digest` | `delivery` | Undo | Archive (one unit) |
| `restore_filtered` | `target, restored: bool` | Undo | Restore (new) |
| `dismiss_marker` | `target, dismissed: bool` | Undo | DismissMarker (new) |
| `sweep_inbox` | none | Undo | Archive (one unit, as a predicate) |
| `view_source`, `open_attachment_or_link` | `message` | None | none |
| `go_to_*`, `saved_search_*`, `toggle_has_action`, `back_to_words`, `picker_*` | none | None | none |

- **Requirements.** `Availability.terminal: bool` becomes `Availability.frontend`,
  and `Requirement::Focus` is added (`crates/postio-core/src/registry.rs:85-117`).
- **Contexts.** `Context` gains `Picker`, `Digest`, `Filtered` and, in
  milestone 3, `Capture` (`crates/postio-core/src/context.rs:24-95`).
- **Undo and the RSVP window.** The host's undo stack gains an entry kind for
  an open RSVP window. It is removed when the window closes, so `mod+z` after
  that reaches the action beneath (R9).

## Reads (`crates/postio-client`)

`Client` already serves `MailStore` (`crates/postio-client/src/api.rs:975-1043`),
so the new `ListScope::Focus(_)` scopes page through `list_page`, `list_count`
and `note_removed` with no new request. It gains the following typed calls, one
`Req` each:

| Call | Answers |
|---|---|
| `focus_counts()` | `FocusCounts` (the header strip, screen 16's next digest) |
| `surfaced()` | The open digest deliveries and fired reminders, each with its `at` and position |
| `marker(message)` | The marker card in the dialog |
| `filter_reason(message)` | The Filtered row and dialog |
| `filtered_tabs()` | The count for each reason |
| `digest_preview(match, since)` | Screen 24's count and first four rows, through the executor |
| `digest_rules()` | Each rule with its next delivery and what it holds now (`g d`) |
| `raw_source(message)` | The raw RFC 822 bytes, fetched on demand if not local |
| `move_recent()` | The move picker's Recent |

**Events.** `Event::SurfacedChanged` joins the event set. The list learns about
held, filtered and restored mail through the existing `MessageListChanged` and
`Paging::plan` (`crates/postio-ui/src/paging.rs:23-28`).

## Store (`crates/postio-storage`, `crates/postio-runtime`)

**Focus scopes.** They are built in `crates/postio-runtime/src/store/local.rs`
beside the thread pages. Every place the membership test appears takes the
Focus scope's extra predicate, or totals, seek marks and rows would disagree:

- `crates/postio-storage/src/repository/threads.rs:291`, `:297-307`,
  `:1220` and `:1443`;
- the unified count.

**Markers on a page** come from one batched statement per page, on the
`participants_for` pattern (`threads.rs:1561`), for Focus scopes only.

**Counts** are counted and cached against the list's witness
(`local.rs:77-110`).

**Counting assertions** (`crates/postio-storage/src/test_support/counting.rs`):

- a Focus inbox page is at most 3 statements at the storage layer (the window,
  participants and markers), with rows equal to the rows returned and no
  scans. The window merges every inbox in one statement. With several inboxes
  it sorts at most inboxes × page rows; with one inbox it does not sort. The
  runtime adds the inbox witness it caches against, so a page there is one
  more. With more than one account enabled, folding a conversation that
  reached two inboxes adds the partner statements Unified already pays
  (T161);
- `focus_counts` is at most 5 statements, with no scans;
- a surfaced row's position is 1 statement.

**`sort_at`** replaces `received_at` in the list's `ORDER BY`, seek marks and
indexes (R7). The existing list counting tests
(`crates/postio-storage/tests/storage_suite/list_statement_count.rs:191`,
`threads.rs:340`) must pass unchanged. Spike S6 showed that they do.

- **What moves:** the folder and conversation lists.
- **What stays on `received_at`:** the query views (Account, Flagged, Snoozed,
  Outbox, the flat Unified read, Thread) and search.
- **Drafts keep rising:** `write_update` keeps `sort_at` at least
  `received_at`, so a re-saved draft still rises to the top of Drafts.

## Boundary rules (`scripts/checks/check-crate-boundaries.py`)

| Crate | Must not depend on |
|---|---|
| postio-widgets | rusqlite, libsqlite3-sys, turso, turso_core, io-imap; postio-host, postio-session, postio-runtime, postio-storage, postio-sync; postio-gtk, postio-app, postio-focus |
| postio-focus | postio-gtk, postio-app |
| postio-gtk (added) | postio-focus |
| postio-classify | postio-smtp, io-smtp, postio-account, postio-sync, postio-runtime, postio-transport, io-imap, io-http; the network crates postio-render's rule lists; gtk4, libadwaita |
| postio-calendar | turso, turso_core, rusqlite, gtk4, libadwaita, tokio; the network crates |
| postio-ai (milestone 2) | postio-smtp, io-smtp, postio-account, postio-sync, postio-transport; gtk4 |
