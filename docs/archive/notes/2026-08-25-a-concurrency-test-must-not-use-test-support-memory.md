# A concurrency test must not use `test_support::memory()`

*Archived 2026-09-14: the premise is gone — `memory()` has been file-backed on `/dev/shm` since #204, and the engine is Turso since ADR 0038, so there is no shared-cache locking model for a test to fall into.*

(#79.) An
in-memory database is opened with SQLite's shared cache, a different locking
model from the WAL one Postio runs on: locks are per-table and a reader blocks
a writer outright rather than the two proceeding side by side. Combined with
the current-thread runtime above, one lane waiting on such a lock blocks every
other lane, and `sync_wave.rs` — whose whole subject is that passes overlap —
went from green to timing out purely because of the store underneath it, not
because anything about the engine had changed. It uses `test_support::temp()`
for that reason. #79's own testing note reached this from the other direction:
in-memory fails with `SQLITE_LOCKED`, which is a different bug.
