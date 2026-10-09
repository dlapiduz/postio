# Data model: Postio Focus

The shapes Focus adds to the store, to `config.toml` and to the types between
them. The reasons are in [research.md](research.md). A schema change comes
with a migration where one can be written, and a store no migration reaches
starts over (R0). So everything here is either in the store, where it can be
recomputed or re-entered, or in `config.toml`, where the user's own decisions
live and no store change can touch them (R14).

## Persisted: the store

All of these are in `crates/postio-storage/src/schema.rs`'s `HEAD`.

### `messages` (Focus's columns)

| Column | Type | Meaning |
|---|---|---|
| `sort_at` | `INTEGER NOT NULL` | The message's place in a list. It equals `received_at` when the message is inserted, and becomes the wake time when a snooze wakes (R7). The folder and conversation lists order, seek and index by it. `write_update` keeps it at least `received_at`, so a re-saved draft still rises |
| `unsubscribe_offered` | `INTEGER` | `NULL`: not yet known. `0`/`1`: whether the message carries `List-Unsubscribe`. Filled at filing on incremental syncs, or from the body's headers when the body arrives (R8). `is:bulk` answers from it together with `automation` |
| `automation` | `INTEGER` | `NULL`: not yet known. Otherwise a bitmask: 1 `Precedence: bulk`, 2 `Precedence: list`, 4 `Precedence: junk`, 8 `Auto-Submitted: auto-generated`, 16 `Auto-Submitted: auto-replied`. Filled like `unsubscribe_offered`. `is:automated` answers from bits 8 and 16 |

`idx_messages_list` and `idx_messages_thread_mailbox` order by `sort_at DESC,
id DESC`.

### `markers`

At most one per message; the row draws one. Recomputable from the message,
except a dismissal.

| Column | Type | Meaning |
|---|---|---|
| `message_id` | `INTEGER PRIMARY KEY REFERENCES messages ON DELETE CASCADE` | The message the marker is about |
| `kind` | `TEXT CHECK (kind IN ('invite','question','todo','no_reply'))` | No `no_reply` marker is written: a fired reminder is a surfaced row that carries its own `since` |
| `source` | `TEXT CHECK (source IN ('calendar','detector','model','reminder'))` | What made it (FR-104 to FR-108, SC-013) |
| `span_start`, `span_end` | `INTEGER` | Character offsets into the message's own text; `NULL` for invitations |
| `excerpt` | `TEXT` | The sentence, verbatim, at most 200 characters; `NULL` for invitations |
| `starts_at`, `ends_at` | `INTEGER` | An invitation's event, in UTC milliseconds |
| `due_at` | `INTEGER` | A to-do's due date |
| `invite_uid`, `invite_sequence`, `invite_stamp` | `TEXT`, `INTEGER`, `INTEGER` | The iTIP identity used to replace or cancel a marker (R9) |
| `invite_state` | `TEXT CHECK (invite_state IN ('open','cancelled','past'))` | |
| `answer` | `TEXT CHECK (answer IN ('accepting','accepted','declining','declined'))` | `accepting`/`declining` last while the RSVP's window is open |
| `answer_until` | `INTEGER` | When an answer's window closes; Focus's due timer then makes the answer final |
| `dismissed_at` | `INTEGER` | Set when the user dismisses the marker. It never returns on this message (FR-108) |

Indexes: `idx_markers_invite (invite_uid)`, which finds an invitation's
markers for an update or a cancellation; `idx_markers_dismissed
(dismissed_at, message_id)`, which the has-action scope seeks; and
`idx_markers_answer_until`. The engine's planner does not read partial
indexes, so none of them is one.

The repository's `insert` never overwrites, `replace` keeps a dismissal, and
`dismiss(None)` undoes one. A page draws the conversation's newest marker
that is not dismissed.

### `filter_decisions`

| Column | Type | Meaning |
|---|---|---|
| `message_id` | `INTEGER PRIMARY KEY REFERENCES messages ON DELETE CASCADE` | |
| `reason` | `TEXT CHECK (reason IN ('spam','promotion','notification','receipt','shipping','social'))` | The fixed vocabulary (FR-113) |
| `source` | `TEXT` | Shown after the reason: "notification · Forge". A sender or list name; `NULL` if none |
| `layer` | `TEXT CHECK (layer IN ('header','senders','server','model'))` | Which layer decided (FR-113) |
| `decided_at` | `INTEGER NOT NULL` | Local midnight bounds "filtered today" |
| `restored_at` | `INTEGER` | When the user restored the message (`R`, FR-116). A restored decision is kept, so SC-012 can count restores, and undoing a restore puts back the reason it had |

