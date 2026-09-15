# `Pool::get()` is a blocking condvar wait

*Archived 2026-09-14: describes `postio_storage::db::Pool`, which no longer exists; the store is async since ADR 0038, concurrency is `MAX_CONCURRENT_PASSES` plus the `WriteGate` (waiters on `tokio::sync::Notify`), and the deadlock this warned about is the one [a condvar in a runtime](../2026-09-12-a-condvar-in-a-runtime-and-a-future-nobody-awaits.md) records being removed.*

`postio_storage::db::Pool::get()` blocks the calling OS thread on a
`std::sync::Condvar` when the pool is exhausted — it is not async-aware. The
sync engine (`postio_runtime::engine::run`) deliberately runs on a
single-thread tokio runtime with no other OS thread to make progress while
blocked. Work that checks out more than one connection concurrently from
tasks running on that thread must acquire every connection it needs
*sequentially* before starting concurrent work, and must never call
`pool.get()` from inside concurrent work once it has started — otherwise two
tasks can both block on the same condvar with nothing left on that thread able
to run and release one: a genuine self-deadlock, not ordinary contention.
`DEFAULT_MAX_CONNECTIONS` is 4, shared with UI-thread reads, so headroom is
thin. `engine::sync_wave` is the one place that does this and is written to
that rule (#32): it pops its mailboxes and takes all of its connections in a
plain `for` loop, and only then builds the `FuturesUnordered`.
