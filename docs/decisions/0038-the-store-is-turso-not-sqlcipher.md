# ADR 0038 — The store is Turso, and ADR 0014 keeps its threat model

- **Status:** Accepted (2026-09-14). Amends ADR 0014's mechanism only.
- **Decision:** The database is Turso, the Rust rewrite of SQLite, with its own AES-256-GCM page encryption keyed from the same master key. SQLCipher, `rusqlite` and `openssl-src` leave the graph. There is no migration: a store the old engine wrote is rebuilt by resyncing.

## Rules
- ADR 0014's threat model is in force. Protected: a stolen disk, backups, another user, any read while the keyring is locked. Not protected: a running process, a compromised account, an unlocked keyring.
- One master key in the Secret Service keyring, no plaintext fallback. A keyring that will not answer is a refusal, never a fresh key.
- The database, blob-content and blob-id keys are BLAKE3-derived from the master (ADR 0014 Q3).
- `temp_store = MEMORY` is set and asserted.
- Blobs stay XChaCha20-Poly1305 with keyed BLAKE3 ids.
- `PRAGMA auto_vacuum` cannot be enabled. Only a full `VACUUM` remains (it blocks writers), gated by `Store::is_worth_reclaiming`. Freed pages are reused, so the store plateaus. `reclaim_pages.rs` fails the day the incremental mode arrives.
- There is no read-only open. Diagnostics under `examples/` run against a copy.
- Bodies are stored as per-row zstd (level 3, no trained dictionary) only where the frame is smaller (`postio-storage/src/body_codec.rs`). The body index sits in its own folded table, `message_search_bodies`.
- Store schema is `HEAD` in `crates/postio-storage/src/schema.rs`. There are no migrations.
- The full list of what the swap lost, with the pinning test for each, is `docs/notes/2026-09-13-what-the-engine-swap-could-not-keep.md`.

## Why
- SQLCipher 4's per-page SHA-512 MAC took 45.9% of CPU against 2.7% for AES on a real 957 MiB mailbox. GCM authenticates in the cipher.
- `openssl-src` cost a 28 s uncacheable build per cold worktree plus an `atexit` shutdown workaround.
- Cost accepted: Turso's encryption and full-text search are experimental upstream and unaudited. Postio is pre-1.0 with one user, so a defect costs a resync, not a mailbox.

## Revisit when
- Upstream stabilises or audits encryption and full-text search.
- Incremental vacuum or a read-only open arrives.