Indexes: `(decided_at)`, `(reason, restored_at, decided_at DESC)` and
`(restored_at, decided_at DESC)`.

### `digest_deliveries` and `digest_holds`

| `digest_deliveries` | Type | Meaning |
|---|---|---|
| `id` | `INTEGER PRIMARY KEY` | |
| `rule` | `TEXT NOT NULL` | The rule's name in `[[focus.digests]]` |
| `due_at`, `delivered_at` | `INTEGER NOT NULL` | When it came due, and when Focus delivered it (later, if Focus was closed) |
| `archived_at` | `INTEGER` | Set by `A`; the row leaves the inbox |
| `summary`, `summary_written_at` | `TEXT`, `INTEGER` | The statements and their references, as JSON; `NULL` until written (FR-172 to FR-175) |

| `digest_holds` | Type | Meaning |
|---|---|---|
| `message_id` | `INTEGER PRIMARY KEY REFERENCES messages ON DELETE CASCADE` | |
| `rule` | `TEXT NOT NULL` | |
| `held_at` | `INTEGER NOT NULL` | |
| `delivery_id` | `INTEGER REFERENCES digest_deliveries ON DELETE SET NULL` | `NULL` while waiting for its delivery |

Indexes: `idx_digest_deliveries_open (archived_at)`, `(delivery_id)` and
`(rule, delivery_id)`. Focus's inbox scope leaves out a message with a hold
whose delivery is `NULL` or not archived. A released message's hold is
deleted (R13).

### `reminders`

| Column | Type | Meaning |
|---|---|---|
| `id` | `INTEGER PRIMARY KEY` | |
| `thread_id` | `INTEGER NOT NULL` | The conversation. Not a foreign key: a threading pass may renumber conversations |
| `anchor_message_id` | `INTEGER NOT NULL REFERENCES messages ON DELETE CASCADE` | The user's message the reminder waits on a reply to |
| `set_at`, `due_at` | `INTEGER NOT NULL` | |
| `fired_at` | `INTEGER` | Set by the due timer that found no reply by `due_at` |
| `cancelled_at` | `INTEGER` | Set when a message from someone else arrives in the thread before it fires (FR-044) |
| `settled_at` | `INTEGER` | Set when a surfaced reminder stops standing |

Indexes: `idx_reminders_standing (settled_at, cancelled_at, fired_at)` and
`(thread_id)`. Like snoozes, reminders are lost when a store starts over:
they are the user's intent, but a short-lived one (R14).

### `correspondents`

| Column | Type | Meaning |
|---|---|---|
| `address_id` | `INTEGER PRIMARY KEY REFERENCES addresses` | |
| `sent_count` | `INTEGER NOT NULL` | Messages the user sent with this address in To, Cc or Bcc, each counted once |
| `last_sent_at` | `INTEGER` | When the latest of them was sent |

It is maintained at local send and when Sent syncs, and never counts the
user's own addresses. It answers the "written to" guard with one lookup
(FR-111), and gives completion its "wrote N times" (FR-052).

- A send that fails takes its one back.
- A Sent folder's sync counts only the rows its upsert inserted, so a local
  copy the sync adopts by Message-ID is not counted twice.
- A Sent folder enumerated again after a UIDVALIDITY reset counts its
  messages again, as the contacts' `times_seen` does.

### `focus_classified`

| Column | Type | Meaning |
|---|---|---|
| `message_id` | `INTEGER NOT NULL REFERENCES messages ON DELETE CASCADE` | |
| `stage` | `TEXT CHECK (stage IN ('filing','body'))` | |
| `version` | `INTEGER NOT NULL` | The classifier's version. A newer version makes the catch-up run again |

The primary key is `(message_id, stage)`. The catch-up at Focus's start
reads the rows with no record at the current version, newest first
(FR-141).

### Other tables

