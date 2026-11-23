# ADR 0005 — Multiple accounts and the unified inbox

- **Status:** Accepted (2026-08-24); latest amendments 2026-09-14 (Turso store). The desktop app has no sidebar and shows every enabled account's inbox at once ([ADR 0043](0043-the-inbox-is-every-account-at-once.md)).
- **Decision:** one sync engine per account, one database, and a unified inbox that is a scope, not a mailbox. A thread never spans accounts. A cross-account move is a saga that never deletes before the copy is confirmed. Every aggregate view says whose data is missing.

## Rules
- **Storage.** One database. `threads.account_id` stays `NOT NULL`. The unified list groups threads at read time (`ThreadGroup`); a message received at two addresses shows once, but counts, actions and exports still see both copies. An action on a group hits every copy.
- **Engines.** One sequential engine per account on its own connection. No global sync lock. Concurrent passes are bounded by `MAX_CONCURRENT_PASSES`; `WriteGate` orders writes with `Interactive` ahead of `Background`. Backoff and budgets are per account.
- **Scope.** `AppState.scope: Scope`, never an optional account. Search takes two composing fields, role `facets::Scope` and `AccountScope`; `account:` resolves in SQL. `Move` is unavailable in Unified (a registry predicate). `a` archives each copy into its own account's Archive. Compose from Unified uses the default identity; a reply uses the identity of the account the message arrived at. Filters are queries and account-agnostic; pin one with `account:`. Rules run per account and resolve mailbox names within the arriving account (ADR 0008).
- **Unified search** needs `idx_messages_recency ON messages (received_at DESC, id DESC)`. Per-account `UNION ALL` is slower (32.4 ms against 18.8 ms) and forces a loop.
- **Accounts.** Disable stops the engine and keeps the mail. Removal cascades, is `destructive`, and has a real Undo (soft-delete, reaped on next start). The keyring entry outlives the account row; deleting it is a separate confirmed step. A saga naming a removed account aborts, source intact.
- **Config.** `config.toml` does not describe accounts. An `[accounts.*]` table lands in `Config::extra`, round-trips verbatim, and the footer says it is ignored. Account edits are the account row's three actions: enable/disable, update credential, remove.
- **Cross-account move.** Three phases: COPY (APPEND to B, record `UidMapping`), CONFIRM (APPENDUID, or a `Message-ID` search without UIDPLUS), REMOVE from A. The `cross_account_moves` row holds the phase on disk before either side is touched, and A's drainer refuses REMOVE until it says CONFIRMED. Phase 1 is idempotent by `Message-ID`. If confirmation fails the move stops and asks. Undo is the inverse saga. A move within one account stays a single `MOVE`.
- **Failure.** `ConnectionState::Failing` carries a reason: `Auth` (never retried on a timer), `Network`, `Server`, `Config`. An aggregate view that cannot refresh an account names it in a persistent non-modal line ("Offline - reading local mail"), marks counts and search results partial, and scopes `Selection::Everything` to what is loaded. A disabled account is not a failing one.
- **Events.** Every `Event` variant that names data names its account.
- **Onboarding.** The screen never asks the account type; the probe answers it (password, app password, OAuth, manual). A duplicate address is refused at the form. Nothing keys off "the first account". A failed probe never blocks manual entry. Pimalaya crates do the protocol work; autoconfig comes from `io-pim-discovery`, not hand-written. `io-proxy` and a direct `io-http` are not adopted.
- **Undo** spans accounts and reverses what it can, naming what it could not. Notifications name the account when there is more than one.

## Why
- A database per account forks the full-text indexes and makes cross-account search unrankable. Cross-account threads would tie thread identity to account membership.
- A synthetic "Unified" mailbox gives `move` and `UIDVALIDITY` a meaning for a folder mail cannot live in. One multiplexing engine lets one slow server stall all.
- Delete-then-append loses mail on any failure between steps; append-then-delete risks a visible duplicate. A copy-only move is not what the user asked for.
- A view that omits an account silently turns an empty search into a lie.

## Revisit when
- Host, port or security need editing after onboarding: do it in the account row, never in `config.toml`.
