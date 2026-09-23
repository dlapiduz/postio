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

## 5. There is no `dbstat`, and the vacuum is all-or-nothing

`dbstat` gave page usage per b-tree, and three size measurements read it. They
weigh the **file** between two states now: coarser, in that a table cannot be
separated from its index; truer, in that it counts everything a step adds to
the disk.

`PRAGMA auto_vacuum` is behind an experimental flag the Rust builder does not
expose, and there is no `incremental_vacuum` step to drive it with even if it
were. So #381's answer — hand a few freed pages back on every housekeeping
pass, at no perceptible cost — is gone. What replaced it is a full `VACUUM`,
via `Builder::experimental_vacuum(true)`, and the four things measured about
it on the 868 MB reference store:

- it reclaims **87–88%** of the free space, at **17–25 MiB/s** — 35 to 50
  seconds there;
- it needs **1.1x** the file in peak disk, because it builds the new database
  beside the old one;
- it **blocks every writer for its whole duration**. A keystroke's write
  waited 2,831.8 ms behind a 2.80 s vacuum, which is the exact stall
  `WriteGate` and #425 exist to prevent;
- **killing it is safe.** Interrupted at six points, the file was byte-identical
  each time and every row still read.

And the thing that makes the whole question less urgent than it looked: **freed
pages are reused.** Deleting 18,000 messages and adding 18,000 back grew the
file by 0.0 MiB. A store plateaus at its high-water mark rather than climbing,
so what is actually lost with `auto_vacuum` is the *return of a one-off
shrink* — after a `UIDVALIDITY` reset, an archive cleared — not a defence
against unbounded growth.

Hence a policy instead of a step: `Store::is_worth_reclaiming` asks for the
holes to be both ≥64 MiB and ≥25% of the file before `reclaim_free_pages` is
allowed to stall the writers, and the housekeeping worker asks on each pass.
The thresholds are `store::reclaim_policy`'s to prove;
`app_suite/reclaim_pages.rs` proves the application reaches them, and its
second case fails the moment the incremental pragma arrives — which is the
signal to take #381's conversion back out of the archive.

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

## Five more, found reconciling the documentation

Smaller than the eight above, and found by reading the docs against the code
rather than by a failing test.

**ADR 0027's gate number moved, and so did its method.** The header index
weighs **4,218 B** a message on this engine against 3,809 B under SQLCipher —
same policy, same fixture, weighed as a file delta because there is no
`dbstat` to attribute pages with (`index_suite/header_index_size.rs`). The
5 KiB ceiling stands; the headroom under it is about a fifth now rather than
a third.

**`busy_timeout` defaults to 0.** A writer that finds the lock taken gets
`Busy` immediately, with no retry at all — SQLCipher's configuration set
5,000 ms and `WriteGate`'s documentation assumes it. The per-connection batch
sets it back to 5,000; drop that line and
`a_resync_batch_does_not_lock_out_an_interactive_write` fails as `database is
locked`, which is the symptom a keystroke during a sync would show.

**`cache_size` defaults differently too**: -2000 (2 MiB) where the old store
ran at 64 MiB. The batch sets -65536 — a cap rather than a reservation, so a
small store never allocates it.

**The connection pool is gone entirely.** No `PooledConnection`, no checkout
timeout, no `spawn_blocking` bridge: the engine keeps its own pool behind a
cheap `connect`, and the async API removed the synchronous-repository shape
that needed the thread pool. `store.rs`'s module doc has the full account.

> **Correction, 2026-09-23.** The engine keeps no pool. `Database::connect`
> builds a new pager with an empty page cache for every connection, so
> "cheap" was true of the call and false of what followed it: every read
> started cold over the encrypted file, and every connection held its own
> 64 MiB cap. See `docs/notes/2026-09-23-the-engine-keeps-no-pool-a-connection-is-a-cold-cache.md`
> and #1602.

**`PRAGMA page_size` is read-only.** The store reads it in two places for
freelist arithmetic and can set it nowhere, so the choice
`2026-09-04-the-page-size-has-to-be-chosen-before-300-and-8192-is-the-an.md`
recorded — 8192, decided before #300 because it could never be revisited — is
unmakeable here in either direction: the page size is the engine's. That note
went with the engine; #381's commits hold the reasoning if it is ever wanted.

## What it bought

`openssl-src` left the dependency graph entirely — with it, a 28-second
uncacheable build step per cold tree, and the `atexit` workaround #794 and #699
needed. The page cipher is AES-256-GCM rather than AES-CBC plus HMAC-SHA512,
which is the ratio `hmac_cost` existed to complain about: 45.9% of sampled CPU
on a real mailbox was in SHA-512, and that code is gone.

Both the encryption and the full-text search are marked experimental upstream
and neither has been audited. `spec.md` records that as accepted; it is the
reason a store is rebuilt by resyncing rather than migrated.
