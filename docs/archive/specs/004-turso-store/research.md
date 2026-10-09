# Phase 0 research: The store, rebuilt on Turso

Six questions the plan could not answer from the spikes. Each is settled with
a decision, the reason, and what was rejected. Two carry an experiment that
must run before the tasks that depend on them.

---

## Q1. Diacritics: where does folding live, and what does it cost?

**Settled by reading the engine.** `CREATE INDEX … USING fts` accepts one
option — `WITH (weights='body=2.0,subject=1.0')` — and **no tokenizer option**.
The analyzer is a thread-local constant:

```rust
TextAnalyzer::builder(SimpleTokenizer::default())
    .filter(tantivy::tokenizer::LowerCaser)
    .build()
```

Lowercasing, and nothing else. There is no way to ask it to fold, and
`fts_match` tokenizes the *query* through the same analyzer — so folding one
side only cannot work.

**Decision: fold in the application, on both sides, and store the folded text
only when folding changed it.**

Postio owns the write path and the query path, so it can apply the same
normalisation (NFKD, drop combining marks, lowercase) to text on the way into
the index and to terms on the way into a query. That reproduces
`unicode61 remove_diacritics 2` above the engine.

The cost is a second copy of body text, because the index reads a column and
the displayed body must stay as it was written. What keeps the cost small:
**for text that folds to itself — which is most mail in most mailboxes — the
folded column is NULL.** Only messages that actually carry diacritics pay.

**Experiment before the tasks that depend on this (R1):** determine whether
Turso can build an fts index over a *generated* column
(`body_indexed AS (coalesce(body_search, body_text)) VIRTUAL`). If it can, one
index covers both cases with no duplication at all for ASCII mail. If it
cannot, the index is built over `body_search` with the fold applied
unconditionally, and the NULL trick is lost — write that down rather than
discovering it.

**Rejected**: folding only the query (cannot work — the text keeps its
accents); expanding a query term to every accented variant (not tractable);
accepting the regression (FR-008, and Constitution III calls search one of
three things Postio must do better than the alternatives).

---


### Answered by experiment (T004 / R1), 2026-09-12

`crates/postio-storage/tests/turso_capabilities.rs` asks the engine directly.
Three facts came back, and the third is the one that decides the design:

1. **An fts index over a generated column works.** `CREATE INDEX … USING fts
   (body_indexed)` where `body_indexed` is `GENERATED ALWAYS AS (…) VIRTUAL`
   builds, populates, and matches. `STORED` is refused outright — "Stored
   generated columns are not supported" — so only `VIRTUAL` is on the table.
2. **Case is folded, diacritics are not.** `CAFÉ` finds `café`; `cafe` does
   not. That is `SimpleTokenizer` + `LowerCaser` exactly as the plan assumed.
3. **The fold has no SQL spelling.** A generated column may only be an SQL
   expression, and the engine has no `nfd`, `normalize`, `unaccent` or
   equivalent — `unicode()` returns a codepoint and `unistr()` decodes
   escapes. So a generated column *could* carry the index and *cannot* compute
   what needs to go in it.

**Decision: `body_search` is an ordinary column, written by the application.**
R1's mechanical answer is yes and it does not help. `postio_index::fold` is the
single writer, and it applies the identical fold to the query — both paths or
neither, or `café` and `cafe` stop meeting. The two tests that pin this are
`fold_cannot_be_expressed_in_sql` and `the_engine_folds_case_but_not_diacritics`:
if a normalising function ever lands, the first one fails and the column can
become generated after all.

**T033 and T034 are unblocked** and take the ordinary-column path.

## Q2. The shape of an async storage layer

**Decision: repositories become `async fn`, and the crossing moves out.**

Today `postio-storage` is synchronous and every caller reaches it through
`spawn_blocking` — `postio-runtime`'s `SqliteStore` does exactly that, and
`postio-session`, `-sync` and `-app` each do it by hand. With an async engine
those `spawn_blocking(move || …)` closures become ordinary `async` blocks and
`SqliteStore` becomes a thin adapter rather than a thread-crossing.

The rule that must not be lost in the change: **`postio-gtk` still never
awaits a store read on the main context.** The `Feeds` seam already answers
that — the frontend awaits a channel, never a query — and it stays.

**Rejected**: the `postio-turso` shim on `spike/turso-port`, which works and
is measured, but carries SQLite's synchronous shape into a database that is
neither SQLite nor synchronous. The maintainer rejected it for that reason and
the reason is right.

---


### A constraint found on the way (T004), 2026-09-12

**An undrained `Rows` holds its connection exclusively.** The next statement on
that connection fails with `Misuse("connection is busy with another
operation")`. Under `rusqlite` the borrow checker enforced this at compile
time, because a `Statement` borrowed its `Connection`; here it is a runtime
error, and a quiet one — it surfaces as a failure in whatever ran *next*, not
in the query that is still holding the gate.

