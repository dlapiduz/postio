# An aggregate hides from a row count (2026-09-11, #1479)

Startup on a real store (223 MB, ~81k messages) measured 1249.7 ms against
a 500 ms budget, and 1044.1 ms of it was the first frame. Nothing was
waiting on the network — `start_syncing` runs *after* the frame and the log
proves it still does — so the whole of that stretch was local work between
`opening account` and the paint.

The cause was one statement:

```sql
SELECT count(*) FROM messages
 WHERE account_id = ?1 AND read_receipt_requested = 1 AND deleted_locally = 0
```

`settings_privacy::install` runs inside `feed_the_window`, on the thread
that has to draw, for a figure shown in a panel that may never be opened.
There is no index on `read_receipt_requested`, so it is a scan of every
message the account holds.

## The part worth writing down

**Two counted budgets were already in the workspace and neither could see
it.** `postio_storage::test_support::counting` counted statements and rows,
and an aggregate defeats both by construction: it is *one* statement, and it
produces *one* row however many it had to read to produce it. Counted over
a seeded store at two sizes an order of magnitude apart, opening a window
measured

| | 1,000 messages | 10,000 messages |
|---|---:|---:|
| statements | 37 | 37 |
| rows | 28 | 28 |
| **steps** | **8,144** | **71,144** |

The first two rows are what a reviewer would have read as proof the path
was bounded. They are identical because they are blind, not because it was.

So: **a row count answers "was a mailbox loaded into memory", and only
that.** It cannot answer "was a mailbox *examined*", and those are different
questions with different bugs behind them. `steps`
(`SQLITE_STMTSTATUS_VM_STEP`, read off each statement as it finishes) is the
count that moves with rows examined. It is machine-independent for the same
data and plan, so it gates a pull request the way the other two do.

Related, and the same trap from the other side: #746's note records that
wrapping a statement in `SELECT count(*) FROM (…)` to time it *un*-measures
it, because SQLite prunes subquery columns nothing reads. An aggregate is
cheap to *report* and arbitrarily expensive to *compute*, and both of these
are that sentence biting.

## And the shape the bug keeps taking

This is the second time a settings panel's own figure has been read on the
startup path. #871 found `MessageRepository::footprint` — `count(*)` and
`sum(size)` over every message an account has — costing 1.48 s there, and
moved it behind the panel's visibility. #1479 is the pane next door, the
same shape, found by measurement again rather than by the fix generalising.

**What a panel draws is read when the panel is looked at.** `install` runs
during composition and the panel is not on screen then; every one of these
surfaces already refreshes on its own `map`, so the read costs nothing to
defer and everything to leave where it is.

The gate is `app_suite`'s `startup_reads` case: pointing a window at a
seeded store, counted, at two sizes, asserting the same numbers from both.
It counts the main thread and nothing else — which is the claim rather than
a limitation, since everything `feed_the_window` hands to the runtime is off
that thread by construction and delays nothing on screen.
