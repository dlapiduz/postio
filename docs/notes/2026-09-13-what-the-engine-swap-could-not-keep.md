# What the engine swap could not keep

2026-09-13, `feature/turso-store`.

Postio's storage layer is Turso now — the Rust rewrite of SQLite — rather than
SQLCipher through `rusqlite`. Most of it was a transcription: 96 of 102
head-schema objects applied unchanged, all fifteen triggers among them. This is
the other six per cent: the things that did not survive, what replaced them,
and what the replacement cannot do.

Each has a test pinning the current behaviour, so a later release that fixes
one of these **fails** rather than going unnoticed.

## 1. Diacritic folding moved into the application

FTS5 tokenised with `unicode61 remove_diacritics 2`, so `jose` found `José`
without anybody arranging it. This engine's tokenizer is tantivy's
`SimpleTokenizer` + `LowerCaser`, which folds case and not accents.

So folding is Postio's now: `postio_model::fold` — lowercase, NFD, drop
combining marks — applied on the way **into** the index and on the way into
**every query**. `messages.body_search` holds the folded text; the query path
folds the same way. The two have to stay in step, and the thing that keeps them
honest is that they are the same function.

`turso_capabilities::the_engine_folds_case_but_not_diacritics` pins the
tokenizer's behaviour, so a release that starts folding accents fails there and
says the column and the fold are no longer needed.

## 2. The cost gate lost sight of rows *examined*

`test_support::counting` read SQLite's `trace_v2` hook: every statement as it
finished, with its row count and its VM step count. There is no trace hook.

What is left is this crate's own seam — every read goes through
`postio_storage::sql`, and every one of them is counted. That sees **statements
and rows**, exactly, which is what most budgets are written in.

It cannot see **steps**, and #1479 is what that costs. An aggregate that scans
the whole table is one statement returning one row: invisible to both counts.
`counting::scans` is the structural replacement — it asks the planner whether a
query *can* be cheap — and it is weaker in a nameable way: it sees a `SCAN` of a
table and is blind to a `SEARCH` that seeks one column of a wide index and then
walks everything under it. Measured, #1479's own aggregate is the second kind.

`app_suite/startup_reads.rs` asserts that blindness rather than papering over
it, and fails if the engine ever reports it honestly.

## 3. A partial index is either a constraint or nothing

The planner will not read through a partial index; it does enforce a partial
`UNIQUE`. Sixteen predicates that existed to keep an index small are gone, and
the store is larger for it — see
`2026-09-12-a-partial-index-the-planner-will-not-read.md`.

## 4. Four indexes went with them

Each of those sixteen left an index that was a prefix of a wider one. Four were
exactly that and nothing more, and the planner preferred the shorter one and
then sorted — which is how the thread list acquired a sort over the whole
folder. They are gone.

## 5. There is no `dbstat`, and no full vacuum worth running

`dbstat` gave page usage per b-tree, and three size measurements read it. They
weigh the **file** between two states now: coarser, in that a table cannot be
separated from its index; truer, in that it counts everything a step adds to
the disk.

Worse, `PRAGMA auto_vacuum` is behind an experimental flag the Rust builder does
not expose, so **a Postio store grows and does not shrink**. A full `VACUUM`
exists behind `Builder::experimental_vacuum` and rewrites the entire database,
which on the reference store is not a startup-path operation. #381's conversion
is archived rather than deleted, and
`app_suite/reclaim_pages.rs` fails the moment the pragma arrives.

## 6. Bodies are plain text

ADR 0020 compressed message text against a trained zstd dictionary. The
full-text index is an index **over a column** here rather than a table beside
one, and an index cannot tokenise compressed bytes. The dictionary, its trainer
and the idle pass that ran it are gone.

## 7. No read-only open

There is no `SQLITE_OPEN_READ_ONLY` and no `PRAGMA query_only` in the engine's
Rust API. The four diagnostics under `examples/` that relied on them now say
**point this at a copy** in their own words, which is the instruction they
always carried and is now the whole of the protection.

## 8. `fts_score` is easy to lose

Two ways, both silent, both pinned in `turso_capabilities.rs`: any arithmetic
around the call answers `0.0`, and so does a query term bound as a *different
parameter* than the `fts_match` that selected the row. See
`2026-09-13-a-score-that-is-zero-and-says-nothing.md`.

## What it bought

`openssl-src` left the dependency graph entirely — with it, a 28-second
uncacheable build step per cold tree, and the `atexit` workaround #794 and #699
needed. The page cipher is AES-256-GCM rather than AES-CBC plus HMAC-SHA512,
which is the ratio `hmac_cost` existed to complain about: 45.9% of sampled CPU
on a real mailbox was in SHA-512, and that code is gone.

Both the encryption and the full-text search are marked experimental upstream
and neither has been audited. `spec.md` records that as accepted; it is the
reason a store is rebuilt by resyncing rather than migrated.