What it means for the port: **a read must be finished before a write on the
same connection begins.** Collect rows into a `Vec` and drop the `Rows`, rather
than writing inside a loop that is still iterating. Connections are cheap
(`Store::connect` is not `async` and the engine pools them), so a second
connection is also a legitimate answer where the read genuinely has to stay
open.

## Q3. Connections: is there still a pool, and a write gate?

**Decision: keep the write gate, drop the pool, and prove which is needed.**

`Pool` exists because a rusqlite connection blocks a thread; that reason is
gone. `WriteGate` exists for a different reason — a first sync's backfill
starving an interactive write (#425, #672) — and that is a fact about Postio's
*workload*, not about SQLite. It stays until something shows it is redundant.

**Experiment (R2):** establish how Turso serialises writers and whether a
background writer can starve an interactive one. `connection.rs` carries a
`total_changes` counter and no priority mechanism, which suggests the
application must still arbitrate.

---


### Answered by experiment (T005 / R2), 2026-09-12

A second connection attempting a short write while the first holds `BEGIN
IMMEDIATE` **does not get through** — it either waits past the test's patience
or is refused as busy. Asserted as completion rather than as a timing, for the
same reason `bench.yml` times nothing: a shared runner cannot defend a
millisecond figure.

**Decision: the write gate stays.** The engine serialises writers, which is
correctness; it does not choose *which* writer, which is the thing
`WritePriority` exists for. A background backfill holding the writer while a
person's archive keystroke waits behind it is exactly the shape the old gate
was built to prevent, and nothing a layer down has taken that over.

**T014 is unblocked** and ports the gate rather than deleting it.

## Q4. The four indexes on `WITHOUT ROWID` tables

**Decision: drop `WITHOUT ROWID` from `thread_links`, `message_labels` and
`message_headers`.**

Turso supports those tables but refuses `CREATE INDEX` on them. The tables are
join tables whose whole purpose is lookup, so losing the index is not an
option; `WITHOUT ROWID` was a space and locality optimisation, and it is the
cheaper of the two to give up.

**Rejected**: keeping `WITHOUT ROWID` and scanning — which would put a
mailbox-proportional read on the label and header paths, the exact shape
FR-015's gate exists to catch.

---

## Q5. What replaces the counted-cost instrument

**Decision: count at the seam this project owns, not inside the engine.**

`postio_storage::test_support::counting` reads SQLite's `trace_v2` hook for
statements, rows and VM steps. Turso exposes no equivalent — no trace hook, no
statement statistics; `total_changes` is the only counter and it counts
writes.

So the instrument moves up one layer: the storage layer's own async entry
points count what they were asked for — statements issued, rows returned — on
the connection handle, in a test-only wrapper. That is strictly less than the
engine could tell us: it cannot see rows *examined*, which is the count #1479
needed to find a full scan hiding behind an aggregate.

**That loss is real and must be written down rather than glossed.** The
mitigation is that the two properties FR-015 actually states — bounded work
when opening a window, and a page costing what a page costs — are both
expressible in statements and rows returned, which the seam can see. A future
`EXPLAIN`-shaped assertion could recover the rest if Turso grows one.

**Rejected**: wall-clock thresholds (Principle V forbids, and #917 is what
they cost); dropping the gate (that is the state #1434 was filed about).

---

## Q6. In-memory databases

**Settled by the spike:** Turso refuses `PRAGMA key` on an in-memory database
— *"Setting key not supported for in-memory or temporary databases"*.

**Decision: `test_support` opens file-backed scratch stores only**, which is
what `test_support::memory` already does for a different reason (#204's
shared-cache locking). The handful of tests that construct an in-memory
database directly move to the same helper.

---

## Q7. Field weights, which we did not have before

`WITH (weights='subject=2.0,body=1.0')` is supported. FTS5's `bm25()` takes
column weights too, and Postio does not use them — it ranks two *tables*
separately and merges. The new index can express the same intent in one index
with weights, which is simpler.

**Decision: keep two indexes (metadata, body) for the first implementation**,
matching today's structure so the equivalence test in SC-002 has a like-for-like
target. Weights are a follow-up, not part of this feature.


## Q8. Partial indexes (found during T032, 2026-09-12)

**The planner will not read through a partial index. It does enforce one.**

Established directly in `turso_capabilities.rs`: a plain index on `(a, b)` is
used for `WHERE a = ? AND b = ?`; the same index with `WHERE a IS NOT NULL`
gets `SCAN`. A partial UNIQUE index still refuses a duplicate inside its
predicate and still allows the row outside it, so this is a performance limit
and not a correctness one.

It is a sharp one here: the schema had **22** partial indexes, and
`idx_messages_uid` is on the path `upsert_batch` takes once per message — a
scan there is a scan per message of every sync.

**What was done.** The sixteen non-unique ones dropped their predicates, which
costs a little index size and nothing else. The six unique ones kept theirs,
because the predicate is what makes `idx_identities_one_default` mean "one
default *per account*" rather than "one default", and gained a non-unique read
companion each. That is one more index to write on those six tables.

`the_planner_does_not_use_a_partial_index` fails the day this lifts, which is
when the companions can go and the predicates can come back.
