# A shared-cache in-memory SQLite database races with a running engine

*Archived 2026-09-14: the same dead mechanism as the "database table is locked" note — a shared-cache `:memory:` store, which `memory()` stopped being at #204 and which the Turso engine (ADR 0038) does not have.*

Writing to a `test_support::memory()` database from the test thread while an
`Engine` is running against the same database fails with `SQLITE_LOCKED`
(extended 262, "database table is locked") rather than waiting —
shared-cache in-memory SQLite takes table locks that `busy_timeout` doesn't
cover. A file-backed production database in WAL mode *does* wait, so this is
a test-harness shape only. Do all account/identity/mailbox setup **before**
`Engine::spawn`; the engine writes on link-up (folder discovery) and again on
every drain, so the collision window isn't small. Symptom: an intermittent
failure in an unrelated assertion, roughly 1 run in 4.
