# A scan wearing a seek's clothes (2026-09-21, #1587)

A first sync measured ~118 ms to write one message row — a 25-row write unit
held the lock ~3 s — and every existing reproduction came back flat. This is
how the cause was found, what it was, and the three rules about Turso's
planner this workspace now knows.

## The method, because the order mattered

1. **Walk the real path, not a model of it.** `examples/insert_cost_curve.rs`
   calls `commit_batch` itself — upsert, threading, triggers — from an empty
   store upward, printing ms/row per thousand. The three earlier examples
   (`commit_cost`, `fts_write_cost`, `fts_merge_stall`) each modelled one
   suspect and were each flat; the composite path was not: **1.5 → 17 ms/row
   over 30k messages with no fts index anywhere**, plus a ~constant
   9–10 ms/row when the `search_documents` fts index is installed.
2. **Bisect by schema, not by theory.** The same run twice, with and without
   the fts schema, split the cost into "a constant the index adds" and "a
   growth that is not the index at all". Both prior theories — the note of
   2026-09-13 blaming tantivy merges, and this session's trigger-multiplier
   arithmetic — pointed at the constant. The growth was the disease.
3. **Then ask the planner, with the real statement shapes.** Two of the
   first probe's "findings" were probe bugs (a missing `uid_validity` term,
   `address` for `address_normalized`). Probe SQL must be copied from the
   repository, not reconstructed from memory of it.

## The cause

`idx_thread_links_lookup` was `(account_id, rfc_message_id COLLATE NOCASE)`,
and **Turso's planner will not bind an equality through a collated index
column** — whatever collation the query itself asks for. Every plan came
back:

```
SEARCH thread_links USING INDEX idx_thread_links_lookup (account_id=?)
```

Seek to the account, then walk every link it has, filtering in the loop.
One walk per message threaded; one row added per message synced. Per-row
cost proportional to everything synced so far.

## Why the existing gate passed the whole time

`threading_lookup_cost` asserted `scans().is_empty()` — and a one-column
`SEARCH` is not a `SCAN` line. **A SEARCH that binds one column of a
two-column key is a scan wearing a seek's clothes**, invisible to any gate
that greps for `SCAN`. The gate now asserts the *bound columns* of every
per-message lookup, so the next planner surprise fails with the statement's
name on it. If a plan gate does not name the columns it expects bound, it
is not a gate.

## The three planner rules, in one place

Turso `0.8.0-pre.11`, as this workspace configures it:

1. **No reading through a partial index** (`7c441e4e`, the `_read` indexes).
2. **No binding through a collated index column** (this note). The cure is
   to move the fold into the key itself — `RfcMessageId::folded`, binary
   equality, plain index — never to add a second collated spelling.
3. **`IN` on a second index column gets the one-column prefix too.** A
   `UNION ALL` of point lookups is the shape that satisfies both this and
   the one-statement-per-chain line `threading_statement_count` holds:
   every arm plans as `(account_id=? AND rfc_message_id=?)`.

`idx_labels_account_name (account_id, name COLLATE NOCASE)` has rule 2's
shape today. It survives because an account holds dozens of labels, not
tens of thousands of links — if labels ever join a per-message hot path,
it is the same bug waiting.

## What is deliberately not fixed here

The fts index's ~9–10 ms/row is real, constant, and paid inside the write
transaction — 3–4 tantivy document writes per message (the message insert,
then one re-aggregate per recipient row). The cure has a precedent: bodies
had the same disease and moved to a background drain. Metadata should
follow, but ten-plus test files across three crates rely on
trigger-immediate indexing, so that is its own change with its own session
(#1587 carries the plan).

## The case-preservation boundary

`RfcMessageId` keeps original case in `as_str()` because a `References`
header we emit must quote the parent's id byte-for-byte. The folded form is
only the *lookup key*, and only in `thread_links`, which never reaches the
wire. Fold at a seam, not at the type's face — the wire is somebody else's
parser.
