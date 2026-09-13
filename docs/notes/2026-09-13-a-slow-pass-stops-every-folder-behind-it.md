# A slow sync pass stops every folder behind it

2026-09-13, from a live run against a real IMAP account.

## What was seen

```
folders known locally, queued for sync known=15 queued=15
sync{mailbox=1 path="INBOX"}: sync started
sync{mailbox=8 path="Drafts"}: sync started
sync{mailbox=1 path="INBOX"}: sync finished inserted=0 updated=2
```

…and then nothing, for the rest of the run. Fifteen folders queued, two
started, one finished. **Fifty-nine thousand messages of `Archive` were never
attempted**, and the store held 22,564 of the account's ~82,000 messages with
`Archive` and `Junk` both showing no `last_full_sync_at` at all.

Two separate faults compose here, and both are worth writing down because
neither is fixed.

## 1. The pass was slow, and the time was inside tantivy

A stack sample of the live process (`eu-stack -p`) put the sync's own thread
here:

```
engine::sync_pass -> resync::resync_mailbox -> initial::enumerate
  -> initial::commit_batch -> turso::Connection::execute -> Statement::step
  -> vdbe::op_auto_commit -> Program::commit_txn -> Pager::commit_tx
  -> turso_core::index_method::fts::directory
  -> tantivy::indexer::merger::IndexMerger::write
```

Eighteen of eighteen samples were inside `tantivy`; fifteen mentioned `fts`.
The folder was **Drafts, with twenty-five messages** — it was not paying for
its own indexing. Every insert into `messages` fires
`trg_search_documents_messages_ai`, which writes a row into
`search_documents`, which carries a five-column tantivy index; a merge runs
inside whichever transaction commits next, so a small write can inherit one
the body backfill's thousands of documents made due.

**This did not reproduce synthetically.** `examples/fts_merge_stall.rs` builds
22,000 messages and indexes 2,600 bodies of ~31 KiB — the live store's exact
shape — and the worst single insert against that index is **1.0 ms**.
`examples/fts_write_cost.rs` shows the body index adds roughly 30% to a header
insert and no merge cliff. `examples/commit_cost.rs` rules out the other
obvious explanation: commit cost is flat in store size, 0.11 ms at 4 KiB and
0.29 ms at 13 MiB, so nothing is checkpointing the database per transaction.

What the reproductions do not have is **concurrency**: live, the body backfill
was writing the same tantivy index while the sync committed. That is the
remaining difference and the place to look next.

## 2. One slow pass holds every folder still queued

`sync_wave` takes `sync_lanes(MAX_CONCURRENT_PASSES)` = **2** mailboxes, runs
both, and does not return until *both* finish. The outer loop only starts
another wave once it does. So a pass that runs long holds every mailbox left
in `state.to_sync`, however many lanes are free — which is how one
twenty-five-message folder kept fourteen others off the disk.

### The obvious fix does not work as written

Refilling a lane the moment it comes free — take the next mailbox from
`to_sync` and push another pass — was implemented and **reverted**. It works
(instrumenting the refill shows the queue draining 8 → 1 with lanes
alternating) and it breaks
`sync_wave::a_job_is_served_without_waiting_out_the_wave_it_arrived_during`.

The reason is structural. The wave's `select!` is `biased`, completions first,
so a wave that keeps feeding itself keeps the first branch ready and the
`interruption` arm — the one that yields to a user's job — never gets a turn.
Checking for a waiting job at each completion boundary instead was not enough
either: the refilled passes keep the wave busy long enough that the archive
finishes before the wave can return, and the job is answered only after it.

#944 and #122 are the history of that guarantee. Whatever fixes the stall has
to keep it: **a wave is background work and the user must never queue behind
it.** A refill and that promise can probably coexist, but not by adding the
refill alone.
