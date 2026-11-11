# Data model: Threads in Postio Focus, Mac and Linux

Types are sketched by field and meaning. Names are the plan's, and the code
may sharpen them. Each section names its crate.

## `postio-model` (pure)

### Threading cue (`threading.rs`)

`ThreadCue` gains:

| Field | Type | Meaning |
|---|---|---|
| `date` | `DateTime<Utc>` | The message's date, for the subject window |
| `is_list` | `bool` | `list_id` present, or bulk by the filing pass's test; disables the subject fallback |

`ThreadIndex` changes:

- `threads_with_subject(subject, since: DateTime<Utc>) -> Option<ThreadId>`:
  the newest thread whose `last_at >= since`, or none.
- `is_split(thread: ThreadId) -> bool`.

`assign` rules, in order:

1. Links (own id, then references nearest first) map to threads.
2. More than one thread, and any of them split: **Join** the first found
   (the nearest link). Otherwise **Merge** into the oldest, as today.
3. No link, subject usable, `!is_list`: **Join**
   `threads_with_subject(subject, date - 7 days)`, never Merge.
4. Otherwise **New**.

`SUBJECT_WINDOW: chrono::TimeDelta = 7 days`.

### Splitting (`threading.rs`)

`descendants(members: &[ThreadMember], root: MessageId) -> Vec<MessageId>`:
the root and every member whose `In-Reply-To` or `References` names the root
or another descendant, transitively, in date order. Pure and O(thread).

## `postio-storage`

### Schema (one migration in `schema.rs`, a new fingerprint under `tests/schemas/`)

```sql
-- A thread the person muted: new replies are archived on arrival (spec 013
-- FR-012). Keyed by a folded Message-ID the thread claims, so it survives
-- merges and the renumbering a rethread does (research R3).
CREATE TABLE thread_mutes (
    account_id     INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    rfc_message_id TEXT    NOT NULL,
    muted_at       INTEGER NOT NULL,
    replies_since  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, rfc_message_id)
);

-- A message the person split into its own thread, with the replies to it
-- (FR-007). Later mail that names it joins the split thread, never merges.
CREATE TABLE thread_splits (
    account_id     INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    rfc_message_id TEXT    NOT NULL,
    split_at       INTEGER NOT NULL,
    PRIMARY KEY (account_id, rfc_message_id)
);

-- The sender's own words, cut to a line: what a child row shows (R5).
-- NULL until the body is local; '' when the message is all quote.
ALTER TABLE messages ADD COLUMN own_line TEXT;

-- The subject fallback's bounded lookup (R2).
CREATE INDEX idx_threads_subject_recent ON threads (account_id, subject, last_at DESC);
```

### Repositories

- `ThreadRepository`:
  - `muted(ids: &[ThreadId]) -> HashMap<ThreadId, u32>` (replies since), one
    statement per page.
  - `mute(thread)` and `unmute(thread)`: they write or delete
    `thread_mutes` for the thread's root claim.
  - `split(message) -> ThreadId`: a new thread, descendants moved, claims
    relinked, both aggregates recomputed, one transaction.
  - `members(thread) -> Vec<ThreadMember>`: one statement.
- `ThreadingRepository`: `LoadedIndex` loads `is_split` for the threads its
  links found, and the subject lookup takes `since`.
- `MessageRepository::set_own_line(id, line)`.

## `postio-client` / `postio-host`

### `ThreadView` (answer to `Req::Thread(ThreadId)`)

| Field | Type | Meaning |
|---|---|---|
| `thread` | `ThreadId` | |
| `subject` | `Option<String>` | |
| `members` | `Vec<ThreadMember>` | Date order, oldest first |
| `draft` | `Option<ThreadDraft>` | An unsent reply, shown last |
| `muted` | `Option<u32>` | Replies since muting, when muted |
| `labels` | `Vec<Label>` | The thread's labels (union) |

### `ThreadMember`

| Field | Type | Meaning |
|---|---|---|
| `id` | `MessageId` | |
| `from` | `Option<EmailAddress>` | |
| `own` | `bool` | Sent by one of the account's identities ("You") |
| `at` | `DateTime<Utc>` | |
| `seen` | `bool` | |
| `has_attachments` | `bool` | |
| `line` | `Line` | `Own(String)`, `Quoted` (all quote), or `Preview(String)` (no body yet, cut at the attribution) |
| `answers` | `Option<u32>` | The member index its `In-Reply-To` names, only when not the previous member |
| `marker` | `bool` | Has an open action marker (for FR-013) |

### `ThreadDraft`

`id: DraftId`, `first_words: String`, `updated_at`.

### `ThreadMatches` (carried into the window from search)

`query: ParsedQuery`, `messages: Vec<MessageId>` (date order), `at: usize`.

### Events

`Event::ThreadChanged { thread, members_added: Vec<MessageId> }` is emitted
on arrival, split, mute and unmute. An open window and an expansion re-read
their `ThreadView` on it. `members_added` drives the new-reply banner.

## `postio-ui` (pure, shared by both frontends)

### Scope (`scope.rs`, new)

