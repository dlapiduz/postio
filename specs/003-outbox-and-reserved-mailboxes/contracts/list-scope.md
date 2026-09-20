# Contract: `ListScope::Outbox`, and the Drafts split

**Consumers**: `postio-storage` (the query), `postio-session` / `postio-runtime`
(reactions), `postio-gtk::feed` and the FFI (what they may ask of a scope).

## The scope

```rust
ListScope::Outbox(AccountId)
```

One per account, matching `Flagged` and `Snoozed`. There is no unified Outbox:
under the unified scope each account shows its own.

## Membership, and the split it forces

Drafts and the Outbox are **two predicates over one mailbox**. A draft's mirror
row (#166) sits in the account's Drafts folder whatever its state, so the
division is by `messages.send_state`, not by `mailbox_id`.

| Draft state | Listed in | Row says |
|---|---|---|
| `editing` | Drafts | "Draft" |
| `queued` | **Outbox** | "Waiting to send", or its due time when scheduled |
| `sending` | **Outbox** | "Sending…" |
| `failed` | Drafts, and counted apart | the server's own reason |
| `unconfirmed` | Drafts, and counted apart | "Not confirmed" |
| *sent* | Sent | — the draft row and its mirror are gone |

**FR-004 is the invariant**: a draft is in exactly one of Drafts and the Outbox
at any moment. Never both, never neither.

## Predicates

Both are added to the existing `where_clause`, alongside the snooze and
`deleted_locally` clauses that every scope already carries.

```sql
-- Outbox(account)
messages.account_id = ?1 AND messages.send_state IN ('queued','sending')

-- Mailbox(drafts), in addition to messages.mailbox_id = ?1
(messages.send_state IS NULL OR messages.send_state NOT IN ('queued','sending'))
```

**The second clause lands on the generic mailbox scope**, which is the query
every folder in the application reads through. It is one comparison against a
column that is `NULL` for essentially every row, and it is the single riskiest
change in the feature. The task that adds it asserts
`postio_storage::test_support::counting` statements and rows before and after —
Principle V gates causes, not milliseconds.

## Reactions

`ListScope` has four arms the compiler will demand:

| Function | `Outbox(account)` answers |
|---|---|
| `mailbox()` | **`None`** — FR-010. The Outbox is not a destination, and this is where that becomes checkable rather than remembered |
| `is_drawn_from(&[Mailbox])` | whether that account has any folder, as `Flagged` and `Snoozed` do |
| `reaction(arrival)` | `Reload` for `DraftStateChanged`; otherwise as `Flagged` — a scope over one account's mail reacts to that account's arrivals |

## The event

```rust
Event::DraftStateChanged { account: AccountId, draft: DraftId, state: DraftState }
```

Emitted wherever the send path writes a draft's state — `queue_send`,
`cancel_send`, and each `set_state` in the drainer. Outbox and Drafts scopes
reload; every other scope ignores it.

**Why not reuse `MailboxesChanged`**: it means "the folder list changed", and
making it also mean "a draft moved" reloads the whole sidebar on every send
transition.

## What a caller may not do

- **Move, copy or file a message into the Outbox.** `mailbox()` returning `None`
  is the mechanism; a command that needs a destination must be unavailable here,
  as it already is for `Unified`.
- **Use the scope as a mailbox id.** There is no `MailboxId` for the Outbox and
  no negative sentinel standing in for one.
- **Assume the Outbox is non-empty.** It is hidden when empty (FR-012) and empty
  is its ordinary state.

## Tests

1. Queue a send with the drainer paused: the message is in `Outbox`, and is not
   in `Mailbox(drafts)`. Release it: it is in neither, and is in Sent.
2. FR-004 as a property over all five states: exactly one of the two lists
   contains the row.
3. `Outbox(account).mailbox()` is `None`; every command needing a destination is
   unavailable under it.
4. Offline — no backend at all — a queued send is listed in the Outbox. This is
   the scenario the whole feature exists for and it must not need a network.
5. `counting` budgets on `Mailbox` listing, unchanged in statements and rows by
   the new clause.
6. A paged Outbox: the cursor is a row value over `(received_at, id)` like every
   other scope, so a second page resumes rather than refilters.
