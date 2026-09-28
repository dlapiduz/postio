# Data model: Postio Focus

Phase 1 for [plan.md](./plan.md). The reasons behind each shape are in
[research.md](./research.md). A schema change moves the store's fingerprint,
and an existing store then resyncs into a fresh one. There are no migrations
(R0). So everything here is either in the store, where it can be recomputed or
re-entered, or in `config.toml`, where the user's own decisions live and a
resync cannot touch them (R14).

## Persisted: the store

All of these are additions to `crates/postio-storage/src/schema.rs`'s `HEAD`.

### `messages` (new columns)

| Column | Type | Meaning |
|---|---|---|
| `sort_at` | `INTEGER NOT NULL` | The message's place in a list. It equals `received_at` when the message is inserted. When a snooze wakes, it becomes the wake time (R7). The list's order, its seek marks and its indexes use it instead of `received_at`. `write_update` must leave it alone, so it stays out of `row_values` (`messages.rs:2201-2218`) |
| `unsubscribe_offered` | `INTEGER` | `NULL`: not yet known. `0`/`1`: whether the message carries `List-Unsubscribe`. It is filled at filing on incremental syncs, or from the body's headers when the body arrives (R8). The `is:bulk` operator answers from it together with `automation` |
| `automation` | `INTEGER` | `NULL`: not yet known. Otherwise a bitmask: 1 `Precedence: bulk`, 2 `Precedence: list`, 4 `Precedence: junk`, 8 `Auto-Submitted: auto-generated`, 16 `Auto-Submitted: auto-replied`. It is filled like `unsubscribe_offered`. The `is:automated` operator answers from bits 8 and 16 |

Indexes: `idx_messages_list`, `idx_messages_thread_mailbox` and their account
and recency variants order by `sort_at DESC, id DESC` where they ordered by
`received_at`.

### `markers`

At most one per message; the row draws one.

| Column | Type | Meaning |
|---|---|---|
| `message_id` | `INTEGER PRIMARY KEY REFERENCES messages ON DELETE CASCADE` | The message the marker is about |
| `kind` | `TEXT CHECK (kind IN ('invite','question','todo','no_reply'))` | `no_reply` is a fired reminder's marker (R7) |
| `source` | `TEXT CHECK (source IN ('calendar','detector','model','reminder'))` | What made it (FR-104 to FR-108, SC-013) |
| `span_start`, `span_end` | `INTEGER` | Character offsets into the extracted own text; `NULL` for invitations and reminders |
| `excerpt` | `TEXT` | The sentence, verbatim, at most 200 characters; `NULL` for invitations |
| `starts_at`, `ends_at` | `INTEGER` | An invitation's event, in UTC milliseconds |
| `due_at` | `INTEGER` | A to-do's due date; for `no_reply`, the date the reminder was set |
| `invite_uid`, `invite_sequence`, `invite_stamp` | `TEXT`, `INTEGER`, `INTEGER` | The iTIP identity used to replace or cancel a marker (R9) |
| `invite_state` | `TEXT CHECK (invite_state IN ('open','cancelled','past'))` | |
| `answer` | `TEXT CHECK (answer IN ('accepting','accepted','declining','declined'))` | `accepting`/`declining` last while the RSVP's window is open |
| `dismissed_at` | `INTEGER` | Set when the user dismisses the marker. It then never returns on this message (FR-108) |

Indexes, as built (T109):

- **`idx_markers_invite`** finds an invitation's markers by UID for an update
  or a cancellation (`MarkerRepository::invitations`).
- **`idx_markers_open` is not built.** It was planned as a partial index,
  `WHERE dismissed_at IS NULL AND answer IS NULL AND …`, and this engine's
  planner does not read partial indexes. The has-action scope (T048) needs
  its own seek, counted.

What the repository does:

- `insert` never overwrites.
- `replace` keeps a dismissal.
- `dismiss(None)` undoes a dismissal.
- A page draws the conversation's newest marker that is not dismissed.

### `filter_decisions`