```text
ScopeFocus = Row { thread, latest: MessageId }
           | Child { thread, message: MessageId }
           | Window { thread, message: MessageId }
Scope      = Thread(ThreadId) | Message(MessageId) | Unavailable

target(focus, action: CommandId) -> Scope     // D2's table, cell by cell
footer(focus, view: &ThreadView) -> FooterView // "Harbor API draft v3 · 6 messages" / "Message 4 of 6 · Ben Adeyemi"
```

### List splice (`thread_list.rs`, new)

```text
Splice { anchor: u32, children: u32 }         // at most one per list
Slot   = Row(u32) | Child { row: u32, index: u32 }
slot(display: u32) -> Slot ; display(slot) -> u32 ; len(rows: u32) -> u32
```

### Thread row words (`focus_row.rs`)

- `participants(members/summary, me) -> String`: up to three first names,
  newest first, then "you", then "+N". A one-message thread gives the full
  name.
- `latest_line(summary) -> String`: "Ben: Looks good…".
- `new_badge(unread) -> Option<String>`: "2 new".
- `child_tag(answers) -> Option<String>`: "↳ to Ana".
- `open_at(view) -> MessageId`: first unread, else open marker, else latest
  (FR-013).

### Thread bar (`thread_bar.rs`, new)

```text
Chip     = Message { index, initials, label: "Ben · Thu", current, unread, matched }
         | More { count, from, to }
chips(members, current, matches, room: u32) -> Vec<Chip>   // R8
kicker(index, count) -> "Message 4 of 6 · ] newer · [ older"   (keys from the keymap)
older_banner() / new_reply_banner(member) / match_banner(query, at, matches)
reply_to_row(view, index) -> Option<String>  // "Ana Ruiz, Mon 21 Sep (message 1)"
```

### Outline (`thread_outline.rs`, new)

`OutlineView { rows: Vec<OutlineRow>, people: Vec<(String, u32)>, filter:
String, person: Option<String>, cursor: usize }`. `filter_rows` is pure.

### Search children (`search_view.rs`)

```text
ResultChild = Match { message, number, source_tag: "message 5 · file", passage }
            | Gap { from: u32, to: u32, senders: String, day: String }  // "Message 4 doesn't match · Ben, Thu"
children(view: &ThreadView, matches: &[ConversationMatch]) -> Vec<ResultChild>
```

## `postio-body`

- `forward_thread(members: &[(header, body)], subject) -> OutgoingBody`:
  oldest first, a header per member, quotes folded, attachments deduplicated
  by content hash.
- `forward_as_attachments(raw: &[Vec<u8>]) -> Vec<Attachment>` (one
  `message/rfc822` per member).
- `own_line(body) -> String`: `own_text` cut to `PREVIEW_CHARS`.

## `postio-focus` (controller)

- `Expansion { thread, anchor, view: Option<ThreadView>, focused: Option<MessageId> }`,
  with at most one per list surface (List, Results, Filtered).
- `Input::ThreadRead(ThreadView)` and `Input::ThreadChanged { .. }`.
- `Request::ReadThread(ThreadId)`.
- `Intent::Splice { anchor, children }`, `Intent::Unsplice`,
  `Intent::ThreadBar(..)` and `Intent::NewReply(..)`.
- Window walk: `Walk { view: ThreadView, shown: MessageId, matches:
  Option<ThreadMatches>, pending_new: Vec<MessageId> }`.
  - `ReaderVerb::StepThread(i32)` (existing).
  - New: `ReaderVerb::{FirstInThread, LatestInThread, FirstUnread,
    NextMatch, PrevMatch}`.
- A `Walk` never changes `shown` on `ThreadChanged` (FR-018). It appends to
  `pending_new` and emits `NewReply`.
- `Outline { view, filter, person, cursor }` and `ThreadMenu` are surfaces
  in `surfaces.rs`.
- Forward chooser: `ComposerKind::Forward { form: ForwardForm }` and
  `ForwardForm::{This, Thread, Attachments}`.

## `postio-core`

New `CommandId`s per research R10. `ArchiveThread` is removed.
`Context::Outline` and `Context::ForwardChooser` are added, or the existing
picker context is reused if its key scoping already fits (decided in the
registry task).

## `postio-ffi`

Records mirror the `postio-ui` views: `ThreadViewFfi`, `ThreadMemberFfi`,
`ChildRowFfi`, `ThreadChipFfi`, `ThreadBarFfi`, `OutlineViewFfi`,
`ResultChildFfi`, `FooterViewFfi`, `ForwardChooserFfi`. Swift decides none
of their contents.

## State transitions

```text
Thread row:  Collapsed --Right/chevron--> Expanded --Left/chevron/other Right--> Collapsed
             Expanded --anchor leaves list--> (gone; cursor to next row)
Mute:        Unmuted --m--> Muted(0) --reply arrives--> Muted(n+1) --m--> Unmuted
Window walk: Shown(i) --[ ] { } g u--> Shown(j)
             Shown(i) --ThreadChanged--> Shown(i) + banner     (never moves)
             Shown(i) --} or "Read it }"--> Shown(latest), banner cleared
Compose:     Reply(older) --alt+e--> Reply(latest), typed text kept
             Forward(This) --2--> Forward(Thread) --3--> Forward(Attachments)
```
