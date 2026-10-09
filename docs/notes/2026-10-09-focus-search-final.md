# Focus search, final: the bench, and what the common word costs

2026-10-09, specs/010-focus-search. The last measurement of the engine
before the final sweep (T137, T138 extend this note). Three shapes are still
over the 50 ms budget, all of them a word in nearly every message, and the
time is in one statement: the walk that feeds the facets.

## Bench

`crates/postio-bench/benches/search_focus.rs`, release build, Apple M1 Pro
(32 GB), in-memory store, 20,000 messages (step 1's corpus, step 9's
attachment units), 60 timed runs per shape after 5 warm-ups. Method as in
step 1's note: the bench's GTK dev-dependencies taken out of
`crates/postio-bench/Cargo.toml` for the build and put back, nothing about
that committed.

Built on `192d8a12`, which carries two changes since step 9's table:

- **A search no longer walks `attachments`** to say whether every
  downloaded attachment has been read (`bb2650fd`). The question is
  answered from `attachment_text_owed`, a set the index keeps by trigger,
  in one seek. `index_suite::driven_join_plan` now plans *every* statement
  `search_conversations` issues, not only those joined to the hits -- the
  gap this walk got through. The bench's attachments have no blobs, so
  the walk it removed was 6,000 rows of a `blob_id IS NOT NULL` filter:
  under a millisecond here, and growing with every attachment on disk in a
  real store. The statement now costs 0.08 ms.
- **A PDF skipped for want of the extraction helper is read once the
  helper is there** (`192d8a12`), rather than at the next extractor
  version. No effect on this bench.

`uptime` load average 4.1-4.8 while the shapes ran (other sessions
building). A second run at load 7-14 put every heavy shape 20-70% higher
(`common word` p95 95.7 ms); the table is the quieter run.

