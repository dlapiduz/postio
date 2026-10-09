# A slow query whose SQL is fast is measuring the machine (#500)

A search the readout timed at **3.8 s** replayed at **15 ms** — the same
three statements, the same term, on a copy of the same store. Nothing was
wrong with the plan; everything was wrong around it. The chain, longest
lever first:

1. **The body catch-up was an infinite loop.** `index_body` deliberately
   wrote no row for a textless body, so an attachment-only message (a DMARC
   report, an image) never left `messages_missing_body_text`'s candidate
   set. The store had 654 of them — more than one 200-message batch — so
   `index_local_bodies` re-selected the same batch for ever: a core at 100%
   for as long as the app ran, a stream of ungated autocommit writes, and a
   full-table candidate probe per pass evicting the page cache the search
   needed. Found not by any test but by `top -H` on the live process and
   `gdb -p <tid>` on the hot thread, which is the first thing to reach for
   when a *read* is slow while the SQL is provably fast.
2. **The replay lied because `cp` warms the cache.** Copying the store to
   probe it pages the whole file into the OS cache, so the replay measured
   warm reads while the app was reading cold, on a machine at full swap
   from parallel builds. A later replay under real load reproduced seconds.
3. **Benches on tmpfs cannot see any of this.** `test_support::memory()`
   lives on `/dev/shm` and plain `tempdir()` lands on `/tmp`, tmpfs on the
   reference platform — WAL exists but disk I/O does not, so no write
   pressure there can ever slow a read. `search_under_load.rs` builds its
   corpus under `CARGO_TARGET_TMPDIR` (inside `target/`, a real filesystem)
   for exactly this reason; anything measuring I/O contention must do the
   same.

The structural fixes, so the shape cannot come back: a textless body writes
an **empty index row** — "tried, nothing there" and "never tried" are now
different states; the catch-up **refuses a batch identical to the last one**,
so no future regression can spin it; batches commit **once, behind a
Background write-gate permit**, with the blob reads phased before the
transaction and a breather after it. On the read side the box runs **one
search in flight at a time** (`Live::settled` is the release valve a failed
run must call) and the debounce is sized to typing cadence, so a slow store
is never asked five questions for one word.
