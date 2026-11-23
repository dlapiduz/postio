# ADR 0040 — The store keeps few connections, maintains its counts, and budgets its index

- **Status:** Proposed (2026-09-23). The connection rule (#1602) is built.
- **Decision:** A few long-lived connections with a cache each. Every per-page aggregate comes from a maintained column or a fully bound index range, gated on its plan. A materialised `(mailbox, thread)` table makes a folder window an index range. The engine's full-text retained budget is patched down. The page codec stays. The allocator is chosen by measurement.

## Rules
- **Connections.** `Store::read` keeps about three warm readers behind a semaphore (a turso `Connection` serialises its operations). Each sync lane holds one checkout for the wave, plus one for settle and one for the body indexer. Interactive connections use `cache_size -65536`; `Store::connect_background` uses `-4096`. No pool, no per-read connect. Connections are counted by `test_support::counting::checkouts()`. `WriteGate` is unchanged. Interactive reads use a reader, never `interactive_write()`.
- **Why caches matter less than count:** turso clears a connection's whole page cache after any other connection's commit (`pager.rs:3198`) and each connection has its own cache. Win comes between syncs.
- **Aggregates.** Cheap only as a column read or a range with every leading index column bound. `counting::scans` sees only `SCAN`, so gate with an `EXPLAIN QUERY PLAN` assertion naming the bound columns plus a two-size counted fixture (the `startup_reads` shape). `scans().is_empty()` alone is not a gate.
  - Folder thread count: `mailboxes.thread_count`, by trigger.
  - Flagged, Account and Unified counts: `MailboxRepository::account_counts`.
  - `corpus_complete`: `mailboxes.bodies_owed` by insert and `AFTER UPDATE OF body_state` triggers. No `(account_id, body_state)` index.
  - Unified count: the witness cache with `sum(total_count)`.
  - `draft_counts`: `idx_messages_send_state` covers `(account_id, send_state, deleted_locally)`.
- **Deep offset.** `mailbox_threads (mailbox_id, thread_id, last_at, rep_message_id, unread, flagged, has_attachments, member_count)`, maintained by triggers on `messages`, indexed `(mailbox_id, last_at DESC, rep_message_id DESC)`. The window is an index-only `LIMIT/OFFSET`. Land only after `examples/insert_cost_curve.rs` shows the fts column remains the ceiling. `Marks` stays as an optimisation.
- **Full-text memory.** Patch `FTS_MAX_RETAINED_CACHE_BYTES` to 32 MiB and request a builder option upstream. Strip quoted lines and cap folded text per document in `index_body`. Know the transients: 64 MiB segment arena per build, merges at 100 KiB, 1 MiB, 100 MiB, 1 GiB.
- **Encryption.** Keep the page codec (ADR 0014). Moving off the engine or to fscrypt is not this ADR's to reopen.
- **Allocator.** Measure the same seeded first sync under the current mimalloc v3, `features = ["no_thp"]` and `System`, recording `VmRSS`, `RssAnon`, `AnonHugePages`, `VmHWM` after open, sync and 60 s idle, and wall time. Ship `no_thp` if it recovers at least 50 MB at no more than 5% sync time. Keep mimalloc. Choose by build feature, not a runtime env var.
- **Measure first.** A `connections` counter asserted per list page and verb. Plan gates for `count_of`, `corpus_complete`, `draft_counts`, `wake_due` and the `mailbox_threads` window. The nightly is #1503's RSS plateau as a `POSTIO-MEASUREMENT` module, relative assertions only, with a control that blows the band.
- Schema changes carry no migration (ADR 0038).

## Why
- Measured: 908-917 MB resident; 786 ms to count a 60,907-message folder, repaid after every archive; scrollbar drag as `OFFSET` over a correlated `NOT EXISTS`; per-page `count(*)`.
- Rejected: connection pool, 64 MiB everywhere, sampled cursors (seeding walks the same window), tantivy outside the engine (needs an encrypting `Directory`, a second commit domain, and `fts_match` leaving SQL), `MIMALLOC_ALLOW_THP=0` as an env var.

## Revisit when
- Upstream honours `_shared_page_cache` or invalidates per frame.
- Upstream exposes the FTS retained budget.
- The metadata index writes move off the write transaction (#1587).
