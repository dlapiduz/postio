# Postio's schema and search on Turso, with both experimental flags on

A compatibility report, run rather than read. `cd spike/turso && cargo run
--release`.

It does not invent a schema. It builds a real Postio store through
`postio_session::open_store_at`, reads the head schema back out of
`sqlite_schema` — every table, index, trigger and FTS5 virtual table the
application actually creates — and replays that against Turso with
`experimental_encryption` and the rest turned on. The head schema rather than
the twenty migrations, because a rebuilt store is created at head and
replaying `ALTER TABLE` history would report failures a rewrite would never
meet.

## Result

```
── Postio's head schema, out of a real store ──
   102 objects: 56 index, 31 table, 15 trigger

── applying it ──
   96 applied, 6 refused

     index      4   idx_thread_links_lookup, idx_thread_links_thread,
                    idx_message_labels_label, idx_message_headers_name
     table      2   messages_fts, message_bodies_fts

       4x  Parse error: CREATE INDEX on WITHOUT ROWID tables is not supported
       2x  Parse error: no such module: fts5
```

**94% of the schema applies unchanged**, triggers included — which is more
than I expected and worth saying plainly.

## The encryption is real

```
header       = [54, 75, 72, 73, 6f, ...]        # "Turso\0", not "SQLite format 3"
plaintext    = false
scan         = no plaintext found               # "invoice", "Thursday", "CREATE TABLE"
wrong key    = refused
```

AES-256-GCM, keyed with a 256-bit hex key — the same shape as Postio's own
store key, and no passphrase KDF. It needs
`.experimental_encryption(true)`, and upstream is explicit that the feature
has not had a third-party security audit.

## Full-text search exists — and it is not FTS5

```
CREATE INDEX mail_fts ON mail USING fts (subject, body)          ok
SELECT … WHERE fts_match(subject, body, 'invoice')               1 hit, score 1.297
```

An **index method over an ordinary table**, not a virtual table; queried
through `fts_match`, `fts_score` and `fts_highlight` rather than `MATCH`,
`bm25()` and `snippet()`. It works, and it is a different API — so "FTS5 is
missing" and "there is no full-text search" are different findings and only
the first is true.

## What it looks like with compression removed

`cargo run --release --bin uncompressed` builds both stores over the same
4,000 messages — text from the real `.eml` corpus, not generated, because
`body.rs` warns that *"generated mail compresses 6-7x and that number means
nothing"*.

### The body column

```
plaintext, as Turso would need it      1292504 bytes
zstd + dictionary, as stored today      930929 bytes   1.39x smaller
```

The column, not the file: comparing whole stores would compare a full Postio
schema — 40 tables, 56 indexes, 15 triggers — against a single three-column
table, and say nothing about compression. Repeated corpus text flatters a
dictionary, so 1.39x is an *upper* bound on what is given up; the reference
figure stays `body.rs`'s **2.19x on a real account's 1.43 GB text axis**.

### The searches, over the same field

```
invoice    fts5      0 hits   370µs     turso      0   105µs
meeting    fts5    105 hits   221µs     turso    105    97µs
thé        fts5   3472 hits   546µs     turso      0    90µs
the*       fts5   3472 hits   337µs     turso   3472    95µs
```

Three of four agree, and Turso is consistently faster on this corpus.

**`thé` is the finding.** FTS5 under `unicode61 remove_diacritics 2` folds it
to `the` and matches 3,472 messages. Turso's tokenizer is tantivy's
`SimpleTokenizer` plus `LowerCaser` — no diacritic folding — so it matches
nothing. Searching `Muller` would not find `Müller`, and `Jose` would not find
`José`. For a mail client with European correspondents that is a visible
regression, and it is not configurable through the index method.

Prefix (`the*`) works identically in both, which is worth saying: the gap is
specific, not general.

## What a rewrite would actually cost

1. **`postio-index` and `postio-search`, rewritten.** Not ported — the
   execution half has no counterpart. Postio's search has scopes, facets,
   operators (`from:`, `header:`), two ranking orders and highlighting, all
   expressed against FTS5. The query *parser* survives; nothing under it does.
2. **Four indexes have nowhere to go.** `CREATE INDEX` on a `WITHOUT ROWID`
   table is unsupported, and those four are lookups on join tables —
   `thread_links`, `message_labels`, `message_headers`. Either the tables stop
   being `WITHOUT ROWID`, or the lookups lose their index.
3. **Every repository becomes async.** `postio-storage` is synchronous and
   crosses to the runtime through `spawn_blocking`; Turso is async-native.
   Probably a simplification in the end, and a rewrite of every call site to
   get there.
4. **The counted-cost instrument goes.** `postio_storage::test_support::counting`
   reads SQLite's own trace hook — it is what caught #1479, and it is the
   project's main performance gate. There is no equivalent, and no
   `EXPLAIN QUERY PLAN` assertions either.

## Not measured, deliberately

Speed. The decision does not turn on it: the schema and search gap is what
costs, and no plausible speed result would justify rewriting search against
an engine whose encryption has not been audited.

## Status upstream

`turso 0.8.0-pre.11` — a prerelease, pre-1.0. Encryption at rest and
full-text search are both listed experimental.