| Column | Type | Meaning |
|---|---|---|
| `message_id` | `INTEGER PRIMARY KEY REFERENCES messages ON DELETE CASCADE` | |
| `reason` | `TEXT CHECK (reason IN ('spam','promotion','notification','receipt','shipping','social'))` | The fixed vocabulary (FR-113) |
| `source` | `TEXT` | Shown after the reason: "notification · Forge". A sender or list name; `NULL` if none |
| `layer` | `TEXT CHECK (layer IN ('header','senders','server','model'))` | Which layer decided (FR-113) |
| `decided_at` | `INTEGER NOT NULL` | Local midnight bounds "filtered today" |

Indexes: `(decided_at)` and `(reason, decided_at DESC)`.

### `digest_deliveries` and `digest_holds`

| `digest_deliveries` | Type | Meaning |
|---|---|---|
| `id` | `INTEGER PRIMARY KEY` | |
| `rule` | `TEXT NOT NULL` | The rule's name in `[[focus.digests]]` |
| `due_at`, `delivered_at` | `INTEGER NOT NULL` | When it came due, and when Focus delivered it (later, if Focus was closed) |
| `archived_at` | `INTEGER` | Set by `⇧A`; the row leaves the inbox |
| `summary`, `summary_written_at` | `TEXT`, `INTEGER` | Milestone 2: the statements and their references, as JSON; `NULL` until written (FR-172 to FR-175) |

| `digest_holds` | Type | Meaning |
|---|---|---|
| `message_id` | `INTEGER PRIMARY KEY REFERENCES messages ON DELETE CASCADE` | |
| `rule` | `TEXT NOT NULL` | |
| `held_at` | `INTEGER NOT NULL` | |
| `delivery_id` | `INTEGER REFERENCES digest_deliveries ON DELETE SET NULL` | `NULL` while waiting for its delivery |

Indexes: `(delivery_id)` and `(rule, delivery_id)`. Focus's inbox scope leaves
out a message with a hold whose delivery is `NULL` or not archived. A released
message's hold is deleted (R13).

### `reminders`

| Column | Type | Meaning |
|---|---|---|
| `id` | `INTEGER PRIMARY KEY` | |
| `thread_id` | `INTEGER NOT NULL` | The conversation |
| `anchor_message_id` | `INTEGER NOT NULL` | The user's message the reminder waits on a reply to |
| `set_at`, `due_at` | `INTEGER NOT NULL` | |
| `fired_at` | `INTEGER` | The tick that found no reply by `due_at` set it |
| `cancelled_at` | `INTEGER` | Set when a message from someone else arrives in the thread (FR-044) |
| `settled_at` | `INTEGER` | Set when the user replies to, archives or dismisses the surfaced row |

Index: `(fired_at, settled_at)` for the surfaced rows, and `(thread_id)`.

Like snoozes, reminders are lost on a resync. They are the user's intent, but
they are short-lived, and the policy is to resync rather than migrate (R0, R14).

### `correspondents`

| Column | Type | Meaning |
|---|---|---|
| `address_id` | `INTEGER PRIMARY KEY REFERENCES addresses` | |
| `sent_count` | `INTEGER NOT NULL` | Messages the user sent with this address in To, Cc or Bcc |
| `last_sent_at` | `INTEGER` | |

It is maintained at local send (`crates/postio-sync/src/send.rs:525-575`) and
when Sent syncs. It answers the "written to" guard with one lookup (FR-111),
and gives completion its "wrote N times" (FR-052).

As built (T075):

- A send that fails takes its one back.
- A Sent folder's sync counts only the rows its upsert inserted
  (`UpsertReport.inserted_ids`), so a local copy the sync adopts by
  Message-ID is not counted twice.
- A Sent folder enumerated again after a UIDVALIDITY reset does count its
  messages again. The contacts' `times_seen` shares that inflation.

### `focus_classified`

| Column | Type | Meaning |
|---|---|---|
| `message_id` | `INTEGER REFERENCES messages ON DELETE CASCADE` | |
| `stage` | `TEXT CHECK (stage IN ('filing','body'))` | |
| `version` | `INTEGER NOT NULL` | The classifier's version. A newer version makes the catch-up run again |

