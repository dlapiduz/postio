# ADR 0038 — The store is Turso, and ADR 0014 keeps its threat model

- **Status:** Accepted (2026-09-13)
- **Date:** 2026-09-13
- **Amends:** [ADR 0014](0014-encryption-at-rest.md) — the
  *mechanism* only. Everything 0014 decided about what encryption must
  protect, what it must not claim, and where the key lives is unchanged.
- **Feature:** `specs/004-turso-store`
- **Decision:** **The database is [Turso](https://github.com/tursodatabase/turso),
  the Rust rewrite of SQLite, with its own AES-256-GCM page encryption keyed
  from the same master key.** SQLCipher and `rusqlite` leave the graph, and
  `openssl-src` with them. Blob encryption is untouched. There is **no
  migration**: a store the old engine wrote cannot be read by this one and is
  rebuilt by resyncing.

---

## What this does not change

ADR 0014's threat model, verbatim in force:

* Protected: a stolen or discarded disk, a wandering backup, another user on a
  multi-user machine, any reading of the files while the keyring is locked.
* Not protected: a running process, a compromised account, a keyring that is
  unlocked.
* One master key, in the Secret Service keyring, with **no plaintext
  fallback** — a keyring that will not answer is a refusal, never a fresh key.
* The database subkey, the blob-content key and the blob-id key are still
  BLAKE3-derived from that one master (Q3), so three purposes stay
  cryptographically separated behind one keyring entry.
* `temp_store = MEMORY`, because an encrypted database whose sort scratch
  lands on disk in the clear has encrypted the wrong thing. This engine
  happens to default it correctly; it is set and asserted anyway.
* Blobs are XChaCha20-Poly1305 with keyed BLAKE3 ids (Q2). Nothing here
  touches them.

## Why the engine changed at all

Not for the cipher. `crates/postio-storage/tests/hmac_cost.rs` — deleted with
its subject — priced SQLCipher 4's per-page MAC on a real 957 MiB mailbox:
**45.9%** of sampled CPU in `sha512_block_data_order_avx2` against **2.7%** in
`aesni_cbc_encrypt`. On this class of CPU the SHA extensions accelerate SHA-1
and SHA-256 and not SHA-512, so the one part of the page path left in plain
software was the part that dominated. AES-256-GCM authenticates as part of the
cipher rather than beside it, and that whole term goes.

And `openssl-src` leaves the dependency graph: a 28-second uncacheable build
step per cold worktree, plus the `atexit` shutdown workaround #794 and #699
both needed.

## What it costs, stated rather than discovered

`docs/notes/2026-09-13-what-the-engine-swap-could-not-keep.md` is the full
list, each item with the test that pins it. The three that bear on **this
ADR**:

1. **Both the encryption and the full-text search are marked experimental
   upstream, and neither has been audited.** SQLCipher was neither. This is
   the real cost of the swap and it is accepted deliberately: Postio is
   pre-1.0 with one user, `spec.md` records the acceptance, and it is why
   there is no migration — a store is rebuilt from the server rather than
   converted, so a defect in the new engine's encryption costs a resync and
   not a mailbox.

2. **`PRAGMA auto_vacuum` cannot be enabled at all.** #381 chose
   `INCREMENTAL` so a mailbox that loses ten thousand messages hands the pages
   back a few at a time; the engine puts autovacuum behind a flag its Rust
   builder does not expose, and offers no `incremental_vacuum` step. Only a
   full `VACUUM` remains, which rewrites the database and blocks every writer
   while it does. It is therefore gated on a policy rather than stepped
   (`Store::is_worth_reclaiming`), and the cost is a one-off shrink deferred,
   not unbounded growth: freed pages *are* reused, so a store plateaus at its
   high-water mark. `reclaim_pages.rs` proves the application reaches the
   gate, and fails the day the incremental mode arrives.

3. **There is no read-only open.** `SQLITE_OPEN_READ_ONLY` and `PRAGMA
   query_only` have no equivalent, so the four diagnostics under `examples/`
   that opened the live store read-only now say *point this at a copy* and
   mean it.

## The one thing 0014 said that is now false

> the pre-release migration path is drain-and-reencrypt

There is no migration path. 0014's was from *plaintext* to encrypted and ran
inside one engine. This is a different file format: the old store cannot be
opened, so there is nothing to drain. The maintainer's instruction was explicit
("we can blow the current store"), and `postio_session::open_store_at` carries
a comment where the migration call used to be.

> **Amended 2026-09-14 (specs/004-turso-store):** the note this ADR wrote
> into ADR 0020's status line — "bodies are plain `TEXT`" — was true for as
> long as the body index sat on the body column itself. Once the index moved
> to its own folded table, `message_search_bodies`, the body column was free
> to be small again, and `crates/postio-storage/src/body_codec.rs` restored
> per-row zstd (level 3, no trained dictionary — `body_dictionaries` stays
> gone), storing the frame only where it is smaller than the text. ADR 0020's
> status line carries the same amendment.
