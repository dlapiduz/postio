# Where a search and a store actually spend their time (2026-09-06, #1216)

A night of measurement against a real 82,057-message store, after a day of
measuring synthetic ones. Every number here is from `scripts/` tooling that
still exists, so it can be re-run rather than believed.

## The one-line version

**Every cost that mattered was in a query plan or a widget lifecycle, and none
of them was visible in a CPU profile.** The profile said "SQLCipher is busy",
which was true and led to two real but secondary fixes. `store_diag` said
which *statement* was busy, and that led to a 78x search.

## The order things actually cost, on real mail

Cold search for `invoice`, one term, through the whole session:

```
                              before      after
search, total               4,619 ms      59 ms
  the fetch                 4,500 ms      12 ms   join form (#1275)
  the hydrate                  41 ms      28 ms   one sender lookup, not three
  the count                    78 ms      17 ms   (unchanged; it got cheaper
                                                   because the corpus did)
```

Two fixes, both in `postio-index`, neither about crypto:

1. **Ordering and join form were one decision.** Past
   `RANK_BY_RELEVANCE_LIMIT` a match is too broad to rank, so it orders by
   recency — and the same threshold flipped the join to a shape that walks
   `messages` newest-first asking each row "did you match?". That is fast when
   matches are dense in recent mail and a deep scan when they are scattered
   through old mail. `PROBED_FORM_LIMIT` splits the two decisions.
2. **Hydrate asked `recipients` for the sender three times per candidate**,
   and two of those were the same row reached by the same join. 11.7x,
   measured by alternating both shapes on one connection.

## What a CPU profile could not tell us, and what could

Five perf captures agreed: 45–56% of samples in `sha512_block_data_order_avx2`
under a btree walk. All true, and it named nothing. `postio_storage`'s own
symbols total **0.04%** of a capture, because the time is inside SQLite's and
SQLCipher's C, and neither frame-pointer nor DWARF unwinding gets out of
hand-written AVX2 assembly — frame pointers *invent* callers there (the ones
reported were SHA-512's own round constants).

`store_diag` (#746) profiles per *statement*, with a warm pass and a
larger-cache pass, and answered in one run what five captures could not. It
already existed. **When a repository has a diagnostic for the class of problem
you are chasing, that is the first thing to reach for, not the last.**

## The crypto fixes were real and secondary

* `cache_size` 16 → 64 MiB. 24% at 400,000 messages (`cache_pressure.rs`),
  and **nothing at all for search**: 16x the cache moved a 4.59 s search by
  1%, because a scan re-reads whatever it evicts. Cache helps queries that
  have locality; it cannot rescue one that has none.
* Page MAC HMAC-SHA512 → HMAC-SHA256. 1.7x on the page-read path
  (`hmac_cost.rs`), because `sha_ni` accelerates SHA-1 and SHA-256 and not
  SHA-512, while AES-NI handles the cipher — so the MAC was the only part of
  the page path still in software. Not portable: without the SHA extensions
  SHA-512 is the faster of the two.

## The largest structural cost left: bodies live in `messages`

Measured with `table_shape.rs`, on a store **still backfilling** (6,068 of
82,057 bodies downloaded):

```
messages                    89.9 MiB   46.7% of the database
  body columns                          66% of that table
  rows per 4 KiB page        6.0 as stored, 17.8 with bodies moved out (3.0x)
recipients                  13.5 MiB    7.0%
message_headers             13.2 MiB    6.9%
search_documents            11.8 MiB    6.1%
```

`body_text`, `body_html` and `body_headers` are columns of `messages`. Every
query that walks message rows — the list, the search hydrate, the count —
therefore touches **three times more pages than it needs**, and under
SQLCipher a page touched is a page decrypted and MAC-verified.

The win scales with how much body text is stored, and this store is early in
its backfill: the store this replaced held 71,422 inline bodies against
today's 6,068. Moving them to a side table keyed by `message_id` is a schema
migration and a change to the body read/write path, and it is the biggest
single lever left on the engine side.

## What was ruled out, so nobody pays for it again

* **Sharing a `WebContext` does not share a web process.** Three views on one
  context gave three processes. WebKitGTK runs one per *view*.
* **The message list is healthy.** Every access an index seek, 766 µs for the
  first page of a 60,898-message mailbox, 1.65 ms at 5,000 rows deep. Covering
  indexes were the top of my improvement list and the measurement dismissed
  them; the `BtreeTableMoveto` in the profile was the search path.
* **`fts5vocab` is not a cheap doc-frequency oracle.** It still walks the
  doclist — about 1.5x cheaper, not a step change — so it does not separate
  the planner's "how broad is this" from the display's "how many".
* **Lowering `TOTAL_HITS_CAP` makes broad searches ten times slower.** Two
  plan thresholds read that count; capped below them it saturates, the probed
  shape is never chosen, and `the` goes from 74 ms to 1.26 s.

## The UI side

* **A `WebView` starts its process on the first *load*, not when it is built.**
  So a reader built at the moment a message expands makes the person wait for
  a process to start, relocate its libraries and paint — and it composites
  black meanwhile. That was the reported "black flicker". Focus now warms the
  next reader; the process is running before it is wanted.
* **Nothing released those readers.** `EAGER_EXPANSION_CAP` bounds how many
  open when a conversation *opens*; scrolling added one per message and
  `collapse` deliberately kept it, so a thirty-message thread ended with
  thirty web processes at roughly 50 MB each. `LIVE_BODY_CAP` windows them,
  the way the message list is windowed over paged SQLite.
* **The UI thread was busy drawing, not blocked on the store.** During
  interaction it took 19% of samples, of which 0.9% touched the store; CSS
  matching was its largest identifiable slice. Worth remembering that a cycles
  profiler cannot see a thread *waiting* — "not burning CPU on crypto" is not
  "never blocked on it", and that gap is still unmeasured.
* **ADR 0032** proposes the larger shape: one document for a whole
  conversation rather than a view per message. Left Proposed; its acceptance
  test is a screen-reader pass, not a benchmark.

## Two habits this cost enough to write down

**A measurement that flatters you is a measurement to distrust.** Scanning
253 MiB in 150 ms "proved" the MAC change worked; it was six times what the
CPU can hash, because `sum(length(blob))` reads the record header and skips
the overflow pages. The A/B that replaced it alternates both shapes and
discards the first round of each, because whichever runs first pays for pages
the other then finds warm.

**Check who else reads the number you are about to change.** Lowering
`TOTAL_HITS_CAP` for the display would have silently pinned every broad search
to the wrong query plan, because two planner thresholds read the same count.
