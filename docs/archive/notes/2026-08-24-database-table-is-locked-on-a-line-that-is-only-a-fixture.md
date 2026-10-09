# "database table is locked" on a line that is only a fixture

*Archived 2026-09-14: the shared-cache `:memory:` mechanism this diagnoses is gone — `memory()` has been file-backed since #204 and the engine is Turso since ADR 0038, which has no shared cache to reintroduce.*

It meant the
scratch database, not your test. Until #204, `test_support::memory()` was
`:memory:` with `cache=shared`, whose *table-level* locks return
`SQLITE_LOCKED` immediately — `busy_timeout` covers only the file lock, so
no pragma waited it out, and the failure rate tracked machine load. A read
transaction on one pooled connection (a list page mid-iteration) failed a
plain write on another, in a test about something else entirely. Fixed by
making `memory()` file-backed in a self-cleaning tempdir (`/dev/shm` where
present, so it still costs RAM); the tempdir rides inside the pool via a
guard slot, so clones of the `Database` keep it alive. If that error string
ever reappears, something reintroduced shared cache — start at
`Database::open_in_memory`'s doc comment, which now records the caveat.
