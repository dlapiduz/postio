# What recording a contact costs a synced message

Measured 2026-10-20 on `examples/insert_cost_curve.rs` (release, a Mac, other
sessions building, so read it to 10%), 10,000 messages from 50 senders, for
specs/011-focus-contacts S5. The example gained a third axis: after each
batch commits it records every message's correspondents, as sync does
(`ContactRepository::record_message`).

| rows | ms/row, fts, before S5 | + contacts, before S5 | ms/row, fts, after S5 | + contacts, after S5 | no fts, after S5 |
|---:|---:|---:|---:|---:|---:|
| 1,000 | 0.43 | 0.58 | 0.52 | 0.68 | 0.42 |
| 5,000 | 1.33 | 1.58 | 1.59 | 1.76 | 1.23 |
| 10,000 | 2.89 | 3.17 | 3.17 | 3.06 | 2.26 |

What S5 adds per recorded address is three statements that write
(a month count, a ring entry, a trim) and, per sighting, one more column. Two
conclusions:

- **Recording contacts costs about 0.15 ms a row at 1,000 rows, before and
  after S5**; the two curves sit within the run-to-run noise of each other at
  10,000. S5's writes are not visible beside the fts column, which is the
  ceiling (#1587): nothing here approaches it, and the curve's climb with
  store size is the existing one (it climbs without the fts index too).
- The trigger on `messages.seen` costs a statement per message whose read
  flag flips. It is not on this curve; a mark-all-read of N messages pays N
  of them, and that is the next thing to measure if marking a very large
  folder read ever shows.

Run it again after any change to what recording writes:

```text
CURVE_CEILING=10000 cargo run -p postio-sync --release --example insert_cost_curve
```
