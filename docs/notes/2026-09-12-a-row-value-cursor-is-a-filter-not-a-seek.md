# A row-value cursor is a filter, not a seek

2026-09-12, `feature/turso-store`.

## The finding

SQLite turns `(a, b) < (?1, ?2)` into a single range constraint and seeks the
index to it. **Turso does not** — it seeks on whatever else the `WHERE` names
and then filters, row by row, over everything above the cursor.

Measured on `messages`, `idx_messages_list (mailbox_id, received_at DESC, id
DESC, …)`, a mailbox of 100,000:

```text
SELECT … WHERE mailbox_id = ?1 AND (received_at, id) < (?2, ?3) …
  SEARCH messages USING COVERING INDEX idx_messages_list (mailbox_id=?)

SELECT … WHERE mailbox_id = ?1 AND received_at <= ?2
                               AND (received_at < ?2 OR id < ?3) …
  SEARCH messages USING COVERING INDEX idx_messages_list (mailbox_id=? AND received_at>=?)
```

The `OR` spelling alone plans like the row value. What buys the range is the
**redundant** `received_at <= ?2` in front of it: a bare inequality on the sort
column, which the planner can turn into a bound, with the tie-break behind it
where it costs nothing.

## What it cost

`messages::paging_stays_flat_over_a_hundred_thousand_messages` walks to page
1,900 and times a window there. Before: first page 1.4 ms, deep page **107
ms**, and the whole test timed out at 240 s. After: the same test passes in
42 s and the deep page is inside the budget. That is the difference between
keyset paging and `OFFSET`, arrived at without anybody writing `OFFSET`.

Four cursors had the row-value spelling — the message list, the account and
unified thread lists, and the folder thread window — so every list in the
application was a skip as soon as the user scrolled.

## The rule this leaves

**Give the planner a bare inequality on the sort column, even when a row value
already says it.** The row value is the better-reading spelling and it is not
enough here; the redundancy is doing real work and a tidy-up that removes it
puts the walk back.

## What notices

`messages::a_cursor_page_seeks_past_the_cursor_instead_of_filtering_down_to_it`
asserts on the plan, in a tenth of a second, and says which of the two things
went wrong. It exists because the clock version — 100,000 seeded rows, two
minutes — is too slow to be the thing that catches this, and because a timing
assertion on a shared machine can only ever say *slower*, not *why*.
