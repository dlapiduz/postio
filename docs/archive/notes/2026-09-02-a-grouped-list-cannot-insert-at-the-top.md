# A grouped list cannot insert at the top (2026-09-02, #185)

`ListScope::reaction` decides what a list does when mail arrives, and until
the unified view every scope whose order guaranteed new mail belonged at the
top answered `InsertAtTop`. `ListScope::Unified` looks like it qualifies —
it is newest-first across every account, so a delivery is always the newest
thing in it — and it must not.

A unified row is a *conversation grouped across accounts*. Mail arriving at
the second address for a conversation already on screen folds into the row
that is already there: the row's `last_at` moves and it rises. An insert
cannot express that. It puts a second row on screen for the same
conversation, which is precisely what `unified_page`'s absorption exists to
prevent, and no later read repairs it because the inserted row is real — it
is a thread, and it is in the list.

**The rule this generalises to:** `InsertAtTop` is only sound when a row
stands for exactly one thing that arrival can create. Any view that folds
several stored rows into one displayed row — grouping, deduplication,
coalescing — has to reload, because the arrival may have changed an existing
row rather than added one, and only the query that built the grouping knows
which.

The same shape is why `unified_count` cannot be `count(*)` over threads, and
why it applies the page's own absorption predicate instead: a count and a
walk that disagree about what a row is give the list trailing placeholders
that never resolve.
