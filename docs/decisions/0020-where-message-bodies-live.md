# ADR 0020 — Message bodies live in SQLite; the blob store keeps attachments

- **Status:** Accepted (2026-08-27); amended 2026-09-14 ([ADR 0038](0038-the-store-is-turso-not-sqlcipher.md)) and 2026-10-05 ([ADR 0046](0046-content-identity-is-distinct-from-mailbox-identity.md)).
- **Decision:** text bodies and headers live in database rows, compressed per value with zstd in Rust (`crates/postio-storage/src/body_codec.rs`, level 3, no dictionary, only where the frame is smaller than the text). Attachment payloads and raw `.eml` stay in the content-addressed blob store.

## Rules
- Decoded bodies and header blocks belong to `message_contents`; derived body and header rows are keyed by content. Mailbox occurrence ids still target location-specific operations. Sharing content requires an account-scoped immutable backend identity, never an RFC Message-ID.
- The body index is a separate folded table (`message_search_bodies`), because the engine's index cannot tokenise compressed bytes and its tokenizer does not strip diacritics.
- Compress in Rust, above the pager; page encryption sits below. Order is compress, then encrypt. No page compression, no loadable extension (`sqlite-zstd` is unavailable, and `load_extension` is an attack surface).
- The blob store keeps what is streamed, deduplicated and mostly incompressible: attachments and raw source, encrypted per blob (ADR 0014), id = keyed hash of the plaintext. `BlobStore::get` serves whole values, `reader` serves attachments.
- `drafts.body_text` stays inline.
- The schema has no migrations: it is one `HEAD` constant in `crates/postio-storage/src/schema.rs`, and an old store is rebuilt by resyncing. This is a one-installation licence, not a policy. When a release has a second installation, numbered forward-only migrations return.

## Why
- Default stores hold almost only text (payloads are on demand, ADR 0017), so body compression is the largest disk lever: 1.43 GB becomes about 0.91 GB per value (1.57x).
- Median body is 325 bytes; a file with a block and an inode is a poor container. Dedup pays on attachments, not bodies.
- Rows inside one encrypted file close the metadata leak a file per body keeps (count, length, mtime), with one encryption mechanism and one key.
- Alternatives rejected: ZIPVFS plus SEE ($4,000, proprietary), CEVFS (no WAL).

## Revisit when
- A text-heavy account measurement shows a trained dictionary is worth its permanent-artifact risk.
