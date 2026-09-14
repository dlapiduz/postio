# A test that spawns a real `Engine` needs `test_support::temp()`, not `test_support::memory()` — `:memory:` has no WAL

*Archived 2026-09-14: the premise is false now — `memory()` is file-backed on `/dev/shm` since #204, and the Turso engine (ADR 0038) has neither shared cache nor `SQLITE_LOCKED`, so `memory()` and `temp()` differ only in whether the test controls the path.*

#109 tracked
`reading::tests::a_part_nobody_has_is_fetched_before_it_is_saved` failing
about once in a dozen runs, load-correlated, always finishing in exactly the
engine's `POLL_INTERVAL` (5s) whether it passed or failed — which read as a
timing coincidence worth chasing but wasn't the mechanism. Reproduced under
sustained moderate CPU load (`yes > /dev/null` on half the cores, matching
"two other worktrees compiling"): `world()`'s own setup query panicked with
`SQLITE_LOCKED` ("database table is locked: messages"), and separately the
test's own follow-up read failed the instant after the awaited fetch had
already succeeded. Neither took anywhere near the 30s `BODY_WAIT` deadline —
both failed as fast as a query returns, which is what actually pointed away
from the poll interval and at SQLite.

The mechanism: `test_support::memory()` opens a shared-cache `:memory:`
database, which cannot use `journal_mode = WAL` — there is no file to write
a WAL against, so it falls back to `memory` journalling, where a writer and
a reader on the same table can collide as `SQLITE_LOCKED_SHAREDCACHE`. That
is a different error from `SQLITE_BUSY`, and critically, `busy_timeout`
does not retry it — recovering from `SQLITE_LOCKED` needs SQLite's
unlock-notify API, which this pool does not use, so the error comes back on
the first try, immediately. A test with only one connection never notices;
one that spawns a real `Engine` on its own thread — anything that touches
`MailBackend`, not the seeded-fixture kind — is running exactly the writer
that can collide with it. `world()` in `reading.rs` does; the fix was
switching it to `test_support::temp()`, which is file-backed and gets the
same WAL guarantees production reads run under. `test_support`'s own doc
comment already said as much ("`temp` when the test is *about* the
file — WAL behaviour... because an in-memory database has no journal"); the
part worth remembering is that "about the file" includes any test running a
concurrent writer, not only tests that reopen or inspect the file directly.
