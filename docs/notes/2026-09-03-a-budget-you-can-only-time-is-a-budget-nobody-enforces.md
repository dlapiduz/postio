# A budget you can only time is a budget nobody enforces (2026-09-03, #100)

`PRODUCT.md` §18 states three budgets — 500ms to a usable UI, 16ms per
interaction, 100ms for local search — and both it and `CLAUDE.md` said they
were enforced by benches in CI. They were not. `bench.yml` says so in its own
header: it compiles the bench targets and deliberately times nothing, because
a shared runner cannot defend a 16ms budget. That is the right call, and its
consequence was that a change making the message list read a whole mailbox
would have passed every gate in the project.

The fix is to stop measuring the budget and start measuring its *cause*. These
budgets hold because of the shape of the queries underneath them, and shape is
countable: statements issued, rows produced, trigger firings. A count is the
same number on a laptop and on a noisy runner, so it can gate a pull request in
a way wall-clock never safely can — and it can be *tight*, which is the part
that matters. The thread-paging test this replaced allowed a deep page ten
times the first page's duration plus 50ms, because that is the width a shared
machine forces on a timing assertion; the counted version holds it to double,
against measured figures of 246 and 252 rows.

`postio_storage::test_support::counting` is the machinery, over rusqlite's
`trace_v2` hook. Three things about it were learned the hard way and are not
guessable from the documentation:

- **SQLite reports nested invocations as SQL comments**, not as statement
  text. Counting them as ordinary statements is how a search of a common word
  appeared to cost 2,584 rows to show 25: 2,524 of those were FTS5's own
  b-tree segment lookups (`SELECT pgno FROM messages_fts_idx …`, 1,111 times).
  That figure tracks how the index happens to be segmented, so a budget over
  it would fail when SQLite merges segments and pass when the application
  started reading whole mailboxes.
- **A statement stack cannot be balanced.** `Profile` fires for the separately
  prepared statements a virtual table runs, but *not* for a trigger body, so
  pushing on `Stmt` and popping on `Profile` leaks: an index build over 2,000
  messages left 4,000 statements unclosed. Rows are attributed to whichever
  statement began most recently instead, which is exact wherever nothing nests
  and undercounts on a full-text path. That is the safe direction, and it is
  why the search budget counts statements rather than rows.
- **The three counts see different things, and none sees everything.** Rows
  cannot see an index build, because an `INSERT … SELECT` returns no rows.
  Statements cannot either, because it is a handful of them however much it
  rewrites. What it does produce is one trigger firing per row, which is why
  `Counts::nested` exists and is what the startup budget is expressed in.

The general lesson is the one `VmRSS` taught elsewhere in this file: pick the
number that reflects the invariant, not the number that is easy to read. And
having picked it, prove it moves — every one of these assertions carries a
control that exercises the failure it guards against, because a ceiling with
nothing underneath it is indistinguishable from a ceiling nothing can reach.