| shape | query | p50 ms | p95 ms | stmts | found |
|---|---|---:|---:|---:|---:|
| one word | `quarterly` | 3.36 | 3.92 | 3 | 203 |
| two words | `quarterly forecast` | 0.81 | 1.01 | 3 | 7 |
| operator only | `from:sender3` | 8.70 | 9.65 | 4 | 479 |
| operator + words | `from:sender3 regarding` | 19.83 | 20.68 | 4 | 460 |
| **common word** | `regarding` | 53.95 | **55.61** | 3 | 1,678 |
| typed `a` | `a` | 43.53 | 44.74 | 3 | 1,560 |
| **typed `at`** | `at` | 53.73 | **55.53** | 3 | 1,557 |
| typed `atl` | `atl` | 4.43 | 5.55 | 3 | 366 |
| completions | `a` | 4.59 | 5.37 | 4 | 2 |
| completions | `at` | 2.37 | 3.16 | 4 | 2 |
| completions | `atl` | 4.33 | 4.94 | 4 | 2 |
| completions | `from:a` | 1.57 | 1.77 | 1 | 4 |
| zero hits, four filters | (step 1's) | 1.02 | 1.38 | 2 | 0 |
| relaxations of that | five variants | 3.78 | 4.52 | 5 | 1 variant |
| e2e one word | `quarterly` | 4.74 | 5.57 | 56 | 203 |
| e2e two words | `quarterly forecast` | 1.23 | 1.56 | 13 | 7 |
| e2e operator + words | `from:sender3 regarding` | 20.88 | 21.88 | 57 | 460 |
| **e2e common word** | `regarding` | 55.24 | **57.01** | 56 | 1,678 |
| e2e typed `atl` | `atl` | 5.99 | 6.95 | 56 | 366 |
| preview check a person | `quarterly from:sender3` | 2.17 | 2.79 | 4 | 9 |
| preview exclude a person | `quarterly -from:sender3` | 4.66 | 6.02 | 4 | 194 |
| preview check a label | `quarterly label:atlas` | 1.30 | 1.49 | 4 | 21 |
| preview check a folder | `quarterly in:inbox` | 2.95 | 3.72 | 3 | 203 |
| timeline range + word | `quarterly after:… before:…` | 1.09 | 1.33 | 3 | 24 |
| date words since + word | `quarterly after:…` | 1.06 | 1.49 | 3 | 24 |
| timeline range alone | `after:… before:…` | 10.97 | 11.70 | 3 | 352 |
| timeline range + operator | `from:sender3 after:… before:…` | 3.14 | 3.93 | 4 | 66 |
| file word | `kestrel` | 1.04 | 1.30 | 4 | 20 |
| e2e file word | `kestrel` | 1.46 | 1.67 | 8 | 20 |
| Files tab, file word | `kestrel` | 0.49 | 0.63 | 3 | 20 |
| Files tab, one word | `quarterly` | 1.54 | 1.84 | 2 | 0 |
| Files tab, operator | `from:sender3` | 8.03 | 8.96 | 3 | 350 |
| e2e preview check a person | `quarterly from:sender3` | 2.51 | 3.08 | 16 | 9 |
| e2e timeline range + word | `quarterly after:… before:…` | 1.78 | 2.19 | 30 | 24 |

Completions are all under their 20 ms budget. Over the 50 ms budget:
`common word` 55.6, `typed at` 55.5, `e2e common word` 57.0 -- the same
three as step 9's table (52.3, 53.4, 55.2 there, at a lower load), within
11-14% of the budget.

## Where the common word's time goes

Measured with timers around each phase of `search_conversations` and the
walk statement re-run with one projection column at a time removed, on
the bench's corpus, the instrumentation not committed. The counting hooks
count statements and rows; they do not time them. Figures are medians,
load 4-9, which is why the total is a little above the table's p50.

| phase | ms |
|---|---:|
| **the walk** (`projection_sql` over `HITS_JOIN_WITH_FILES`, 10,001 rows until the fold holds `TOTAL_HITS_CAP`) | **52.8** |
| of which: the fold's own Rust per row (`Fold::take`) | 3.4 |
| grouping into conversations | 1.2 |
| ordering and ranking the pool | 0.2 |
| hydrating the pool and the page (250 rows) | 1.7 |
| facets over 1,674 conversations, files, people | 0.2 |
| searched / corpus complete / contents complete | 0.1 |
| **total** | **56.2** |

The facets *fold* is not the cost. What the facets need *from the walk* is.
The walk statement alone, 10,001 rows, with each per-row subquery removed
in turn:

| walk statement | ms | the column's share |
|---|---:|---:|
| as it is | 57.5 | |
| the hits and the join to `messages`, `m.id` only | 13.6 | |
| plus the message's own columns | 19.2 | 5.6 |
| plus people (`group_concat` over `recipients`) | | **19.3** |
| plus labels (`group_concat` over `message_labels`) | | 9.4 |
| plus the attachment count (`count(*)` over `attachments`) | | 7.2 |
| plus "has a live marker" (`markers`) | | 3.6 |

So two thirds of the walk is four correlated subqueries per matched
message, and people and labels are the two that exist only for the
facets. The plan is already a seek for each (`idx_recipients_message`,
the label key, `idx_attachments_message`, the marker's key); this is the
cost of 10,000 seeks of each, not of a bad plan.

## Options, not taken

None of these is implemented: the first two are schema work on the write
path, the last two change what a person sees, and all four want the
maintainer's choice.

1. **Maintained people and label lists per message.** Two columns on
   `messages` (the packed address ids the people facet reads, the label
   ids), kept by triggers on `recipients` and `message_labels` -- deferred
   during a sync batch the way `search_documents` is (`defer_documents`).
   The walk reads two columns instead of two subqueries: about -27 ms,
   common word ~30 ms. Nothing visible changes. Costs a storage migration
   and a trigger write per recipient and label row on the sync path, which
   wants `insert_cost_curve` measured before it lands (ADR 0040).
2. **The attachment count as a column.** `messages.has_attachments` is
   already maintained (the content projection's trigger sets it from
   `EXISTS (... attachments)`); a count beside it removes the third
   subquery: about -7 ms. Cheapest of the four; with 1, common word ~23 ms.
3. **Lower the walk's cap and show broad counts as floors.** The walk is
   linear in rows: a 5,000-message cap is roughly -26 ms. The total, the
   facets and the months of a word in most of the mailbox become "5,000+"
   sooner than they do now. Visible, so the maintainer's.
4. **Facets after the page.** Answer the page from the cheap walk (the
   19 ms row above, plus hydration) and send people and labels in a second
   reply. First paint ~22 ms; the facet column fills in ~35 ms later.
   Visible, and a second reply in the protocol.

Measured as a cheaper fifth: a covering index on
`recipients (message_id, kind, address_id)` makes the people subquery
index-only and took the walk from 56.9 to 52.5 ms -- not enough on its
own, and it adds an index entry to every recipient row.
