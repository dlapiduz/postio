# Phase 1 Data Model: The Outbox, and reserved mailboxes every account has

**Feature**: `specs/003-outbox-and-reserved-mailboxes` | **Date**: 2026-09-11

What changes in the store and in the model types. Nothing here restates
`spec.md`; it says how each entity is represented and what must remain true of
it.

---

## The one new stored fact

### `messages.send_state` — a draft's send state, denormalised onto its row

```sql
ALTER TABLE messages ADD COLUMN send_state TEXT
  CHECK (send_state IS NULL
         OR send_state IN ('editing','queued','sending','failed','unconfirmed'));

CREATE INDEX idx_messages_send_state
    ON messages (account_id, send_state)
 WHERE send_state IS NOT NULL;
```

`NULL` for every ordinary message — which is almost every row, hence the
partial index. Non-`NULL` exactly on the mirror row `list_row` writes for a
draft, carrying the same value as `drafts.state`.

**Why it is duplicated rather than joined.** Drafts and the Outbox are two
predicates over one mailbox (research R2), and one of them is
`ListScope::Mailbox` — the query every folder in the application reads through.
A correlated subquery there is a cost on the hot path; a column is an indexed
predicate. This is the same trade the schema already makes for `messages.draft`,
`seen`, `flagged` and `answered`, which are denormalised off the `FlagSet`
(`messages.rs:974-981`) for exactly this reason.

**Who writes it.** `DraftRepository` only, in the same transaction that writes
`drafts.state` — `save`, `set_state`, `queue_send`, `queue_send_at`,
`cancel_send`. Never the sync layer directly, and never a trigger: two writers
with different invariants is how the value drifts.

**Invariant, and it is a test.** For every row where `drafts.message_id` is not
null, `messages.send_state = drafts.state`. A repository-level check asserts it
after each verb; the store's consistency test asserts it across a seeded
mailbox.

**`Sent` never appears.** A draft that has been sent has no `drafts` row and no
mirror row — `DraftRepository::delete` removes both (`drafts.rs:609`). The
column's `CHECK` therefore omits `sent`: a value it can never legitimately hold
should not be spellable.

### The count triggers split

`mailboxes.total_count` is trigger-maintained over `messages`
(`0001_initial_schema.sql:657-720`) and currently counts the mirror rows. FR-022
needs Drafts to show only what is not in flight, so the triggers gain the same
`send_state` condition the Drafts scope uses. The Outbox has no mailbox row and
therefore no cached column; its count is computed (below).

---

## Model types that change

### `MailboxRole` — gains `Outbox`, and gains a kind

```rust
pub enum MailboxRole {
    Inbox, Archive, Sent, Drafts, Trash, Junk,   // reserved: a real folder
    Flagged, Snoozed, Outbox,                    // views: never a folder
    Regular,                                     // an ordinary folder
}

pub enum RoleKind { Folder, View }

impl MailboxRole {
    pub fn kind(self) -> RoleKind;
    /// The six every account must have a folder for (FR-026).
    pub const RESERVED: [MailboxRole; 6];
}
```

`Outbox` joins `Flagged` and `Snoozed` as a `View`. `Regular` is a `Folder`.

**The rule `kind()` exists to make checkable** (FR-037, FR-040, FR-041): a view
names no server folder, can never be stored, and is never a destination. Today
that is answered by `id.get() > 0` or an empty `path`, in each caller that
remembers to ask; `Snoozed` is unstorable only because
`0001_initial_schema.sql:119`'s `CHECK` happens not to list it.

**Three places enforce it**, so no single one is load-bearing alone:

1. `MailboxRepository::create`/`update` refuse a `View` role with a typed error.
2. The SQL `CHECK` lists exactly the `Folder` spellings.
3. `scripts/checks/check-view-roles-are-not-storable.py` fails when those two
   disagree — which is what stops the next role from being added to one and not
   the other.

**`role_order` gains the Outbox** (`postio-ui/src/sidebar.rs:23`), between
Drafts and Sent (FR-014): Inbox, Flagged, Snoozed, Drafts, **Outbox**, Sent,
Archive, Junk, Trash. Its position is fixed so that appearing and disappearing
does not move the rows around it.

