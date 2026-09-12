---

description: "Task list for feature implementation"
---

# Tasks: The store, rebuilt on Turso

**Input**: Design documents from `/specs/004-turso-store/`

**Prerequisites**: spec.md, plan.md, research.md, data-model.md, contracts/, quickstart.md — all written and committed

**Tests**: Test-first is Constitution IV and NON-NEGOTIABLE. Every task below
that changes behaviour names the test *first* and the code second, and the
test is **observed red** before the code is written. A task whose test was
never seen red has not been done.

## Format: `[ID] [P?] [Story] Description`

- **[P]** — touches files nothing else in its phase touches, so it can run beside its siblings
- **[US#]** — the user story it serves; setup, foundational and polish tasks carry none

## Path Conventions

Paths are workspace-relative from `~/src/postio-worktrees/turso-store`.
Commits end `Refs: specs/004-turso-store` and the task id — never `Refs: #<issue>`.

## Two rules that hold for every task

1. **`postio-search` is not edited.** It parses the query language and knows
   nothing about execution. If a task needs to touch it, the design is wrong
   and the task stops for a re-think.
2. **`postio-gtk` learns nothing about the engine.** It reads through
   `MailStore`, whose shape does not change. A diff that reaches `postio-gtk`
   with a database word in it is the boundary failing.

---

## Phase 1: Setup

- [ ] T001 Add `turso = "=0.8.0-pre.11"` to `crates/postio-storage/Cargo.toml` and remove `rusqlite`, leaving the crate non-compiling — this task's only claim is that the dependency resolves and `openssl-src` leaves the graph, proved by `cargo tree -i openssl-sys -e normal` answering "did not match any packages"
- [ ] T002 Ban `turso` wherever `rusqlite` is banned in `scripts/checks/check-crate-boundaries.py` (`postio-gtk`, `-model`, `-config`, `-search`, `-body`) and add its test — a check that silently stops checking when a dependency is renamed is worse than no check
- [ ] T003 [P] Record the engine swap in `docs/decisions/` as an amendment to ADR 0014, citing the three spikes rather than re-arguing them

---

## Phase 2: Foundational (blocks every user story)

**The two experiments come first because three later tasks branch on their
answers.** Neither is allowed to end in a guess: each writes its result into
`specs/004-turso-store/research.md` under the question it answers.

- [ ] T004 **R1** — determine whether Turso can build an fts index over a generated column, in `crates/postio-storage/tests/turso_capabilities.rs`: create `body_indexed AS (coalesce(body_search, body_text)) VIRTUAL`, attempt `CREATE INDEX … USING fts (body_indexed)`, and assert either that it works or that it does not. Write the answer into research.md Q1. **T033 and T034 depend on which it is.**
- [ ] T005 **R2** — determine whether a background writer can starve an interactive one, in `crates/postio-storage/tests/turso_capabilities.rs`: hold a long write while a second connection attempts a short one, and measure whether the short one waits unboundedly. Write the answer into research.md Q3. **T014 depends on it.**
- [ ] T006 Write `crates/postio-storage/src/schema.rs`: the head schema as one constant, with the four changes data-model.md names — bodies as TEXT, `body_search` added, `WITHOUT ROWID` dropped from `thread_links`/`message_labels`/`message_headers`, the two FTS5 virtual tables replaced by fts indexes. Test first: a test that every table, index and trigger the old schema declared is present, so nothing is lost by transcription
- [ ] T007 Write `crates/postio-storage/src/store.rs` replacing `db.rs`: `Store::open(path, key)` over Turso with encryption on, schema at head on a new file. Test first: opening a fresh path yields a store whose schema is at head
- [ ] T008 Delete `crates/postio-storage/src/db.rs`, `encrypt.rs` and `body.rs`, and the tests that assert SQLCipher's own behaviour (`encrypt_migration`, `page_mac`, `key_pragma_failure`, `hmac_cost`, the `cipher_*` pragma cases in `concurrent_open`). Each deletion says in one line which engine behaviour it was about — a deleted test with no explanation is indistinguishable from a lost one
- [ ] T009 Port `crates/postio-storage/src/test_support.rs` to file-backed Turso stores only, per research Q6 — the engine refuses to key an in-memory database

**Checkpoint**: the crate does not compile yet and is not expected to. Nothing below starts until T004 and T005 have written their answers down.

---

## Phase 3: User Story 1 — A mailbox that opens, encrypted (P1) 🎯 MVP

**Goal**: a store that exists, is encrypted, and refuses everything it should.

**Independent test**: create a store through the application's own opening
path, then read the file — not `SQLite format 3`, no table name or message
text in the clear, another key refused.

### Tests first

- [ ] T010 [P] [US1] `crates/postio-storage/tests/storage_suite/encryption.rs` — a store created through `Store::open` holds no table name and no message text in its raw bytes, and a second key is refused. Rewritten rather than ported: it currently asserts SQLCipher's pragmas
- [ ] T011 [P] [US1] `crates/postio-storage/tests/storage_suite/schema_fidelity.rs` — every object the schema declares exists after an open, and a database this build did not write is refused rather than opened partly (FR-003)

### Implementation

- [ ] T012 [US1] Make `Store::open` refuse a foreign or unreadable database with a sentence meant for a person, in `crates/postio-storage/src/store.rs` — ADR 0014 Q3's rule is unchanged, only the engine is
- [ ] T013 [US1] Port `crates/postio-session/src/lib.rs`'s `open_store_at` to `Store`, dropping the ADR 0014 Q4 plaintext migration entirely (spec: no migration, by instruction) and the `store_key` path unchanged — keyring, raw key, no passphrase KDF
- [ ] T014 [US1] Keep or drop `WriteGate` and `Pool` in `crates/postio-storage/src/store.rs` **according to T005's answer**, and say which in the commit body
- [ ] T015 [US1] `crates/postio-session/examples/prove_cipher.rs` — the quickstart's US1 command, reporting cipher, schema object count, header bytes, a plaintext scan and a wrong-key refusal

**Checkpoint**: `cargo run -p postio-session --example prove_cipher` passes. The store is real and encrypted; nothing reads or writes mail yet.

---

## Phase 4: User Story 2 — Mail arrives and can be read (P2)

**Goal**: sync writes, the list fills, a message opens. The bulk of the
surface — 17 repositories and everything downstream.

**Independent test**: against a loopback IMAP server, drive a first sync and
assert the rows land, the list model fills and a body opens — the assertions
the existing suites already make, over the new engine.

### Repositories — each is test-first, and each is [P]

Each task: run that repository's existing suite against the new engine, watch
it fail, port the repository until it passes. The suites are the specification
and **must not be weakened to fit** — a suite that needed changing is a finding
for the commit body.

- [ ] T016 [P] [US2] `accounts` — `crates/postio-storage/src/repository/accounts.rs`, suite `tests/storage_suite/accounts.rs`
- [ ] T017 [P] [US2] `mailboxes` + the count triggers — `repository/mailboxes.rs`, suites `mailboxes.rs`, `mailbox_counts.rs`, `mailbox_size.rs`
- [ ] T018 [US2] `messages` — `repository/messages.rs`, suite `messages.rs`. Not [P]: the largest repository, and bodies change shape here (TEXT, and `body_search` written when folding changes the text)
- [ ] T019 [P] [US2] `threads` + `threading` — `repository/threads.rs`, `repository/threading.rs`, suite `threads.rs`
- [ ] T020 [P] [US2] `drafts` — `repository/drafts.rs`, suites `drafts.rs`, `draft_indexes.rs`
- [ ] T021 [P] [US2] `contacts` + `contact_groups` — suites `contacts.rs`, `contact_groups.rs`, `contact_rank_index.rs`
- [ ] T022 [P] [US2] `labels` — `repository/labels.rs`, suite `labels.rs`
- [ ] T023 [P] [US2] `operations` — `repository/operations.rs`, suite `operations.rs`
- [ ] T024 [P] [US2] `settings`, `sync_state`, `egress`, `unsubscribe`, `mailbox_roles`, `cross_account` — the small ones, one commit each
- [ ] T025 [P] [US2] `actions.rs` and `bulk` — `repository/mod.rs`'s bulk paths, suites `actions.rs`, `bulk.rs`
- [ ] T026 [P] [US2] `seed.rs` — the seeded-store helper every measurement depends on, suite `seed_is_honest.rs`

### The layers above

- [ ] T027 [US2] `crates/postio-runtime/src/store/sqlite.rs` → a thin async adapter, keeping every `MailStore` method and signature exactly (contract 2). Test first: `postio-runtime`'s own suite
- [ ] T028 [US2] `crates/postio-sync` — the write paths become awaits rather than `spawn_blocking` closures. Test first: its suite, against the mock backend
- [ ] T029 [US2] `crates/postio-session` — `Wiring`, the housekeeping passes, `begin_session`. Test first: `session_suite`
- [ ] T030 [US2] `crates/postio-app` — the composition root follows; `feed_the_window` and the settings panels. Test first: `app_suite`
- [ ] T031 [US2] Confirm the boundary held: `git diff --stat origin/main -- crates/postio-gtk crates/postio-search` is empty, and T002's check passes

**Checkpoint**: `cargo nextest run -p postio-storage -p postio-sync -p postio-session -p postio-app` green. Mail syncs, lists and reads. Search does not work yet.

---

## Phase 5: User Story 3 — Search that finds what it finds today (P3)

**Goal**: the same queries return the same messages, diacritics included.

**Independent test**: one corpus, both engines, the same query strings. Any
message today's search returns that the new one does not is a failure.

### Tests first

- [ ] T032 [P] [US3] `crates/postio-index/examples/search_equivalence.rs` — the acceptance for SC-002. Index one corpus under both engines, run the same queries, diff the result sets. It **must** include a query whose term differs from the text only by diacritics; that case is expected red until T034

### Implementation

- [ ] T033 [US3] Rewrite `crates/postio-index/src/index.rs`: the two fts indexes, the `message_bodies` table, and deletion that actually deletes (FR-011). Shape follows **T004's answer** — a generated column if it can be indexed, a plain folded column if not
- [ ] T034 [US3] Folding, in `crates/postio-index/src/index.rs`: NFKD, drop combining marks, lowercase — applied identically on the way into the index and into a query. Test first: `José` is found by `jose` and `Jose` by `josé`, both directions
- [ ] T035 [US3] Rewrite `crates/postio-index/src/executor.rs`: `MATCH`/`bm25()` → `fts_match`/`fts_score`, the two result sets merged as today
- [ ] T036 [US3] Re-derive the ranking weights in `executor.rs`'s `rank_score`: the relevance term's scale changes with the engine, so `RECENCY_WEIGHT` and `SENDER_WEIGHT` are re-measured against it rather than carried over. Test first: a more recent message outranks an older one of equal textual relevance, and a frequent correspondent outranks a stranger
- [ ] T037 [US3] Highlighting via `fts_highlight`, or the existing `postio-search::highlight` if it is engine-independent — check before replacing

**Checkpoint**: `search_equivalence` reports no missing messages, diacritics included.

---

## Phase 6: User Story 4 — It still feels instant (P4)

**Goal**: the budgets are defended by something that says the same thing on
any machine.

**Independent test**: a gate that fails when a read becomes proportional to
the mailbox, over two stores an order of magnitude apart.

- [ ] T038 [US4] Replace `crates/postio-storage/src/test_support/counting.rs`: count at the storage seam — statements issued and rows returned — since Turso exposes no trace hook (research Q5). Its docs **must** state plainly what it can no longer see: rows *examined*, which is the count that caught #1479
- [ ] T039 [US4] Port `crates/postio-app/tests/app_suite/startup_reads.rs` to the new counter, keeping its claim exactly: opening a window costs the same over two stores an order of magnitude apart
- [ ] T040 [US4] Port `crates/postio-storage/tests/storage_suite/list_statement_count.rs` and `threads.rs`'s flat-paging case to the new counter
- [ ] T041 [US4] Make an unbounded read visible (FR-017): a debug assertion, or a repository API that cannot express a query without a limit. Decide which in the commit body — this is the property §18 rests on and convention is not enough
- [ ] T042 [US4] Measure the real numbers and record them in `docs/PERFORMANCE.md`: startup on the reference mailbox, a page read, a search, and the store's size against the old one — the size regression is expected and must be stated rather than discovered

---

## Phase 7: Polish & cross-cutting

- [ ] T043 [P] Update `docs/ARCHITECTURE.md` and `docs/PRODUCT.md` where they name SQLCipher, FTS5 or compression
- [ ] T044 [P] A note under `docs/notes/` on what the engine swap cost and what it could not keep — the diacritics fold moving into the application, and the cost gate's lost sight of rows examined
- [ ] T045 Run `scripts/check.sh` and the full workspace suite; fix what the port left
- [ ] T046 The quickstart's end-to-end run against a real account, on a **fresh store, never the live one** — the acceptance for SC-001, and the one a test cannot give

---

## Dependencies & Execution Order

### Phase dependencies

```text
Phase 1 Setup
   └─> Phase 2 Foundational ── T004 (R1) and T005 (R2) gate T033/T034 and T014
          └─> Phase 3 US1  (MVP: an encrypted store that opens)
                 └─> Phase 4 US2  (mail syncs, lists, reads)
                        ├─> Phase 5 US3  (search)
                        └─> Phase 6 US4  (the cost gate)
                               └─> Phase 7 Polish
```

### User story dependencies

- **US1** depends on nothing but Phase 2. It is the MVP and it is where the non-negotiable lives.
- **US2** needs US1 — there must be a store to write to.
- **US3** needs US2 — there must be mail to index.
- **US4** needs US2 for a read path to measure, and is independent of US3.

US3 and US4 can run in parallel once US2 is green.

### Parallel opportunities

- Phase 2: T004 and T005 are independent experiments.
- Phase 4: T016–T026 are eleven repositories in eleven files. They are the bulk of the work and almost all of it parallelises.
- Phase 5: T032 is written before and beside T033–T037.
- Phase 7: T043 and T044 are documentation and independent.

## Implementation strategy

**US1 is the MVP and it is small.** A store that opens, is encrypted, and
refuses a wrong key is a complete, demonstrable increment — and it is the one
that must be right before anything writes mail into it.

**Phase 4 is the project.** Eleven repositories, each with a suite that
already specifies it. The discipline that matters there is that the suites are
the specification: a suite that needed changing to pass is a finding worth a
paragraph in the commit body, not a quiet edit.

**Stop and report rather than push through** if: an experiment's answer makes
a task impossible as written; a suite cannot pass without being weakened; or
`postio-gtk` or `postio-search` need editing. All three mean the plan was
wrong, and the plan is cheaper to change than the code.
