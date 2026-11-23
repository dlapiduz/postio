# Gotchas: the store and the search index

The store is Turso (ADR 0038). Its planner and defaults are not SQLite's, and most failures here are silent.

- **`fts_score` returns `0.0` unless projected bare and sharing its parameter with `fts_match`.** `-fts_score(..)` and `?1` beside `?2` both give `0.0`. Score in a subquery, do arithmetic outside, reuse one explicit `?N`; a bare `?` before an explicit `?N` collides. Only order changes, so a ranking test needs an older, better match beating a newer one (`crates/postio-storage/tests/storage_suite/turso_capabilities.rs`).
- **A row-value cursor filters instead of seeking.** Keep the bare `received_at <= ?2` in front of `(received_at, id) < (?2, ?3)`.
- **The planner never reads through a partial index but still enforces a partial `UNIQUE`.** A `WHERE` on an index in `schema.rs` is a constraint, not an optimisation; add an unpredicated companion if the index is on a read path.
- **The planner does not bind equality through a `COLLATE NOCASE` index column, nor `IN` on a second column.** Fold the value into the key (`RfcMessageId::folded`; `idx_labels_account_name` still has the shape), or use `UNION ALL` of point lookups.
- **A `SEARCH` binding one column of a two-column key is a scan that a `SCAN` grep misses.** Plan gates name the columns they expect bound. An aggregate is one statement and one row, so counts cannot see it: keep `count(*)` off the first-frame path, and do not time a statement wrapped in `SELECT count(*) FROM (..)` (the engine prunes columns).
- **Without statistics the planner prefers the seek that binds more columns, even in a join.** A one-column lookup (`idx_messages_content`) loses to a two-column range (`idx_messages_account_list`) and the range is walked per outer row. Name the index with `INDEXED BY`, as `HITS_JOIN` in `crates/postio-index/src/executor.rs` does (#1809).
- **A correlated `count`/`min`/`max`/`avg`/`total` on a bare column pair may be rewritten "group-first".** The inner table is read whole (plan: `SEARCH scalar_subquery_tN` beside a `SCAN`). Make the outer side an expression (`f.message_id = m.id + 0`), as `crates/postio-index/src/executor/conversations.rs` does.
- **A bench sees `rows x other_table` only if both tables are seeded at real-mailbox scale.**
- **Never run `ANALYZE`.** Statistics turn hot sync statements into scans; fix a plan with an index. `crates/postio-sync/tests/sync_suite/scan_audit.rs` asserts no `sqlite_stat1`.
- **The engine has no connection pool; each new connection starts cold.** Use `Store::read` for many reads, `Store::connect` for a writer, `Store::connect_background` for background work. Never open a raw connection: `store::PER_CONNECTION` sets `busy_timeout`, `foreign_keys` and the cache Turso defaults get wrong.
- **`WriteGate`: take the connection, then the permit, and hold one permit at a time.** It is not re-entrant, and a permitless writer starves during sync. `let permit = gate.acquire(p);` without `.await` compiles and does nothing; only a test catches it.
- **The outermost write transaction must be `BEGIN IMMEDIATE`.** A deferred one that reads first fails with `Busy` at once. Use the scope in `crates/postio-storage/src/sql.rs`.
- **Triggers in `schema.rs` maintain mailbox counts.** Add no recount to a write path; a low `total_count` renders an empty mailbox.
- **A list-row column belongs only in `LIST_COLUMNS`.** A second spelling broke paging two crates away with every storage test green.
- **Postio folds accents itself, on both sides.** Index and query text both go through `postio_model::fold`; the tokenizer folds only case.
- **Exclude negated free text in the `WHERE`, outside the match.** Inside the metadata match, `NOT "spam"` admits a message whose body has "spam". Adding a column to the broad path's candidate statement can lose its plan.
- **Index bodies through `postio_index::index::index_body_of` (`Document::to_search_text`).** `to_text` keeps link addresses and `[image]`, so messages would match words they never showed.
- **`Engine::request_body` queues the fetch.** Wait with `postio_host::parts::wait_for_body`; treat a busy read as "look again".
- **A body fetch replaces the message's attachment rows.** An `AttachmentId` does not survive it; resolve to the MIME part path first. Blob ids hash the decoded payload.
- **On-demand reads need the message's own account engine.** The wiring slot holds only the first engine. Use the host's account lookup for bodies, parts and source exports; never broadcast a fetch.
- **The schema stamp hashes `HEAD`'s text, comments and spacing.** A mechanical rewrite touching a comment in `HEAD` fails `a_schema_change_comes_with_the_migration_that_reaches_it`.
- **A schema change edits `schema::HEAD` and adds a `schema::MIGRATIONS` step.** Keep the replaced `HEAD` in `crates/postio-storage/tests/schemas/`. An unreachable store gets `Remedy::StartOver`. The engine has no read-only open: diagnose a copy.
- **Only a keyring `NotFound` mints a store key.** `Locked`, `Timeout`, `Backend` mean no answer, and treating them as first run destroys the store. Never `#[derive(Debug)]` on key material.
- **Reclamation grace periods are load-bearing.** A blob is written before its row commits, and the file-transfer portal hands out paths, so early reclaim yields a silent empty drop (`DRAG_EXPORT_GRACE_PERIOD`). Back-date mtimes to test; never shorten.
