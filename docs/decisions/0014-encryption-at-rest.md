# ADR 0014 — The local store encrypts itself

- **Status:** Accepted (2026-08-25); the mechanism is amended by [ADR 0038](0038-the-store-is-turso-not-sqlcipher.md) (2026-09-13): Turso's own AES-256-GCM page encryption replaces SQLCipher.
- **Decision:** Postio encrypts at rest itself. The database is encrypted by the engine, blobs by per-blob AEAD, with one master key in the Secret Service keyring and no plaintext fallback. Blob ids are keyed hashes.

## Rules
- Protected: a stolen or discarded disk, a wandering backup or sync copy, another user who defeats the `0700` permissions, and any read while the keyring is locked. Not protected, and the docs say so: an attacker running as the user with the keyring unlocked, root on a live system, RAM, swap and hibernation images. Full-disk encryption stays complementary.
- The database key is the raw 32-byte `Purpose::Database` subkey passed to `Store::open` (`CIPHER = "aes256gcm"`). A raw key, not a passphrase KDF: the keyring holds high-entropy material. `temp_store = MEMORY` keeps spill off disk.
- Encrypt bodies and the index together: the full-text index derives from message content.
- Each blob is XChaCha20-Poly1305 with a fresh 24-byte nonce, stored as a versioned header (`magic || nonce || ciphertext || tag`), 64 KiB streaming chunks. The version field lets an algorithm change without a flag day.
- Blob ids are BLAKE3 keyed by a store-specific subkey: dedup within the store survives, cross-store correlation and known-file confirmation do not. Nothing recomputes a blob name from content outside `BlobStore`.
- One 32-byte master key per store from the OS RNG, kept in the keyring via the `SecretStore` seam. Subkeys come from BLAKE3 `derive_key` (`"postio db"`, `"postio blob content"`, `"postio blob id"`).
- No plaintext fallback, ever. A locked keyring means the mail does not open (`SecretError::Locked` routes to the unlock-and-retry surface). The keyring entry is part of the mailbox: copied ciphertext without the key is useless.
- Tests run the real encrypted path with a fixed key.
- There is no migration: an existing store is rebuilt by resyncing. A future in-place migration drains the operation queue first and swaps last.
- Budgets (startup, interaction, search) do not move for encryption; `store_reads`, `search_budget` and the startup trace are the gate. The lever is `cache_size`.

## Why
- fscrypt needs per-machine setup and is unsupported on btrfs, so "encrypts at rest" would hold only on some filesystems. Hand-rolled page encryption repeats others' mistakes. SQLite SEE is proprietary.
- The store is a cache: only drafts and the operation queue are local truth.

## Revisit when
- A bench shows the engine cannot meet the 100 ms search budget on the 120k-message index after the cache lever. Then consider fscrypt where available.
