# A condvar in a runtime, and a future nobody awaits

2026-09-12, `feature/turso-store`.

Two findings from porting the write gate, both about code that compiled,
passed its own tests, and did nothing.

## 1. `WriteGate` blocked the runtime

`WriteGate` (#425) is Postio's answer to the fact that the database has one
writer at a time: a queue with a priority in it, so a keystroke's write
overtakes a backfill's batch rather than racing it. It was a
`std::sync::Condvar`, which blocks the **thread**.

Every thread in Postio is now a tokio worker. Three writers parked on a
condvar waiting for a permit the sync pass will release *when its own task next
runs* is a deadlock with no error message: there is nobody left to run the task
that releases it. `concurrent_writers::a_sync_batch_survives_the_ui_thread_
writing_underneath_it` hit the 240-second timeout and said only that.

`acquire` is `async` now and waiters park on a `tokio::sync::Notify`. Two
details that are not optional:

- The state stays under a **blocking** `Mutex`. Nothing awaits while it is
  held, every critical section is a few integer operations, and an async mutex
  would cost a task wake-up per acquisition to protect nothing.
- The `Notified` future is created and `enable()`d **before** the state is
  read. Otherwise a permit released between the read and the await is missed,
  and the waiter sleeps until the next unrelated release.

## 2. Three ways to drop a future, and the compiler sees one

`unused_must_use = "deny"` caught 64 dropped futures during the port. It cannot
see either of the other two shapes:

```rust
let _ = repository.delete(id);        // used — bound to `_`
let permit = gate.acquire(priority);  // used — bound to a name, then dropped
```

Both compile silently. Both do nothing.

`clippy::let_underscore_future` catches the first, and is `deny` at the
workspace root now. It found five in `send.rs`, and every one of them was a
user-visible defect: after sending a message the draft was never deleted, its
state never became `Sent`, and the sent copy was stored with **no body and no
thread**. `send::sending_a_draft_delivers_it_and_files_a_sent_copy` is what
noticed, by asserting the draft was gone.

Nothing catches the second. `resync.rs` had `let permit = ...acquire(...)` with
no `.await`, so every resync batch ran with no permit at all, and
`a_resync_batch_does_not_lock_out_an_interactive_write` failed as `database is
locked` — the exact symptom the gate exists to prevent, in the exact test
written for it.

**So: the test is the only detector for that shape.** A guard that binds a
future to a name and relies on `Drop` is a guard that cannot be checked by the
compiler, and the assertion that it works has to exist somewhere.

## 3. And the busy timeout was zero

Turso's `busy_timeout` defaults to **0** where SQLite's configuration here set
5,000 ms: a writer that finds the lock taken gets `Busy` immediately, with no
retry. `WriteGate`'s own documentation is written against the timeout — the
gate orders Postio's *own* writers and the timeout covers everything it does
not.

It was not the only per-connection setting missing. Measured against a fresh
store:

```text
foreign_keys         0  -> 1        cascades silently did not fire
temp_store           2              already MEMORY; asserted anyway
busy_timeout         0  -> 5000     no retry at all
cache_size       -2000  -> -65536   2 MB against the 64 MB this store expects
synchronous          2  -> 1        FULL fsyncs on every flag change
journal_mode       wal              already; setting it is a query, not an execute
```

`store::PER_CONNECTION` carries all of them with the measurement beside each,
because an engine's documented default and its actual one have already differed
twice here.