### `Mailbox` — carries the kind it resolved to

No new stored column: `kind` is derived from `role` and is present so a consumer
can ask a row what it is rather than inferring it from a negative id or an empty
path. The view rows are constructed with `id` unset rather than negative, which
is what removes `FLAGGED_ROW = -1` and `SNOOZED_ROW = -2` from `postio-gtk::feed`.

### `ListScope` — gains `Outbox(AccountId)`

```rust
pub enum ListScope {
    Mailbox(MailboxId), Account(AccountId), Unified,
    Flagged(AccountId), Snoozed(AccountId),
    Outbox(AccountId),          // new
    Thread(ThreadId),
}
```

Per-account, matching `Flagged` and `Snoozed` (spec Assumptions). Arms required
in `reaction`, `is_drawn_from`, `mailbox()` and `where_clause` — the compiler
finds all four, which is why the enum is the right place for this.

`mailbox()` **must** answer `None` (FR-010). Its doc already states the reason:
the answer is load-bearing wherever a caller goes on to use it as somewhere a
message could be put.

### `MessageListRow` — `draft: bool` becomes a state

```rust
pub struct MessageListRow {
    // ...
    pub send_state: Option<DraftState>,   // replaces `draft: bool`
}
```

FR-015 and FR-021 need a row to say *which* of editing, queued, sending, failed
or unconfirmed it is; `draft: bool` cannot (#1491's third open question). The
`bool` is recoverable as `send_state.is_some()` for the callers that only want
the draft mark.

### `Event` — gains `DraftStateChanged`

```rust
DraftStateChanged { account: AccountId, draft: DraftId, state: DraftState },
```

Emitted wherever the send path sets state. Outbox and Drafts scopes react
`Reload`; everything else ignores it. Without it the Outbox updates only on the
next whole-sidebar refresh, which is "the UI awaits the network" wearing
different clothes: the local write has happened and the screen does not know.

---

## Reads this feature adds

### The sidebar's two counts

One query per account, run where the sidebar's data is assembled, returning
both numbers:

- **Outbox count** — rows whose `send_state` is `queued` or `sending`.
- **Drafts attention count** — rows whose `send_state` is `failed` or
  `unconfirmed` (FR-022).

The Drafts *total* comes from the same place and stops reading
`mailboxes.total_count`, because that column now excludes in-flight rows and the
badge needs the count the user sees.

**Bounded, and gated as counts** (Principle V, SC-008): statements and rows
fixed and independent of mailbox size, asserted with
`postio_storage::test_support::counting` before and after the partial index
exists.

### The two scope predicates

| Scope | Predicate (in addition to the existing snooze and `deleted_locally` clauses) |
|---|---|
| `Outbox(account)` | `messages.account_id = ?1 AND messages.send_state IN ('queued','sending')` |
| `Mailbox(drafts)` | the existing `mailbox_id = ?1`, **and** `send_state IS NULL OR send_state NOT IN ('queued','sending')` |

The second is the risk: it lands on the generic mailbox scope, so every folder
pays for it. It is one comparison against a column that is `NULL` for
essentially every row, and the task that adds it carries `counting` assertions
either side.

---

## The reserved-role record

`mailbox_roles` (migration `0017`, merged with #1496) already holds each
account's role→path map. FR-031 adds, per account and role, the fact that the
server refused to create the folder and what it said:

- when the refusal happened, and
- the server's own words, so the user is told rather than guessed at.

Cleared when the map changes or the folder appears. **Its whole purpose is that
a refusal is not retried on every discovery pass** — a server that will never
allow a folder must not be asked forever.

---

## What does not change

- **How a draft is stored.** `drafts` keeps its columns and `drafts.message_id`
  keeps being the link. #166 already put the mirror row where this feature needs
  it.
- **The send state machine.** ADR 0021's states, the operation queue, the
  drainer's ordering and the `Message-ID` reserved at queue time all hold. What
  changes is where those states are *shown*.
- **`Flagged` and `Snoozed` semantics.** They move crates; they do not change
  meaning.
- **No backwards compatibility.** `send_state` takes a plain `NULL` default and
  existing drafts are corrected on their next save or by a one-pass backfill in
  the migration; there is no shim.
