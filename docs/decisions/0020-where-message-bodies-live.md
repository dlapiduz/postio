# ADR 0020 — Message bodies live in SQLite; the blob store keeps attachments

- **Status:** Accepted (2026-08-27)
- **Date:** 2026-08-27
- **Decision by:** the maintainer, asking two questions in sequence — *"are all
  the bodies just out there in the open?"* and *"why not store the bodies
  inside SQLite?"* — neither of which the existing ADRs answered.
- **Issue:** [#399](https://github.com/dlapiduz/postio/issues/399)
- **Supersedes:** ADR 0017's amendment of the same day, which deferred the
  compression dictionary on an arithmetic error. See *The correction* below.
- **Related:** ADR 0014 (encryption at rest), ADR 0016 (full backfill),
  ADR 0017 (backfill cost), [#300](https://github.com/dlapiduz/postio/issues/300)
  (SQLCipher), [#301](https://github.com/dlapiduz/postio/issues/301) (blob AEAD),
  [#78](https://github.com/dlapiduz/postio/issues/78) (a real-account
  measurement)
- **Decision:** **text bodies and headers move into SQLite rows, compressed
  per value with zstd against a trained dictionary stored in a sibling table.
  Attachment payloads and raw `.eml` stay in the content-addressed blob
  store.** Compression is done in Rust rather than by a SQLite extension. No
  page compression, which ADR 0017 already settled and this does not disturb.

---

## The correction this rests on

ADR 0017 measured a 12.43 GB reference mailbox: 11.00 GB (88.5%) attachment
payloads, 1.43 GB (11.5%) headers and `text/*`. On the strength of those
figures I amended it earlier the same day to defer the compression dictionary,
computing its value as **2.2% of the store**.

That number is wrong, and the way it is wrong is worth recording because it is
easy to repeat. **ADR 0017's own decision line says the payload axis "defaults
to on-demand".** The 11 GB is not on disk unless the user asks for it. So the
default store is essentially the text axis alone, and the denominator I divided
by is the one the product deliberately avoids:

| | size | share of the default store |
|---|---:|---|
| text axis, uncompressed | 1.43 GB | — |
| per-value zstd (1.57x) | 0.91 GB | **compression saves 36%** |
| with a trained dictionary (2.19x) | 0.65 GB | **a further 28%** |

The earlier amendment even contains the right row — *"0.15 GB payloads →
24.3%"* — labelled as the mailing-list edge case. It is not an edge case. It is
the default configuration.

**Compression of bodies is the single largest disk lever in the product**, and
the dictionary is worth roughly a further quarter on top. Neither conclusion
survives being averaged against attachments nobody downloaded.

## Why bodies belong in rows

The case for the blob store is real and it is entirely a case about
**attachments**. Applied to bodies it does not hold, and the earlier amendment
let it carry them along:

- **Bodies are already read whole.** `BlobStore::get`'s own doc says it is
  "for a body or a header block, which is what the reading pane wants", with
  `reader` reserved "for anything that might be an attachment". The streaming
  argument is an argument about attachments.
- **Dedup is worth nothing on bodies and everything on attachments.**
  Identical message bodies are rare — a quoted reply resembles its parent, it
  is not byte-equal — while the same PDF genuinely arrives five times.
- **Median body: 325 bytes.** A file each, with a 4 KiB block and an inode, is
  a poor container for a value that small.
- **There is precedent in the schema.** `drafts.body_text` is stored inline
  already, deliberately, and the module says why.

And two things rows buy that files cannot:

- **SQLCipher covers them for free** (#300). The most sensitive bytes in the
  product need no second encryption mechanism, no second key, no second
  correct implementation of an AEAD.
- **The metadata leak closes.** A file per body leaks its count, its ciphertext
  length and its mtime *even when the contents are encrypted* — message sizes
  are a real fingerprint, and mtimes trace when mail arrived and was read. Rows
  inside one SQLCipher file leak none of it. This is what prompted the
  question, and it is the strongest argument here.

**And it dissolves the dictionary hazard.** The earlier amendment deferred the
dictionary because it is "a new way to lose mail": a separate artifact that,
if lost, takes every blob written against it. A dictionary stored as a row in
the same database is backed up, encrypted, and restored with the data it
decodes. It cannot go missing independently, and the failure mode disappears
rather than being managed.

## Why we compress in Rust rather than with an extension

Researched rather than assumed, because *"use a library"* was the right
instinct and deserved a real answer.

| | what it does | verdict |
|---|---|---|
| **ZIPVFS + SEE** | page compression + encryption, both official | SQLite says combining them is "pointless" — SEE encrypts first, leaving nothing compressible. The documented route is *one callback that compresses and encrypts*. **$4,000** perpetual for ZIPVFS, SEE priced separately. |
| **CEVFS** | both, at the pager level, MIT | **No WAL.** Postio runs WAL and depends on it. Also incomplete free-space handling and no VACUUM. Disqualified. |
| **SQLite3MultipleCiphers** | encryption only, ChaCha20-Poly1305 | No compression, but a serious candidate for #300 — actively maintained, and its recommended cipher carries tamper detection, unlike SQLCipher's AES-CBC. |
| **`sqlite-zstd`** | transparent row-level zstd **with automatic dictionary training** | Exactly this decision, as a maintained library. Blocked on integration — see below. |

`sqlite-zstd` is the right shape and cannot be linked. Measured, not assumed:
its latest release wants `libsqlite3-sys ^0.33`, Postio's rusqlite 0.40 wants
`^0.38`, and cargo's `links = "sqlite3"` rule permits exactly one. The
resolver refuses outright.

The remaining route is a loadable `.so`, which means shipping a native artifact
per platform and enabling `load_extension` — an attack surface a mail client
should think hard about — for a component whose Rust equivalent is small.

**Also worth correcting:** ADR 0017 rejected compression VFSs because stacking
one under SQLCipher is a build and correctness problem. That is right, and it
does not apply here. **Row compression sits above the pager and page encryption
below it**; they never meet, and the resulting order is compress-then-encrypt,
which is what ADR 0017 requires anyway. "No page compression" and "no row
compression" are different claims and only the first was decided.

Measured in ~40 lines against a real `rusqlite` in WAL mode, storing a trained
dictionary as a row: values round-trip, and compression behaves as expected.
zstd is already a dependency. There is no library-shaped hole here.

## What this does not change

- **Attachments and raw `.eml` stay in the blob store**, content-addressed,
  streamed, deduplicated, mostly incompressible, and encrypted per blob (#301).
  All of ADR 0017's Axis 1 stands.
- **No page compression**, now or later (ADR 0017 Axis 3).
- **Compress before encrypt**, never the reverse.
- **The id is the hash of the plaintext** for everything still in the blob
  store, which ADR 0014's keyed-BLAKE3 id depends on.

## Consequences

- A migration moves existing body blobs into rows, and the blob store loses its
  text tenants. It is not a flag day: bodies can be read from either place
  during the move.
- **Migration 0001's rule and `PRODUCT.md` §6 need amending.** "SQLite holds
  the blob key and the metadata needed to list and search" stops being true of
  bodies, and the sentence has been quoted enough that leaving it would mislead.
- **#399 as specified is no longer needed.** The blob container's dictionary id
  was for compressing text blobs; with the text gone, blobs are attachments,
  which are incompressible and want no dictionary. The format work already on
  its branch is harmless and probably not worth landing on its own.
- **#301 shrinks** to attachments — still needed, and no longer the thing
  standing between the user's prose and a stolen laptop.
- **#300 becomes the urgent one.** Once bodies are rows, SQLCipher *is* the
  body encryption. It was already `ready`/p2; this makes it the single highest
  security item.
- The database grows by roughly 0.9 GB on a large account, with `VACUUM` and
  backup consequences a directory of files does not have. Worth watching; not
  worth trading the metadata leak for.

## What would falsify this

- **A real account whose fetched-payload-to-text ratio is high** — someone who
  does download their attachments, where the text axis really is a small share
  and the metadata argument is doing all the work. #78 produces this number.
- **Dictionary ratios that do not survive real mail.** Every figure here comes
  from this project's corpus or from synthetic samples, and synthetic mail is
  far more self-similar than real mail. The 2.19x ceiling is the honest planning
  number; anything above it in a scratch benchmark is an artifact of the
  fixture, not a forecast.
- **Database growth hurting more than expected** — a `VACUUM` that takes
  minutes, or backups that stop being incremental.
