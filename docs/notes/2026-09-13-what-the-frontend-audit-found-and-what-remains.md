# What remains open from the frontend audit, and what was kept on purpose

2026-09-13. The audit of the frontend↔engine boundary and the simplification
pass that followed it moved frontend-agnostic decisions into `postio-ui`
(`paging::Paging`, `notify::decide`, `focus::next_pane` and `Returns`); that
work is in the code and its commits. This note is what was mapped and left,
so a later pass starts from the map rather than re-opening it.

## Still open

- **`[sync] notify_roles` does not cross the FFI**, so the macOS build
  notifies for every folder, not the configured ones
  (`crates/postio-ffi/src/notify.rs`). The notification *wording* is a
  per-platform choice on purpose (`postio_ui::notify::Wording`: counts on
  macOS, where the lock screen reads it out; the newest sender and subject on
  GNOME, which keeps banners off the lock screen).
- **The fts merge cost** (tantivy inside a committing transaction) is
  measured but untamed. The wave scheduler survives it
  (`2026-09-13-a-slow-pass-stops-every-folder-behind-it.md`), but a
  merge-laden pass is still a slow pass, and turso exposes no merge tuning.
- **Bulk test fixtures pay the fts index per document.**
  `total_hits_stops_counting_at_the_cap` inserts 10,050 messages one at a
  time and takes minutes, which is why it is in the nightly measurement tier.
  A bulk loader that seeds `messages` and `search_documents` in multi-row
  statements would take most of that back.

## Looked at, and kept — with the reason

- **`zbus`** (~35 crates) for one NetworkManager watcher
  (`postio-runtime/src/network.rs`): online/offline detection has no cheaper
  honest source.
- **`postio-body/src/styles.rs` over `cssparser`:** it complements `ammonia`,
  which filters attributes and does not parse CSS.
- **`MailboxRepository::{recount, recount_account, set_counts}`** have no
  production caller. They are the trigger-independent ground truth the count
  tests check against, and the repair tool for a store the triggers were not
  there for; gating them behind a test feature would gate a repair tool.
- **`RETAINED` in `postio-runtime`'s engine**, which keeps engines alive
  rather than dropping them abruptly: the engine is pre-1.0 and its recovery
  on an abrupt drop is not something to lean on.
- **`scripts/cc-wrapper.sh`** is in use: `install-shims.sh` installs it as
  `postio-cc`, the bare-name shim the shared compile cache depends on.
- **`blake3`** in storage and session: blob ids and the store-key derivation.
