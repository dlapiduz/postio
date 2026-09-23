# The engine keeps no pool, so a connection is a cold cache

*2026-09-23, from the felt-speed and RAM review (#1602).*

## What was believed

`store.rs` said, from the engine swap on: "the engine keeps its own pool
behind `turso::Database::connect`, which is why that call is cheap." The
hand-written pool was deleted on that belief, and every read since --
`LocalStore::read`, the search `ask`, the sidebar refresh, every sync lane,
every body fetch, every indexer batch -- opened a connection of its own.

## What the engine does

`turso_core::Database::connect` (0.8.0-pre.11, `database.rs`) runs `_init`
and builds `Pager::new(.., PageCache::default(), ..)` per connection. The
`_shared_page_cache` field on `Database` is read only by `cache_info`. The
SDK's `Database::connect` goes straight to it. So:

- **Every connection is a cold cache** over an encrypted file: the pages a
  query touches are read and decrypted again, and `PRAGMA cache_size =
  -65536` caps what *that* connection may hold, until it drops.
- **The cap is per connection.** A sync wave held one checkout per lane for
  the length of the folder plus one for settling, each allowed 64 MiB as the
  lane wrote across the file: up to ~300 MB of a 917 MB process, and the
  growth watched live (591 -> 640 -> 671 MB of heap while two large folders
  synced) is consistent with it. An upper bound, not a measurement: the
  pager clears a connection's whole cache whenever a read follows another
  connection's commit (`storage/pager.rs`, `begin_read_tx`, "default SQLite
  behavior"), so two lanes committing in turn empty each other's caches, and
  only a lane writing alone fills its cap. The #1503 nightly is what turns
  the bound into a number.
- **A list page opened two**, paying ten `PRAGMA` statements the counting
  seam never saw (`execute_batch` bypasses it) and two cold caches per page.

## What the store does now

- `Store::read` keeps `READERS` (three) connections warm and hands out
  turns; a fourth caller waits rather than opening a cache nothing else
  will hit. `LocalStore::read` and the search `ask` take turns. The same
  invalidation means the warm cache pays between syncs -- scrolling and
  opening on an idle store -- and what every read saves during one is the
  pager, the header read, the schema clone and the five pragmas.
- `Store::connect_background` sets `cache_size = -4096`; the engine's own
  connections, the indexer's batches and the session's housekeeping take
  it. A write-through lane gains nothing from 64 MiB of clean pages.
- `Store::connect` is for a writer, which wants a connection of its own.
- `test_support::counting::checkouts()` counts connections process-wide,
  and `paging_a_folder_opens_one_connection_rather_than_one_per_page`
  holds the list to one.

## The rule

**A read that comes in numbers takes a turn on a warm reader; a writer
takes a connection; background work takes a small cache.** And a doc
comment that says what a dependency does is a claim to check against the
dependency's source, not a fact to build on -- this one cost a hand-written
pool, ~300 MB, and a cold cache per page for eleven days.