- `settings`: `focus.move_recent`, the last few Move destinations as JSON
  (R6), and `focus.filed_through`, the newest message id the filing catch-up
  has sorted (R12).
- `drafts`: `label_ids`, the labels chosen for a draft, a list column on the
  draft's row so loading a draft stays at three statements; `remind_at`, the
  composer's remind-if-no-reply; and `calendar_reply`, an RSVP's calendar
  part. The send job applies the labels to the Sent copy and the rest of the
  conversation, and creates the reminder.
- `egress_log.subsystem` includes `'model'` (FR-168).

## Persisted: `config.toml`

The full contract is [contracts/config.md](contracts/config.md). In short:

```toml
[focus]
filtering = true                     # FR-119
reading   = "dialog"                 # or "pane" (FR-038)

[focus.filter]
never = ["ada@example.org"]          # pinned senders, and senders restored from Filtered (FR-111, FR-116)
stop_markers = [                     # what repeated dismissals stopped (FR-108)
  { sender = "news@localfirst.example", kind = "question" },
]

[[focus.digests]]
name    = "Newsletters"
match   = ["from:news@localfirst.example", "from:editor@ledger.example"]
cadence = "weekly"                   # daily | weekly | monthly
day     = "saturday"                 # weekly: a weekday; monthly: 1–28
at      = "16:00"

[focus.model]                        # off unless present (FR-166)
endpoint = "http://127.0.0.1:11434/v1"
model    = "qwen3:4b"
needs_action   = true
digest_summary = true
like_this      = true

[focus.vault]
path = "~/Notes"
```

A digest rule's `match` is a list of queries in the one query language, and
the rule holds a message when any of them matches. It is a list because the
language has no `OR` (ADR 0008), and a rule for several senders still has to
mean what search means.

## Runtime (not persisted)

### Scopes and selections

`ListScope::Focus(FocusScope)` (`crates/postio-model/src/scope.rs`) pages
through `list_page` and `list_count` like any other list:

| `FocusScope` | Rows | Membership |
|---|---|---|
| `Inbox` | Conversations | Every enabled account's inbox, minus held messages and minus conversations with a surfaced reminder (R7, R13) |
| `HasAction` | Conversations | `Inbox`'s rows that draw a marker (FR-017), read from the markers |
| `Snoozed` | Messages | Everything snoozed, in every enabled account (`g z`) |
| `Flagged` | Messages | Everything flagged, in every enabled account (`g *`) |

Filtered, a delivery's messages and what a rule holds are client requests,
not list scopes (contracts/engine.md, "Reads").

A select-all in Focus's inbox resolves to `ViewScope::Focus { accounts }`
and then `MessageSet::InFocusInbox`: Focus's membership, so held digest mail
is never in it, minus every copy of each deselected conversation.

### `ThreadSummary`'s Focus fields

- `marker: Option<MarkerSummary>`, filled only for Focus's scopes by one
  batched statement per page, so every other app's reads are unchanged.
- `copies: Vec<ThreadId>`: the other accounts' threads a folded row stands
  for, filled for Focus and the unified inbox. Acting on the row aims at
  `MessageTarget::Threads(id + copies)`, so every copy moves.

### `MarkerSummary` (what a row draws, no body needed)

`{ kind, when: Option<MarkerWhen>, excerpt: Option<String>, answer:
Option<InviteAnswer>, cancelled }`. `when` is an event's start and end, a due
date, or the day a reminder was set. `cancelled` is true for a cancelled
invitation, which offers no answer. There is no action field: `postio-model`
cannot name a command, so a frontend derives the row's action from the kind
and the answer.

### Surfaced rows

`Surfaced::Digest { delivery, rule, cadence, count, senders, summary_line,
at, position }` and `Surfaced::Reminder { reminder, thread, since,
representative, at, position }`. `position` is the number of Focus-inbox
conversations newer than `at`: one bounded count, cached against the list's
witness (R3).

### `FocusRow` (the list model's item)

`Conversation`, `Reminder { reminder, row }` or `Digest`. A conversation row
carries its labels, read for a page in one extra request
(`Req::ThreadLabels`). The row's height follows from the item:

- one line: a conversation with no marker, or a digest;
- two lines: a conversation with a marker, or a reminder.

### Classification output (`crates/postio-classify`)

