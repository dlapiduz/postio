# Performance: budgets and the measured baseline

Performance is a functional requirement in Postio, enforced by `cargo bench`
rather than checked by hand at the end:

| Budget | Target | Measured |
|---|---|---|
| Startup to usable UI (populated DB) | < 500 ms | **147 ms** |
| Ordinary UI interaction | < 16 ms | **~2 ms** |
| Local search | < 100 ms | **27 ms** |
| Memory, 100,000 messages | no full-mailbox load | **47 MiB**, flat |

Transitions are ≤ 100 ms or absent entirely, and `prefers-reduced-motion` is
always honored. A mailbox is never loaded into memory in full — the message
list is windowed over paged SQLite.

## The baseline

Measured on one developer machine, which makes the numbers a regression guard
rather than a promise about anybody else's hardware. Reproduce them:

```sh
cargo run -p postio-runtime --example seed_store -- /tmp/postio.db 20000
POSTIO_STORE=/tmp/postio.db POSTIO_STARTUP_TRACE=1 POSTIO_STARTUP_EXIT=1 \
  cargo run --release -p postio-app

cargo bench -p postio-runtime --bench store_reads   # the database read
cargo bench -p postio-gtk     --bench list_scroll   # the row draw
cargo bench -p postio-search  --bench search_budget --features index
```

Startup, on a 20,000-message store with an account and six folders: 147 ms,
of which 47 ms is `adw::init` and 82 ms is the first frame. The window, the
styles and the fonts are 18 ms between them.

An ordinary interaction is a scroll, and a scroll is two things — a page read
and a screenful of rows drawn. Together they are the ~2 ms above:

| | 1,000 messages | 100,000 messages |
|---|---|---|
| Page read, top of the folder | 139 µs | 219 µs |
| Page read, scrolled to the middle | — | 176 µs |
| Page read, *jumped* to the middle | — | 28 ms |
| Row draw, one screenful | 1.6 ms | 1.6 ms |

Flat against mailbox size, which is the claim that matters: reading page one
of a hundred thousand messages costs what reading page one of a thousand
costs. The exception is a *jump* to a page nobody has scrolled through — the
store has no boundary to seek from and falls back to walking — which happens
once per jump, and every page after it is the 176 µs row.

Search, over a 120,000-message index, by query shape:

| Query shape | Measured |
|---|---|
| Composed — an operator plus free text, what the search bar usually produces | 0.45 ms |
| Simple term — a word matching about 1% of the corpus | 7.0 ms |
| Operator only — `from:`, no free text and no FTS join | 12.4 ms |
| Common word — a word in every message, the worst case | 26.4 ms |
| Common word, with facet counts | 27.5 ms |

The worst shape is the one to watch: `MATCH` and the `count(*)` behind it have
to walk effectively the whole corpus, and it is where a missing index would
show up first.

## Memory, and the claim it tests

A mailbox is never loaded into memory in full. Measured rather than asserted,
by sampling `/proc/<pid>/status` while the application is open on a store of
each size:

| | 1,000 messages | 100,000 messages |
|---|---|---|
| Anonymous — the application's own heap | 47 MiB | 47 MiB |
| File-backed — mapped store, WAL, shared libraries | 83 MiB | 167 MiB |
| Resident total | 131 MiB | 215 MiB |

**The anonymous figure is the claim.** It is what Postio itself allocates —
the windowed list model, the widgets, the runtime — and it does not move
between a thousand messages and a hundred thousand. The file-backed half grows
because `PRAGMA mmap_size` is 256 MiB and SQLite maps as much of the store as
it touches; those are reclaimable page-cache pages, not mail the application
is holding on to.

Reproduce it:

```sh
cargo run --release -p postio-runtime --example seed_store -- /tmp/big.db 100000
POSTIO_STORE=/tmp/big.db cargo run --release -p postio-app &
grep -E '^(VmRSS|RssAnon|RssFile):' /proc/$!/status
```
