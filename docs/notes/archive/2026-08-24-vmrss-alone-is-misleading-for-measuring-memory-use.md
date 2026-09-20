# `VmRSS` alone is misleading for measuring Postio's memory use

*Archived 2026-09-14: the explanation rests on `PRAGMA mmap_size`, which the Turso engine (ADR 0038) does not have; the habit of splitting `RssAnon` from `RssFile` before believing a resident-set figure is kept in the testing section of `docs/engineering-notes.md`.*

Measured: total resident set is 131 MiB on a 1,000-message store and 215 MiB on a
100,000-message one, which reads exactly like the mailbox being loaded — the
one thing PRODUCT.md §18 promises never happens. Split
`/proc/<pid>/status` instead: `RssAnon` is 47 MiB at *both* sizes (what
Postio itself allocates — the windowed list model, the widgets, the
runtime), and the entire difference is `RssFile`, because `postio-storage`
sets `PRAGMA mmap_size = 256 MiB` and SQLite maps as much of the store as it
touches. Those are reclaimable page-cache pages, not mail being held. Anyone
re-measuring must split anon from file, or raise `mmap_size` as a suspect
before the list model. Reproduce with
`crates/postio-runtime/examples/seed_store.rs` and the release binary; the
README carries the table.