```text
Outcome  { filter: Option<Reason>, hold: Option<RuleName>, marker: Option<MarkerCandidate> }
Reason   { kind: ReasonKind, source: Option<SourceName>, layer: Layer }
MarkerCandidate { kind: MarkerKind, span: Option<Span>, starts_at, ends_at, due_at,
                  invite: Option<InviteIdentity { uid: InviteUid, sequence, stamp }> }
```

This is the fixed schema (FR-132). There is no free-text field anywhere in it:

- **The quote is only a span.** `Span` is a character range into the
  message's own text. The writer of the marker cuts `markers.excerpt` from
  the own text by that span, so the stored quote is verbatim by construction.
- **The strings are newtypes.** `SourceName`, `RuleName` and `InviteUid` can
  only be built inside `postio-classify`, so nothing else can put text into an
  outcome, a model's answer included.

The classifier's input at filing is `postio_model::filing::FiledMessage`: the
message, its thread, its mailbox role, the promoted header facts, and whether
it has a calendar part. At the body stage it is `BodyMessage { filed,
identities }`: the same message and the user's identities on its account,
which say who "you" is. The automated-senders table reaches the classifier
through `Rules::senders()`.

### `Invitation` (`crates/postio-calendar`)

`{ uid, sequence, stamp: Option<DateTime<Utc>>, method: Request|Cancel|Other,
cancelled, summary, starts_at: EventTime, ends_at: EventTime, zone: Zone,
location, organizer, attendees: Vec<Attendee{address, partstat}>, recurring:
bool, recurrence_id: Option<EventTime>, series_ends_at }`

- **`EventTime`** is `At(instant)`, `Floating(local time)` or `Day(date)`.
  The adapter keeps floating and all-day times as they are rather than
  guessing a zone; `instant_in(zone)` places them in the user's zone when a
  marker needs an instant.
- **`Zone`** is `Utc`, `Named(IANA name)`, `Floating` or `Unknown(TZID)`.
  Windows names such as `W. Europe Standard Time` resolve to their IANA name.
- **`cancelled`** is true for a `METHOD:CANCEL`, or an event whose `STATUS`
  is `CANCELLED`. `recurrence_id` names the occurrence an update is about.
- **`series_ends_at` and `last_end()`** give a recurring series' last
  occurrence, bounded at 1,024 occurrences, so a series can be past.

`reply(invitation, attendee, answer)` writes a `METHOD:REPLY` with the
invitation's UID, SEQUENCE and organiser, and only the answering attendee,
spelled as the invitation spells them. `supersedes(newer, older)` requires the
same UID and occurrence, then compares SEQUENCE, then DTSTAMP, and returns
false when it cannot tell.

### `FocusCounts` (the header strip)

`{ conversations, unread, has_action, filtered_today }`, each a counted read
over a Focus scope, cached against the list's witness rather than the
per-mailbox trigger counts, which count held mail (R13). "N digest rules" and
the next digest come from `[focus]`, not from the store.

## State transitions

**Marker (question or to-do):** open → dismissed. A new classifier version
may re-evaluate a marker that has not been dismissed.

**Marker (invitation):**

```text
open ──y──▶ accepting ──10 s──▶ accepted
  │                 └──undo──▶ open
  ├──Y──▶ declining ──10 s──▶ declined
  │                 └──undo──▶ open
  ├── REQUEST with higher SEQUENCE ──▶ open (new time)
  ├── CANCEL for the UID ──▶ cancelled (no actions)
  └── event ends ──▶ past (no actions)
```

**Reminder:** set → cancelled (a reply from someone else) | fired (due, no
reply) → settled. Undo of "set" deletes the reminder.

**Digest hold:** held → delivered (the delivery is created at its due time) →
archived (`A`). Held → released when the rule is removed or its sender is
stopped (`D`); a release deletes the hold, and the message rejoins the inbox.

**Filter decision:** decided → restored (`R`): the message moves to the inbox,
the decision gains `restored_at`, and the sender joins `[focus.filter]
never`. Undo reverses the move and clears `restored_at`, and removes the
sender from `never` only if no other restore from that sender still stands.

**RSVP send:** queued with not-before = keypress + 10 s → cancelled (undo
within the window) | sending → sent.
