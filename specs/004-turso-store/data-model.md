# Phase 1 data model: The store, rebuilt on Turso

The schema is Postio's existing one at head. A spike applied 96 of its 102
objects to Turso unchanged — 39 tables, 56 indexes, 15 triggers. This records
only what **changes**, and why.

## What changes

### 1. Bodies become text

| | today | here |
|---|---|---|
| `messages.body_text` | `BLOB` — zstd, dictionary-compressed | `TEXT` — the extracted plain-text body |
| `messages.body_html` | `BLOB` — zstd | `TEXT` |
| `messages.body_dictionary_id` | FK to `body_dictionaries` | **gone** |
| `body_dictionaries` | table | **gone** |

A column-indexing full-text engine cannot tokenise compressed bytes. Bodies
must be readable text for the index to exist at all.

`body_html` follows `body_text` rather than staying compressed: keeping one
compressed and one not would mean two body paths, and the html body is not
indexed anyway — so the reason it changes is consistency of the read path, and
that is worth saying because it is the weaker of the two arguments.

### 2. A folded column, written only when folding changes something

```
messages.body_search   TEXT NULL   -- NFKD, combining marks dropped, lowercased
                                   -- NULL when it would equal body_text
```

The engine folds no diacritics (research Q1). This is the column the index
reads. `NULL` for text that folds to itself — most mail in most mailboxes —
so the duplication is paid only by messages that carry accents.

`search_documents`'s metadata columns take the same treatment: the folded form
is what is indexed, the original is what is displayed.

### 3. Three tables stop being `WITHOUT ROWID`

`thread_links`, `message_labels`, `message_headers`. Turso will not put an
index on a `WITHOUT ROWID` table, and these are join tables that exist to be
looked up.

### 4. The FTS5 virtual tables become indexes

| today | here |
|---|---|
| `CREATE VIRTUAL TABLE messages_fts USING fts5(…, content='search_documents', content_rowid='message_id', tokenize='unicode61 remove_diacritics 2')` | `CREATE INDEX messages_fts ON search_documents USING fts (sender, recipients, subject, filenames, list_id)` |
| `CREATE VIRTUAL TABLE message_bodies_fts USING fts5(body, content='', contentless_delete=1, …)` | `CREATE INDEX message_bodies_fts ON message_bodies USING fts (body)` |

`content=''` has no counterpart — the index reads a column, so the column must
exist. `message_bodies(message_id, body)` is a table of its own rather than a
column on `messages` for a reason worth keeping: an index on `messages` would
be re-maintained on every flag change, and a flag change is the most common
write in the application.

`messages_fts`'s external-content shape maps cleanly: `search_documents` is
already a real table keyed by `message_id`.

The five FTS5 shadow tables (`_data`, `_idx`, `_content`, `_docsize`,
`_config`) disappear with it — they were SQLite's to create.

## What does not change

Everything else. Accounts, mailboxes, threads, drafts, contacts, labels, the
operation queue, settings, the egress log, the unsubscribe log, every foreign
key, every check constraint, and **all fifteen triggers** — including the ones
that maintain `mailboxes.total_count` and `unread_count`, which are what make
the sidebar's numbers real rather than recounted.

The `BlobStore` is untouched: attachments and raw messages stay files on disk,
content-addressed and sealed with XChaCha20-Poly1305.

## Key entities

- **Store** — one encrypted database per installation. AES-256-GCM under a
  256-bit key from the OS keyring, no passphrase KDF on the opening path.
- **Message** — headers, flags, thread membership, and its body as text plus,
  when it differs, its folded form.
- **Search index** — two fts indexes, derived entirely from rows, rebuildable
  from them, never the only copy of anything.
- **Store key** — never in config, never in a log, never derived from a
  passphrase.

## Invariants that must survive

1. No plaintext of any message in the store file.
2. The index and the rows commit together — a message and its searchability
   land atomically or not at all.
3. A read is bounded by its `LIMIT`, never by the size of the mailbox.
4. A message deleted stops matching.
5. `body_text` round-trips byte for byte: what was stored is what is displayed
   and what is quoted in a reply.