The primary key is `(message_id, stage)`. It is what the catch-up at Focus's
start reads: rows with no record, newest first (FR-141).

### Changes to existing tables

- `settings`: the key `focus.move_recent` holds the last few Move
  destinations as JSON (R6).
- `drafts.label_ids`: the labels chosen for a draft (`Draft.labels`, T077), a
  list column on the draft's row, so loading a draft stays at three
  statements. The send job applies them to the Sent copy and to the rest of
  the conversation, as `add_label` does.
- `egress_log.subsystem` gains `'model'` in milestone 2 (FR-168).

## Persisted: `config.toml`

The full contract is [contracts/config.md](./contracts/config.md). In short:

```toml
[focus]
filtering = true                     # FR-119

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

[focus.model]                        # milestone 2, off unless present (FR-166)
endpoint = "http://127.0.0.1:11434/v1"
model    = "qwen3:4b"
needs_action   = true
digest_summary = true
like_this      = true

[focus.vault]                        # milestone 3
path = "~/Notes"
```

A digest rule's `match` is a list of queries in the one query language.
The rule holds a message when any of them matches. It is a list because the
language has no `OR` on `main` (ADR 0008, "What is already built"), and a
rule for several senders still has to mean what search means.

## Runtime (not persisted)

### `ListScope` additions (`crates/postio-model/src/scope.rs`)

`ListScope::Focus(FocusScope)`:

| `FocusScope` | Rows | Membership |
|---|---|---|
| `Inbox` | Conversations | The unified inbox's membership, minus held messages and minus conversations that have a surfaced reminder (R7, R13) |
| `HasAction` | Conversations | `Inbox`, and the conversation has an open marker (FR-017) |
| `Filtered { reason: Option<Reason> }` | Messages | Messages with a filter decision, newest first (FR-115) |
| `Delivery(DeliveryId)` | Messages | A digest delivery's messages (FR-125) |
| `Held(rule)` | Messages | What a rule holds now (`g d`, FR-126) |

