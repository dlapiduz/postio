# ADR 0017 — What "download everything" costs, and the four axes that pay for it

- **Status:** Accepted — **GO** (2026-08-26)
- **Date:** 2026-08-26
- **Amended:** 2026-08-27 — why blobs rather than compressed rows, and the
  dictionary deferred behind a measurement ([#399](https://github.com/dlapiduz/postio/issues/399))
- **Decision by:** the maintainer, asking what ADR 0016 actually implies for
  attachments, memory, disk, compression and encryption
- **Extends:** [ADR 0016](0016-full-mailbox-backfill-by-default.md) (full-mailbox
  backfill by default), [ADR 0014](0014-encryption-at-rest.md) (the local store
  encrypts itself)
- **Related:** `PRODUCT.md` §6 (what is stored locally), §7 (search), §11
  (attachments), §14/§15 (sync, offline), §18 (performance),
  [#318](https://github.com/dlapiduz/postio/issues/318),
  [#327](https://github.com/dlapiduz/postio/issues/327),
  [#350](https://github.com/dlapiduz/postio/issues/350),
  [#352](https://github.com/dlapiduz/postio/issues/352),
  [#299](https://github.com/dlapiduz/postio/issues/299)/[#300](https://github.com/dlapiduz/postio/issues/300)/[#301](https://github.com/dlapiduz/postio/issues/301)
- **Decision:** **backfill splits into a text axis and a payload axis.** The
  text axis is what ADR 0016 means by "every message's body": headers and every
  `text/*` part, for every message, to completion, by default. The payload axis
  — attachment bytes — is separately governed, defaults to on-demand, and is
  never a precondition for search or for offline reading. Blobs are compressed
  under the AEAD; the FTS5 index becomes contentless; the store gains a byte
  budget with eviction. Nothing here weakens ADR 0016: it is what makes ADR 0016
  affordable.

---

## The measurement this decision rests on

ADR 0016 decided the horizon is the whole mailbox and left the cost unpriced.
It is priced now, against the reference account `engineering-notes.md` already
cites — 81,744 messages, whose `BODYSTRUCTURE` metadata is fully synced, so
every number below is known *before a single body byte is fetched*:

| | messages | bytes |
|---|---:|---:|
| the whole mailbox | 81,744 | **12.43 GB** |
| … of which attachment payloads | 25,752 parts | **11.00 GB (88.5%)** |
| … of which everything else (headers + `text/*`) | all | **1.43 GB (11.5%)** |
| messages carrying an attachment | 12,712 (15.5%) | 11.26 GB (90.6%) |
| messages carrying none | 69,032 (84.5%) | 1.17 GB (9.4%) |
| over the existing 5 MB `max_body_bytes` cap | 539 (0.66%) | 6.02 GB (48.4%) |

The database holding *only* the metadata for those 81,744 messages, with 902
bodies fetched, is already **163 MB** (plus an 18 MB WAL). The blob store for
those 902 messages is 118 MB, of which 90 MB is raw source and 28 MB is the
decoded text and HTML stored *alongside* it — every fetched message is
currently stored **1.3 times**.

Three things follow immediately, and they are the whole of this ADR:

1. **Ninety percent of a mailbox by weight is bytes FTS5 cannot index.** A PDF,
   a JPEG and a ZIP contribute their filename to search and nothing else. The
   corpus that makes search complete — ADR 0016's own justification — is the
   1.43 GB, not the 12.43 GB.
2. **The 5 MB cap is not a rounding error, it is half the mailbox**, and it
   achieves that by refusing 0.66% of messages. A cap that good is a cap that is
   doing the split badly: it is discriminating by message size when the thing
   worth discriminating by is *part kind*.
3. **We know all of this in advance.** `BODYSTRUCTURE` arrives with the header
   fetch. Postio can tell a user "1.4 GB of mail, 11 GB of attachments" on the
   first run and let them choose, having spent nothing.

## Axis 1 — Attachments are not bodies

### What the code does today

`postio-sync::backfill::fetch_body` issues `BODY.PEEK[]` — the whole message,
attachments and all, base64-inflated — buffers it in a `VecSink`, `mime::parse`s
it, and writes the entire thing to `messages.raw_blob_id`, plus decoded text and
HTML blobs beside it. `attachments.blob_id` is never written on the receive path
at all; `engineering-notes.md` already records this as a wart, and
`postio_app::reading::part_bytes` works around it by re-parsing the whole raw
blob to extract one part by its `part_id`.

So today ADR 0016 does not mean "download every body". It means **download every
byte of every message, twice over, through the heap.**

### The decision

**Two axes, governed separately.**

- **The text axis — this is ADR 0016.** For every message in every non-excluded
  selectable folder: fetch the header block and every `text/*` part, decode them,
  store them as the `body_text` / `body_html` blobs, index the text. Runs to
  completion, in the background, throttled by the existing `BackfillPolicy`.
  This is the search corpus and the offline-reading corpus, and on the reference
  account it is under 1.5 GB in wire form before compression.
- **The payload axis — attachment part bytes.** Governed by a new
  `AttachmentPolicy` with three settings: `on_open` (default), `eager`, `never`.
  `on_open` fetches the part when the user opens or saves it, by `part_id`, into
  `attachments.blob_id` — the column the schema always intended and the receive
  path has never filled. `eager` backfills payloads too, for the user who wants
  a genuinely complete offline archive and has the disk. `never` means filename
  search and nothing more.

**Bodies stop being lazy; only attachments stay lazy.** This is the sentence
`PRODUCT.md` has had backwards since it was written. §11 says attachments are
fetched "lazily, like bodies" and §14 lists "lazy body and attachment fetch" —
but under ADR 0016 a body is not fetched lazily at all: every one of them is
pulled unprompted, to completion, because that is what makes search complete and
offline reading real. Laziness is now exactly one thing, and it is the payload
axis. A message's text is *eager*; its attachments are *lazy*. Saying it the old
way describes a product where search has holes in it.

The machinery for this already exists and has no production caller:
`BodyPart::Section(spec)` maps to `BODY.PEEK[2.1]`, `attachments.part_id` already
stores exactly that spec, and `stream_windows` already fetches a named section in
bounded 128 KiB windows. Nothing new is needed at the protocol layer.

### Inline parts ride with the text

`disposition = 'inline'` is 2.64 GB of the reference account — CID images in HTML
mail. A message whose inline images are missing renders as broken boxes, and the
reader already blocks *remote* images by default, so CID parts are the images
that are supposed to appear. Rule: **inline parts under `max_inline_bytes`
(default 256 KiB) belong to the text axis**; larger ones are payloads. HTML mail
therefore reads correctly offline without pulling the 40 MB video someone
embedded.

### `body_state` finally means something

The `partial` variant has been in the schema's `CHECK` constraint since migration
0001 and nothing has ever written it. It is now the honest state of a
text-backfilled message: **`partial` = text local, payloads not.** `full` means
every part is local. That distinction is precisely what #352 needs in order to
tell the user when search is answering for an incomplete corpus, and what the
attachment chip needs in order to show "download" rather than "open".

### Raw source becomes a cache

Nothing reads `messages.raw_blob_id` except part extraction, which moves to
`attachments.blob_id`. What genuinely needs exact original bytes — "view source",
forward-as-`message/rfc822`, and eventually PGP/S-MIME verification over the
signed bytes — is on-demand and refetchable. So raw source is **retained only as
an evictable cache entry**, not written unconditionally on every backfill. That
alone removes the 1.3x duplication measured above.

## Axis 2 — Memory: the store grows, the process must not

ADR 0016 correctly separated "on disk" from "in memory" and pointed at §18's
windowed list. It did not look at the *fetch* path, which has three real faults:

1. **`VecSink` buffers whole messages on the heap.** The 5 MB cap bounds the
   background lane — but the interactive lane **ignores the cap by design**,
   because the user is watching a spinner. Opening the largest message in the
   reference account therefore allocates it in full, `mime::parse` walks that
   allocation, and `BlobStore::put` takes it again. There are 539 messages over
   5 MB in one account. This is a reproducible multi-hundred-megabyte spike
   behind a single keypress.
   **Fix: a `BlobSink`** that streams arriving bytes straight into the blob
   store's existing temp-file-then-rename path in 64 KiB chunks and yields a
   `BlobId`. Attachment payloads then never enter the heap at all: socket →
   temp file → rename → `attachments.blob_id`.
2. **Parsing scales with the fetch, not with the text.** With the text axis,
   `mime::parse` sees the header block and the text parts, which are kilobytes.
   The 40 MB parse disappears with the 40 MB fetch.
3. **The backlog is unbounded in principle.** `Backfill` holds a `BinaryHeap` of
   `BodyRequest`, each carrying an owned mailbox path `String`, seeded
   `seed_batch` per folder across every folder and re-seeded on drain. That is
   fine at 200 × 40 folders and is not fine if a future re-seed reads "the whole
   folder". The heap gets an explicit cap and the path gets interned per mailbox.

**The rule this axis states, and which the benches will enforce:** *no message
byte is ever resident in the process except the text being parsed.* Payload bytes
go socket-to-disk and are read back streaming, exactly as `BlobStore::reader`
already promises.

## Axis 3 — Disk and the database

### The blob store: compress under the AEAD

Message text compresses 5–8x with zstd, and mail text compresses far better than
that in aggregate because every reply quotes its parent and every message carries
the same signature. **Blobs are stored zstd-compressed**, level 3, with a
dictionary trained on the store's own text corpus and versioned in the blob
header. **Superseded by ADR 0020**: text bodies move into SQLite rows and are
compressed there, so the blob store's dictionary is not needed — what remains
in it is attachments, which are incompressible.

Three constraints on how:

- **The id is the hash of the plaintext.** Compression happens *below* the
  content-addressed name, so dedup is unaffected and ADR 0014's keyed-BLAKE3 id
  is unaffected. The pipeline is `id = BLAKE3_keyed(plaintext)`, then
  `compress`, then `encrypt`.
- **Compress then encrypt, never the reverse.** Ciphertext does not compress. The
  usual objection — CRIME/BREACH — is a chosen-plaintext-with-an-oracle attack
  and does not apply to blobs at rest: an attacker holding the disk already reads
  the file's length. Recorded here so it is not re-litigated.
- **Do not compress the incompressible.** 8.9 GB of the reference account's
  payloads are JPEG, PNG, PDF and ZIP. Skip by MIME type, plus a cheap
  incompressibility probe over the first 128 KiB, and record the outcome in the
  header's format byte.

**Measured, and lower than this ADR first claimed.** The original text here
said mail compresses 5–8x and projected the 1.43 GB text axis landing under
350 MB. That is true of mail text *in bulk* and not of mail text compressed one
body at a time, which is what a content-addressed store does. Measured on the
project's own corpus, over decoded `text/*` bodies only (median body: 325
bytes):

| | ratio |
|---|---:|
| per blob, zstd-3, no dictionary — what ships | **1.57x** |
| the same bodies in one frame — the ceiling a shared dictionary approaches | 2.19x |
| whole `.eml` files per blob, base64 payloads included | 1.37x |

Small inputs are the whole story: zstd has almost no window to work with in a
few hundred bytes, and a dictionary is the standard answer. Real accounts skew
larger than this corpus, so real ratios sit above these — but the honest
planning number for the text axis is **around 2x, not 4x**, until a dictionary
exists. Payloads compress by essentially nothing, which is the correct result.

That is a smaller prize than this ADR first advertised, and it does not change
the decision: 2x on the corpus that search reads is worth having, and the
*format* work is what has to happen now regardless, because a container that
cannot name its dictionary can never gain one.

### The database: shrink the content, do not compress the pages

**SQLite pages are not compressed.** The options are a compressing VFS
(`sqlite-zstd`, or the proprietary ZIPVFS), and ADR 0014 is about to put
SQLCipher underneath, where stacking a second VFS is a build and correctness
problem far larger than the saving. Decision: **no page compression, now or
later** — shrink what is stored instead. Four things do that, in descending
value:

1. **`messages_fts` becomes contentless.** This is the important one.
   `search_documents` is an ordinary table holding a **full copy of every
   message's body text** inside SQLite, existing only to feed an
   external-content FTS5 table. With bodies actually indexed — which they were
   not until #327 — that is the entire text corpus duplicated into the database:
   projected several hundred megabytes for the reference account, on top of the
   index itself. It also quietly breaks §6's own rule that bodies live in the
   blob store and SQLite holds metadata.
   `content=''` makes FTS5 store the inverted index and no copy of the text.
   **The cost is real and is taken deliberately:** `snippet()` and `highlight()`
   stop working and `rebuild` stops working, so Postio generates result
   highlights itself from the blob it already has, and the maintenance pass
   re-indexes from the blob store rather than from a shadow table. That is a
   fair price for removing a duplicate of the whole corpus from the hot,
   encrypted, budget-gated path.
2. **Delete two indexes that index nothing.** `idx_recipients_draft` and
   `idx_attachments_draft` are not partial, so they index all 378,819 recipient
   rows and every attachment row — of which **zero** have a `draft_id`.
   `idx_recipients_draft` alone is 6 MB, 3.9% of the database. Adding
   `WHERE draft_id IS NOT NULL` is one migration and costs nothing.
3. **Normalize addresses out of `recipients`.** `recipients` and its indexes are
   **56 MB, 34% of the database** — 378,819 rows at 4.6 per message, each storing
   `address` and a near-duplicate `address_normalized`. An `addresses` table with
   a foreign key collapses that to a few tens of thousands of distinct strings.
4. **`auto_vacuum = INCREMENTAL`, and `page_size = 8192`.** The store is
   `auto_vacuum = 0` today, so deleted mail never returns its pages. Both pragmas
   require a full `VACUUM` to take effect — **which means they must be chosen
   before ADR 0014's drain-and-reencrypt migration runs**, since that migration
   rewrites the whole database anyway and doing it twice is a second hours-long
   pass over a user's mailbox. This is a sequencing constraint on #300/#301, not
   a preference.

### A byte budget, because the store is a cache

ADR 0014 says it plainly: everything but drafts and the operation queue can be
re-synced. So the store gets `[storage] max_bytes`, and exceeding it evicts
**refetchable** blobs — raw source first, then attachment payloads, least
recently used — never text bodies (they are the search corpus), never drafts,
never the queue. Eviction sets `body_state` back to `partial` so the UI keeps
telling the truth. `BlobStore::collect_garbage` already sweeps unreferenced
blobs; this is the same sweep with a second predicate.

## Axis 4 — What this does to encryption at rest

ADR 0014 is accepted and entirely unimplemented (#299, #300, #301). Full backfill
changes its cost model, and the changes must be recorded *before* those three
land rather than discovered by them.

1. **The threat model's stakes change, though the model does not.** ADR 0014 was
   written when the local store held a few hundred messages per folder. Under
   ADR 0016 it holds **a complete replica of the user's mail**. Nothing in
   0014's protected/not-protected lists changes; what changes is that at-rest
   encryption moves from prudent to **required before v1**, and that the privacy
   page acquires a duty it did not have: to say that a backup of
   `$XDG_DATA_HOME` is now a backup of the entire mailbox, and that losing the
   keyring entry costs a re-sync and never costs mail.
2. **ADR 0014 priced SQLCipher against the wrong database.** Its gate is the
   `<100 ms` search budget "on the 120k-message index" — an index that, when it
   was written, held metadata. Contentless FTS (Axis 3) is what keeps that gate
   reachable: it removes the duplicated corpus from the encrypted pages the
   search path touches. **Axis 3 and Axis 4 are one decision seen twice.** The
   benches that gate #300 must be re-baselined against a **fully backfilled**
   store, not against a seeded fixture, or they will certify a 163 MB database
   and ship against a 900 MB one.
3. **The blob header must carry compression from day one.** #301 defines
   `magic ‖ nonce ‖ ciphertext‖tag`. Compression is a field in that header, not a
   later addition — retrofitting one into a format already written across a
   user's whole mailbox is a flag day. The ordering is fixed above:
   keyed-hash the plaintext, compress, encrypt.
4. **Keyed ids and dedup are unaffected**, since the id is taken before both
   compression and encryption.
5. **Eviction does not need secure erase.** An evicted blob's ciphertext is
   unlinked without overwriting its blocks. Under 0014's threat model —
   stolen disk, no key — ciphertext is nothing, so no shred pass is warranted
   and none should be added.
6. **The download itself is a privacy-adjacent cost.** Pulling 1.4 GB (or 12 GB
   with `eager` payloads) is real money on a tether and real time on a slow link.
   `pause_on_metered` exists and stays. What is added is honesty: because
   `BODYSTRUCTURE` gives us the totals for free, first run *states the number*
   before spending it.

## Amendment — blobs rather than compressed rows, and what the dictionary is worth

> **Superseded by [ADR 0020](0020-where-message-bodies-live.md) (2026-08-27),
> and wrong in a way worth leaving visible.** The reasoning below computes the
> dictionary's value as 2.2% of the store by dividing into a 12.43 GB mailbox —
> but this ADR's own decision line says the payload axis "defaults to
> on-demand", so those 11 GB are not on disk unless the user asks. Against the
> default store, compression of bodies is worth 36% and the dictionary a
> further 28%. The section's own table has the correct row in it, labelled as
> a mailing-list edge case; it is the default configuration.
>
> The argument against putting the *blob store* in rows is sound and survives.
> The argument it was used for — that bodies belong in the blob store — does
> not: every load-bearing reason (streaming, dedup, 11 GB) is a reason about
> attachments.

Asked directly: *why hand-roll any of this instead of using something that
compresses SQLite?* The question is a good one and it has two halves. The first
was already answered above and is restated here because it kept being
re-asked. The second changes a decision this ADR made.

### Why the bodies are not SQLite rows

Page compression is settled: **no**, for the reason Axis 3 gives — SQLCipher is
going underneath, and stacking a second VFS is a build and correctness problem
larger than the saving. Row-level compression (`sqlite-zstd`, which does
per-column zstd with automatic dictionary training — very nearly this issue, as
a library) does not rescue that: it would have to be a loadable extension
inside an encrypted database, and the store bundles rusqlite statically.

But the deeper reason is that **the bytes are not shaped like rows**:

- **88.5% of a real mailbox is attachment payloads**, and 8.9 GB of the
  reference account's 11 GB is JPEG, PNG, PDF and ZIP. Compression buys
  essentially nothing on those, so the thing a compression extension is *for*
  does not apply to the overwhelming majority of the bytes.
- **Reads must stream.** `BlobStore::reader` hands back a file, so a 30 MB
  attachment never exists whole in memory. Row-level compression decompresses
  a whole value.
- **The id is the hash of the plaintext**, which is what makes the store
  content-addressed: an attachment sent to five people, or quoted down a
  forwarded thread, is stored once. ADR 0014 builds its keyed-BLAKE3 id on
  that. Rows have no such property; getting it back would mean a
  content-addressed table — the blob store, reimplemented inside SQLite, minus
  streaming.
- An 11 GB database is a `VACUUM` that rewrites 11 GB, a backup that copies it
  whole, and one file whose corruption costs everything rather than one
  message.

So the blob store is not a rejection of libraries — `zstd` does all the actual
compression. It is the shape the data has. The ~12 bytes of container header
exist for the one thing no compression library can answer: *is this blob
compressed at all*, given that most of them deliberately are not.

### What the dictionary is worth, which is less than this ADR implied

Axis 3 above says blobs are stored "with a dictionary trained on the store's
own text corpus". #399 is that work. Doing the arithmetic before building the
training half turned up a number that should have been here from the start.

The measured ratios are per-blob **1.57x** against a shared-dictionary ceiling
of **2.19x**. That is a 40% improvement, and 40% is how #399 states it — but it
is 40% *of the smallest axis*. On the reference mailbox:

| | |
|---|---:|
| text axis, uncompressed | 1.43 GB |
| stored today (1.57x) | 0.91 GB |
| stored with a perfect dictionary (2.19x) | 0.65 GB |
| **saving** | **0.26 GB** |
| **as a share of the 11.9 GB store** | **2.2%** |

Against that 2.2%, a dictionary is **permanent, load-bearing data**:

- Lose it and every blob written against it is unreadable. That is a *new way
  to lose mail* in a store whose entire design is that the id is the hash of
  the plaintext, so bytes can always be verified.
- It is derived from the user's mail, so ADR 0014 requires it inside the
  encrypted store — a new table, a new backup-and-restore obligation.
- Dictionaries accumulate and may never be dropped, so the store carries every
  one it ever trained.
- It adds an error class that looks exactly like corruption and is not.

**Decision: keep per-blob compression; do not build the dictionary yet.** What
ships today (1.57x, no dictionary, no invariant) already takes the text axis
from 1.43 GB to 0.91 GB. The marginal 0.26 GB does not buy a permanent
data-loss-shaped invariant.

### The condition that would change it

The prize is entirely a function of the account's shape — how much payload
sits beside the text:

| payloads beside a 1.43 GB text axis | dictionary saves |
|---|---:|
| 11.0 GB (the reference account) | 2.2% |
| 3.0 GB | 6.6% |
| 1.0 GB | 13.5% |
| 0.15 GB (mailing lists, no attachments) | **24.3%** |

So this is not "the dictionary is not worth it". It is **"the dictionary is
worth it for text-heavy accounts and not for attachment-heavy ones, and the
only account measured is attachment-heavy."** A subscriber to busy lists who
never receives attachments is at the bottom row, where it is worth a quarter of
their store.

Build it when a real account measurement shows payload-to-text below roughly
3:1. Until then the format work stands — the container header carries the
dictionary id, `BlobStore` resolves it, and a blob written against one reads
back — so this is a decision that can be reversed by writing the training half,
with no migration and no flag day. That was the point of reserving the field in
#380, and it is still being served.

### What would falsify this

- A real account whose payload-to-text ratio is far below the reference
  account's 7.7:1. That is the measurement #78 needs a live server for, and it
  answers this question as a side effect.
- Text bodies compressing materially better than 2.19x under a dictionary
  trained on a real corpus rather than this project's synthetic one. The 2.19x
  ceiling is measured on 902 messages; a real corpus has more redundancy to
  find, and if the ceiling is nearer 3x the arithmetic moves.

---

## Consequences

- **`PRODUCT.md` §11 and §14 are rewritten, not patched.** Both currently say
  bodies are fetched lazily. Under ADR 0016 they are not: bodies are eager and
  complete, and lazy describes attachments alone. §6 regains its "bodies are not
  in SQLite" rule via contentless FTS.
- `BackfillPolicy` is unchanged in shape; `max_body_bytes` now measures the text
  axis, where it will essentially never bind, and a new `AttachmentPolicy`
  governs payloads.
- `messages.raw_blob_id` stops being written unconditionally and becomes a cache
  entry. `attachments.blob_id` gains its first receive-path writer, which retires
  the `engineering-notes.md` wart and makes `parts::Node::downloaded` meaningful.
- #350 (per-folder opt-out) is unaffected and orthogonal: it excludes a folder
  from both axes.
- #352 (search corpus honesty) gains the vocabulary it needs — `partial` versus
  `full` — and a much smaller gap to report, since the text axis completes
  roughly nine times sooner than a whole-message backfill would.
- #300/#301 acquire two hard prerequisites: the pragma choices in Axis 3 must be
  made before the re-encrypt migration, and the blob header must carry a
  compression field.
- `docs/engineering-notes.md` records the measured shape of a real mailbox —
  90% payload by weight, 15% of messages carrying it — because every future
  sizing argument in this project will want that number and nobody should have
  to re-derive it.

## What would falsify this

- **If the text axis on a real account turns out not to be dominated by
  attachments** — a mailbox that is 80% plain text by weight — the split buys
  little and the simpler `BODY.PEEK[]` path is worth keeping. The reference
  account says 88.5%; a second account saying 20% reopens this.
- **If contentless FTS makes result highlighting materially worse** — highlights
  regenerated from the blob disagreeing with what FTS5 matched, on real queries —
  that reopens Axis 3's first item toward `detail=none` with content retained,
  and the database pays for it.
- **If per-part fetching costs more than it saves**, because a message with
  twelve small text parts becomes twelve round trips where one `BODY.PEEK[]`
  was one, the text axis needs part coalescing (a single `FETCH` naming several
  sections) before it is worth it. That is a protocol-level fix, not a reason to
  fetch payloads nobody asked for.
