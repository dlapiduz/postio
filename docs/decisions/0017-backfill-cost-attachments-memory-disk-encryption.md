# ADR 0017 — What "download everything" costs, and the four axes that pay for it

- **Status:** Accepted (2026-08-26); amended 2026-10-14 (new mail's attachments fetch on arrival). Page compression, contentless FTS and the SQLCipher premises are gone with the engine ([ADR 0038](0038-the-store-is-turso-not-sqlcipher.md)); bodies live in rows ([ADR 0020](0020-where-message-bodies-live.md)).
- **Decision:** backfill splits into a text axis and a payload axis. The text axis is [ADR 0016](0016-full-mailbox-backfill-by-default.md)'s "every message's body": headers and every `text/*` part, to completion, by default. The payload axis (attachment bytes) is governed separately and defaults to on demand.

## Rules
- **Reference measurement:** 81,744 messages, 12.43 GB, of which 11.0 GB (88.5%) is attachment payloads and 1.43 GB headers and text. Text is the search and offline corpus.
- **Text axis.** For every message in every non-excluded folder: fetch headers and `text/*` parts, store `body_text` and `body_html`, index the text. Bodies are eager; only attachments are lazy. Inline parts under `max_inline_bytes` (256 KiB) ride with the text; larger ones are payloads.
- **Payload axis.** `AttachmentPolicy` is `on_open` (default), `eager` or `never`. `on_open` fetches by `part_id` (`BODY.PEEK[<section>]`) into `attachments.blob_id`. `never` means filename search only.
- **New mail is the exception.** What an incremental pass announces as new fetches its attachments as it arrives, on the fresh lane, text first, up to `BackfillPolicy::fresh_payload_bytes` (25 MB a message). `never`, excluded folders, background-off and metered connections still hold. A first-sync backlog stays on demand.
- **`body_state`:** `partial` means text local, payloads not; `full` means every part is local. Eviction sets it back to `partial`.
- **Raw source** (`messages.raw_blob_id`) is an evictable cache entry, not written unconditionally. View source, forward-as-`message/rfc822` and signature verification refetch it.
- **Memory.** No message byte is resident except the text being parsed. A `BlobSink` streams payloads socket to temp file to rename in 64 KiB chunks; the interactive lane ignores `max_body_bytes` by design, so it must stream too. The backfill heap is capped and mailbox paths interned.
- **Blobs.** Pipeline: `id = BLAKE3_keyed(plaintext)`, then compress, then encrypt, never the reverse. Skip compression by MIME type and a probe over the first 128 KiB, and record it in the header's format byte. Bodies compress per row, zstd level 3, no dictionary (`body_codec.rs`), about 1.6x. No page compression.
- **Dictionaries** are not built: a lost dictionary makes blobs unreadable. Build one only if a real account's payload-to-text ratio is below about 3:1.
- **Partial indexes.** A `WHERE` on an index in `schema.rs` must be a constraint, never a size optimisation; the planner will not read a partial index.
- **Budget.** `[storage] max_bytes` evicts refetchable blobs (raw source, then attachment payloads, least recently used), never text bodies, drafts or the queue. Eviction needs no secure erase.
- **Encryption.** The local store is a complete replica, so at-rest encryption is required (ADR 0014). The `search_budget` bench is baselined against a fully backfilled store, not a seeded fixture.
- First run states the download size (from `BODYSTRUCTURE`) before spending it. `pause_on_metered` stays.

## Why
- Before this, backfill fetched every byte of every message, twice, through the heap.
- Attachments are 88.5% of bytes and incompressible, so compressing rows buys little; the content-addressed blob store gives streaming and dedup.

## Revisit when
- A text-heavy real account (payload-to-text below 3:1) is measured: a dictionary would save up to a quarter of its store.