`ThreadSummary` gains `copies`: the other accounts' threads a folded row
stands for, filled for Focus and Unified (T161). Acting on the row aims at
`MessageTarget::Threads(id + copies)`, so every copy moves (#1701 is the
classic Unified row's gap). It also gains `marker: Option<MarkerSummary>`. It is filled only for
Focus scopes, by one batched statement per page, so the classic app's reads
are unchanged.

### `MarkerSummary` (what a row draws, no body needed)

`{ kind, when: Option<When>, excerpt: Option<String>, answer: Option<Answer>, cancelled }`.
`cancelled` is true for a cancelled invitation, which offers no answer (T109).
`When` is either an event's start and end, or a due date. There is no
`action`: `postio-model` cannot depend on `postio-core`, so a frontend derives
the row's action from the kind and the answer (T033).

### Surfaced rows

`Surfaced::Digest { delivery, rule, cadence, count, senders, summary_line: Option<String>, at }`
and `Surfaced::Reminder { thread, since, representative: MessageSummary, at }`.

Each one's list position is the number of Focus-inbox conversations whose
`sort_at` is newer than its `at`: one bounded count, cached against the list's
witness (R3).

### `FocusRow` (the list model's item)

`Conversation(ThreadSummary)`, `Digest(Surfaced::Digest)` or
`Reminder(Surfaced::Reminder)`. The row's height follows from the item:

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
  message's own text. The candidate carries no excerpt. The writer of the
  marker (T117) cuts `markers.excerpt` from the own text by that span, so the
  stored quote is verbatim by construction.
- **The strings are newtypes.** `SourceName`, `RuleName` and `InviteUid` can
  only be built inside `postio-classify`, so nothing else can put text into an
  outcome, a model's answer included.

The classifier's input is `postio_classify::FiledMessage`: the message, its
mailbox role, the promoted header facts, and whether it has a calendar part.
At the body stage it is `BodyMessage { filed, identities }`: the same message,
and the user's identities on its account, which say who "you" is. The
automated-senders table reaches the classifier through `Rules::senders()`.
Filing and the classifier share one type, `postio_model::filing::FiledMessage`
(T102).

The filing lane added:

- `focus_classified.message_id`, which is `NOT NULL`;
- the settings key `focus.filed_through`, the newest message id sorted so
  far, which the catch-up on open reads (T127);
- `Invitation::series_ends_at` and `last_end()`, bounded at 1,024
  occurrences, so a recurring invitation can be past (T110).

### `Invitation` (`crates/postio-calendar`)

`{ uid, sequence, stamp: Option<DateTime<Utc>>, method: Request|Cancel|Other, cancelled,
   summary, starts_at: EventTime, ends_at: EventTime, zone: Zone, location, organizer,
   attendees: Vec<Attendee{address, partstat}>, recurring: bool,
   recurrence_id: Option<EventTime> }`

- **`EventTime`** is `At(instant)`, `Floating(local time)` or `Day(date)`.
  The adapter keeps floating and all-day times as they are rather than guessing
  a zone. `instant_in(zone)` places them in the user's zone when a marker needs
  an instant (T110).
- **`Zone`** is `Utc`, `Named(IANA name)`, `Floating` or `Unknown(TZID)`.
  Windows names such as `W. Europe Standard Time` resolve to their IANA name.
- **`cancelled`** is true for a `METHOD:CANCEL`, or an event whose `STATUS` is
  `CANCELLED`. `recurrence_id` names the occurrence an update is about.
- **The adapter exposes the first occurrence only.** For a recurring series,
  T110's "past" state needs the rule's last occurrence, which the adapter does
  not expose yet. T110 adds it.

`reply(invitation, attendee, answer)` writes a `METHOD:REPLY` with the
invitation's UID, SEQUENCE and organiser, and only the answering attendee,
spelled as the invitation spells them. `supersedes(newer, older)` requires the
same UID and occurrence, then compares SEQUENCE, then DTSTAMP. When it cannot
tell, it returns false.

### `FocusCounts` (the header strip)

`{ conversations, unread, has_action, filtered_today, digest_rules, next_digest: Option<(RuleName, DateTime)> }`.

Each figure is a counted read over a Focus scope, cached against the list's
witness, not the per-mailbox trigger counts (R13).

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
reply) → settled (reply, archive or dismiss). Undo of "set" deletes the
reminder.

**Digest hold:** held → delivered (the delivery is created at its due time) →
archived (`⇧A`). Held → released when the rule is removed, or its sender is
stopped (`D`). A release deletes the hold, and the message rejoins the inbox.

**Filter decision:** decided → restored (`R`). Restoring moves the message
to the inbox and adds the sender to `[focus.filter] never`. Undo reverses
both. As built (T125), the decision is not deleted: it gains `restored_at`.
Undo removes the sender from `never` only if no other restore from that
sender still stands.

**What the commands lane added:**

- the columns `drafts.calendar_reply`, which is the RSVP's calendar part, and
  `markers.answer_until`, when the answer's undo window closes (T112);
- the `reminders` index `idx_reminders_standing (settled_at, cancelled_at,
  fired_at)`, because `(fired_at, settled_at)` scanned (T094).

The Focus window lane added (T048, T043):

- `idx_markers_dismissed`, a full index, where the plan named a partial
  `idx_markers_open`;
- `Req::ThreadLabels`, which reads a page's labels in one extra request.
  `FocusRow::Conversation { summary, labels }` is the row;
- `FocusCounts { conversations, unread, has_action, filtered_today }`. It has
  no digest fields: the window derives "next digest" and "N digest rules"
  from `[focus]`.

The classic lane added (T167, T168):

- `ViewScope::Focus { accounts }`. A select-all in Focus resolves to
  `MessageSet::InFocusInbox`: Focus's membership, so held digest mail is
  never in it, minus every copy of each deselected conversation.
- `Marked::copies`. The has-action page draws a folded conversation once,
  and its counts are deduplicated.

No `no_reply` marker is written. A fired reminder is a surfaced row that
carries `since` (T136).

**RSVP send:** queued with not-before = keypress + 10 s → cancelled (undo
within the window) | sending → sent.
