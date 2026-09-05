# A warm `-shm` hides the whole cost of a write-ahead log (2026-09-05, #1175)

#1175 is a 676 MB write-ahead log against an 868 MB database, and five
seconds in front of every first frame. Two things about it were only learnt by
measuring the wrong thing three times.

## The log survives because the app does not close

A clean close checkpoints the log and **deletes** it: measured here, a
`-wal` of 615 MB became a `-wal` of zero bytes the moment the `Database` was
dropped. So a log that is still there at the next launch is a log whose
process never closed — and this one never does, on purpose. `db.rs`'s
`silence_openssl_atexit` comment says why: the exit path calls `exit()`,
because tearing the store down eagerly was its own crash (#794, #699).
Nothing runs, nothing is checkpointed, and the log is waiting next time.

That is what makes `Database::open` the right place to reclaim it. Before the
pool has handed anything out there is no reader to block a `TRUNCATE`
checkpoint, which is the one moment that is true.

## Why it grew: `journal_size_limit` was unset

A checkpoint *resets* the log — SQLite starts writing again from the top of
the same file — and without `journal_size_limit` it does not give the file
back. Measured on a 20,000-message seed: 6.8 MB of log with nothing else
happening, and **44 MB with one reader holding a snapshot across the writes**,
because a checkpoint cannot pass the oldest open reader. Neither shrinks by a
byte afterwards. One bad afternoon is therefore permanent, and a running
Postio always has readers.

## The measurement trap, which cost three runs

**In-process, a 615 MB log opens in a millisecond.** The cost is not reading
the log, it is rebuilding the `-shm` index from it, and that only happens when
no other connection has the database. A test that builds the log and then
opens the same path in the same process inherits a warm index and measures
nothing:

```
MEASURED wal_left=615120152 cold_open=1.169ms   <- wrong, -shm was live
```

Worse, `drop(database)` cleans up behind the measurement: the clean close
deletes the log, so the *second* open in a without-the-fix run looks fixed.

The shape that measures it:

- `std::mem::forget` the handle rather than dropping it — that is what
  `exit()` does, and it is the state the store is actually left in;
- copy the database and its `-wal` to a fresh path **without** the `-shm`,
  and open the copy. That is a new process finding the log.

With that, on a 615 MB log built from 90,000 messages:

| | cold open | log after | next cold open |
|---|---|---|---|
| today | 1.41 s | 615 MB | **1.39 s, every launch** |
| reclaiming at open | 1.75 s | 0 | **4.8 ms** |

One launch pays a third of a second more, and every launch after it stops
paying at all.
