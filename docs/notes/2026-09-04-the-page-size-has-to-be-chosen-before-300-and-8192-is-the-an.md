# The page size has to be chosen before #300, and 8192 is the answer (2026-09-04, #381)

**A prerequisite of #300/#301.** ADR 0014's drain-and-re-encrypt migration
rewrites every page of the store. `cipher_page_size` can only be changed by
rewriting every page of the store. Doing them together is free; doing them in
either order separately is two multi-hour passes over somebody's mail. So the
number below has to be settled before #300 starts, which is why this note
exists rather than a comment in `db.rs`.

### Measured

`storage_suite/measurements.rs`, ignored by default, 20,000 seeded messages
with bodies written (the reference store is 81,744; a body-less seed is
~580 bytes a message against a real ~2 KB, and the difference is exactly the
compressed body columns, so bodies are not optional to this measurement):

```
                        bytes        pages
vacuumed  (4096)   26,963,968   6,583 of 4096
exported  (8192)   24,961,024   3,047 of 8192     -7.4%
```

Without bodies the sign flips — 11,612,160 against 11,771,904, 8192 being
**1.4% larger** — because the saving is not b-tree packing at all. It is
overflow: a compressed body column spills to an overflow chain, and doubling
the page halves the number of links in it. Measuring a metadata-only store
would have answered this question backwards, confidently.

### Three things about SQLCipher that cost an afternoon

**`PRAGMA cipher_page_size = 8192; VACUUM;` silently does nothing.** It is the
obvious spelling, it returns success, and the file comes back at exactly the
page count it had. SQLCipher's own path for changing cipher settings on a
database that already exists is `sqlcipher_export` into a database attached
with the new settings — a full rewrite, which is the point above.

**The page size is not discoverable from the file.** A store written at 8192
and opened by a connection that does not say `cipher_page_size = 8192` answers
`file is not a database`. `db::configure`'s probe turns that into
[`Error::WrongStoreKey`], so what a user would read is *"the local store will
not open with this key: it belongs to another installation, or the keyring
entry has been replaced"* — a sentence that is wrong about the cause and sends
them to their keyring. **Whatever adopts 8192 has to probe**: try the
configured size, and on `WrongStoreKey` retry at 4096 before believing it.
Two opens in the worst case, and it is the difference between a migration and
a support incident.

**`PRAGMA page_size` on a keyed connection answers a *text* column named
`cipher_page_size`.** Asking rusqlite for an `i64` fails with
`InvalidColumnType` on a value that is plainly a number.

### `auto_vacuum` is not in the same trap

It is an ordinary SQLite header field, and `PRAGMA auto_vacuum = INCREMENTAL;
VACUUM;` really does convert. It is set for new stores in `db::PRAGMAS` (first
in the batch — it only takes on a database with no tables yet) and
`Database::adopt_incremental_vacuum` converts an older one, once, from
`reclaim_disk`'s worker.

### And the index this issue is named after

`idx_recipients_draft` — the 6 MB, 3.9%-of-the-database one — has been partial
since #610 consolidated the schema. Only `idx_attachments_draft` was still
whole, and measured it costs **9.6 bytes a row**, which over the reference
store's attachment count is on the order of a hundred kilobytes rather than
six megabytes. Migration 0009 fixes it because indexing NULL for every
message in the mailbox is wrong, not because of what it reclaims. The headline
number in the issue was collected by somebody else's commit.
