# ADR 0005 — Multiple accounts and the unified inbox

- **Status:** Accepted — **GO** (2026-08-24)
- **Date:** 2026-08-24
- **Issue:** [#1 Multiple accounts & unified inbox](https://github.com/dlapiduz/postio/issues/1)
- **Unblocks:** [#64](https://github.com/dlapiduz/postio/issues/64) (add-account
  flow), and is the reason [#2](https://github.com/dlapiduz/postio/issues/2)
  (OAuth) has somewhere to put a second provider
- **Decision:** **an engine per account, one database, and a unified inbox that
  is a scope rather than a mailbox.** A thread never spans accounts; the
  unified list groups threads at read time. `AppState.account: Option<AccountId>`
  becomes `AppState.scope: Scope`.

---

## What is already built

More than the issue implies, which is why this is wiring rather than a
redesign. Measured at `0e0ec08`.

| Piece | State |
|---|---|
| `accounts` and `identities` tables | Built, no cardinality assumption |
| `account_id` on `mailboxes`, `threads`, `messages`, `sync_state`, `operation_queue`, `contacts` | Built, indexed |
| `contacts.account_id NULL` meaning "shared across accounts" | Built, with the partial unique indexes to match |
| `[accounts.<id>]` as a map, with `default = true` | Built (`config/src/accounts.rs`) |
| `AppState.connections: BTreeMap<AccountId, ConnectionState>` | Built — status is already per account |
| `Engine` keyed to one account, owning one connection on one thread | Built (`runtime/src/engine.rs`) |
| SQLite in WAL with a connection pool and a 5s busy timeout | Built (`storage/src/db.rs:73`) |
| **`first_account()`** | The cut. `app/src/lib.rs:389` and `:449` |

The single-account assumption lives in exactly two call sites in the
composition root and in `AppState.account`. Everything below it is already
account-aware.

---

## Q1 — One database or one per account?

**One database.** It is what is built, and the alternative is worse in the
place that matters.

A database per account would make the unified inbox a cross-connection join,
which SQLite can do with `ATTACH` but which then makes every list query's plan
depend on how many accounts are configured. It would also fork the FTS5 index,
so a search across accounts becomes N searches merged in Rust with no way to
rank them against each other — and search is Postio's primary way to move
around (`spec.md` §7).

The cost of one database is writer contention, and Q3 answers it.

---

## Q2 — Does a thread span accounts?

**No.** `threads.account_id` stays `NOT NULL`, and
`idx_threads_account_last_at` stays the list's index.

The temptation is real: the user is cc'd on one conversation at two of their
addresses, and it is one conversation to them. But a thread in Postio is not
only a display grouping — it is **sync state**. JWZ threading runs over an
account's message set; `threads.last_at` drives a per-account index; removing
an account has to remove its threads. A thread whose `account_id` were nullable
would need re-keying on every account add and split on every account removal,
and it would lose the index that makes the message list windowed rather than
loaded (`ARCHITECTURE.md` §4).

**Instead, the unified list groups threads at read time.** A `ThreadGroup` is
one or more threads from different accounts whose JWZ root carries the same
`RfcMessageId`, or — where a root is missing, which is common — the same
normalised subject within the coalescing window `postio-model::subject` already
defines. Grouping is computed by the same paged query that builds the list, over
`idx_messages_rfc_message_id`, which already exists.

Two consequences worth stating rather than discovering:

- **A message the user received at two addresses appears once.** Deduplication
  is by `RfcMessageId` within a group. The copies are distinct rows in distinct
  accounts and both stay; the *list* shows one.
- **An action on a group hits every copy.** Archiving a unified-inbox row
  archives it in both accounts, as two operations in two per-account queues.
  This is the only answer that matches what the user believes they did. It is
  also why `Selection` staying a predicate (`ARCHITECTURE.md` §4) matters here:
  the expansion from group to messages happens in the store, not in a `Vec` the
  frontend built.

---

## Q3 — Concurrency: how do N engines share one database?

**One engine per account, each on its own thread with its own connection,
exactly as `runtime/src/engine.rs` already builds one.** Nothing about that
design assumed there was only one.

The pieces that make N of them safe are already in place:

- **WAL** (`db.rs:73`) — readers never block the writer, so the UI's list
  queries are unaffected by a sync pass on another account.
- **`busy_timeout = 5000`** (`db.rs:79`) — SQLite allows one writer at a time;
  a second engine's write retries rather than failing. Sync writes are short
  batched transactions, so five seconds is many orders of magnitude of headroom.
- **Per-account operation queues** — `idx_operation_queue_drain` is
  `(account_id, state, next_attempt_at, id)`. Two drainers on two accounts never
  see each other's rows.

**The one thing that must be added: a bound on concurrent engines.** Each
engine holds an IMAP connection, a TLS session and a connection from the pool.
`Database::open_with` takes `max_connections`; the composition root must size
the pool from the account count rather than from a constant, and must refuse to
start more engines than the pool can serve — otherwise the tenth account
deadlocks waiting for a connection that a sync pass is holding.

**What must *not* be added: a global sync lock.** Serialising accounts would
make one slow or unreachable server stall every other account's mail, which is
the failure mode multi-account exists to avoid.

**The test that proves the criterion.** Issue #1's first acceptance criterion —
"two accounts sync concurrently without interference" — is a `postio-runtime`
test over two `MailBackend` mocks with different latencies, asserting that the
fast account's messages land before the slow account's pass finishes, and that
each account's `sync_state` row reflects only its own pass. No network, per the
repository rule.

---

## Q4 — What the frontend selects: `Scope`, not an optional account

```rust
pub enum Scope {
    /// One account. Its mailboxes are real folders; `a` archives into one.
    Account(AccountId),
    /// Every enabled account at once. A view, never a destination.
    Unified,
}
```

`AppState.account: Option<AccountId>` becomes `AppState.scope: Scope`, and
`Option` stops carrying two meanings at once ("no account configured" and "no
account chosen").

**The unified inbox is a scope, not a mailbox, and the distinction is the one
`ARCHITECTURE.md` §6 already draws.** A real mailbox has a `UIDVALIDITY`, a
message set that physically lives there and a `MailboxRole`; mail *moves into*
it. Unified is a view over the `Inbox`-role mailbox of every enabled account.
Concretely:

- **Move has no meaning in Unified.** The move command is unavailable in that
  scope — not silently a no-op, *unavailable*, so it is absent from the palette
  and the cheat sheet, which is what `registry::reachable` for a context is for.
- **Archive does.** `a` archives each copy into *its own account's* Archive
  mailbox. There is no ambiguity: every message knows its account.
- **Compose from Unified uses the default identity** of the account marked
  `default = true` in `[accounts]`, and replying uses the identity of the
  account the message arrived at. A reply that went out from the wrong address
  is a bug the user notices after it is sent, so the identity picker shows the
  resolved identity rather than assuming it (`composer.rs` already has
  `identity_row` and `identity_only` for exactly this).

**Per-account visual identification.** One accent hue per account, drawn as the
3px left border the PLATE design already gives the selected row, plus the
account's short name on the row in Unified scope only. Hues come from a fixed
ordered palette in `tokens.rs`, assigned by account position — generated, never
typed (`ARCHITECTURE.md` §10). Colour is never the only signal: the short name
carries it for anyone who cannot distinguish the hues.

**Sidebar shape.** Unified at the top as its own root, then one collapsible
section per account. `Context::Sidebar` already exists and the folder commands
are already reachable from it, so account switching is `registry` work rather
than new interaction.

---

## Q5 — Search across accounts

`postio-index` executes a parsed `postio-search` query against FTS5. Scope
becomes a *filter on the executor*, not a change to the query language: the
same query string means the same thing in either scope, which is
`ARCHITECTURE.md` §6's rule.

Two small additions:

- The executor takes `Scope`, and `Scope::Account` adds the `account_id`
  predicate it already has an index for.
- The query language gains `account:` as a `Field`, so a saved search can pin
  itself to one account regardless of the scope it is run from. This is one row
  in `postio-search::Field` and one arm in the parser, and it keeps
  `[filters]` expressive enough to survive multi-account without a second
  syntax.

---

## Q6 — Adding, disabling and removing an account

- **`accounts.enabled`** already exists. A disabled account keeps its mail,
  stops its engine, and drops out of `Scope::Unified`. This is the reversible
  operation, and it is what the settings panel offers by default.
- **Removal** is `ON DELETE CASCADE` across mailboxes, messages, threads,
  sync state and the operation queue — and is therefore genuinely destructive
  and genuinely local. It must be `destructive: true` in the registry, and
  because `Recovery::None` on a destructive command is rejected at
  registration (`ARCHITECTURE.md` §2), it needs a real recovery: the account is
  soft-deleted and its rows are reaped on next start, so the toast's *Undo*
  has something to undo.
- **The keyring entry outlives the account row on purpose.** Removing an
  account removes Postio's copy of the mail, not the user's credential;
  deleting the keyring entry is a separate, confirmed step. A remove that
  silently destroyed a keyring item shared with another tool would be
  unrecoverable in a way nothing else here is.

---

## Alternatives

**A database per account.** Rejected in Q1: forks the FTS5 index and makes
cross-account search unrankable.

**Threads that span accounts.** Rejected in Q2: it makes thread identity depend
on account membership, costs the list its index, and requires splitting threads
on account removal — to buy something read-time grouping already gives.

**Unified as a synthetic mailbox row.** Attractive because the sidebar and the
list would need no new concept, and wrong for the reason `ARCHITECTURE.md` §6
gives: `move` and `UIDVALIDITY` would then have to mean something for a folder
mail cannot live in.

**One sync engine multiplexing all accounts.** Fewer threads, one connection
pool, and one slow server stalling everyone. The engine is already a thread
that owns a connection because `rusqlite::Connection` is `!Sync`; N of them is
the cheap path, not the expensive one.

---

## Consequences

- `first_account()` disappears. `app/src/lib.rs` starts an engine per enabled
  account and sizes the pool from the count.
- `AppState.scope` is a breaking change inside `postio-core`, caught by the
  compiler everywhere it matters.
- `postio-search` gains one `Field`; `docs/keybindings.md` and the config
  reference regenerate themselves (`ARCHITECTURE.md` §2).
- The `Move` command becomes context-conditional on scope, which is the first
  time a command's availability depends on state rather than on `Context`.
  That is a registry question, not a frontend one — resolve it as a predicate
  the registry can evaluate, or the palette will offer a command that cannot run.
