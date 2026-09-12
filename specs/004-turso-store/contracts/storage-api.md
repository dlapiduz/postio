# Contract: what the storage layer offers, after the rewrite

Postio is a desktop application, so its contracts are internal: the seams
between crates. Three of them change shape here and one deliberately does not.

## 1. `postio-storage` — async repositories (changed)

Every repository method becomes `async`. The shape is otherwise preserved, so
that the change reads as a mechanical one at each call site rather than a
redesign:

```text
today   MessageRepository::new(&connection).page(&query)        -> Result<Vec<Row>>
here    store.messages().page(&query).await                     -> Result<Vec<Row>>
```

- **Opening**: `Store::open(path, key) -> Result<Store>`. Creates the schema at
  head when the file is new; refuses a database it did not write.
- **No pool.** Connections are the engine's to multiplex (research Q3).
- **The write gate stays** until R2 shows it is redundant: an interactive write
  must not queue behind a backfill.
- **Counting** is a test-only wrapper over the same entry points, replacing the
  trace hook (research Q5).

## 2. `postio-runtime::MailStore` — unchanged in shape, simpler inside (unchanged)

The trait the frontend reads through keeps every method and every signature.
Its implementation stops crossing to a blocking thread and simply awaits.

**This is the contract that matters most**, because it is what makes the
frontend's ignorance of the engine true rather than claimed: if `MailStore`
changes, `postio-gtk` has learned something about the database, and Principle
VII says it must not.

## 3. `postio-index` — same inputs, same outputs, different engine (changed inside)

```text
search(connection, SearchRequest) -> Result<Results>
```

Signature preserved. Inside, `MATCH` and `bm25()` become `fts_match` and
`fts_score`, and every text crossing the boundary is folded first.

- **`postio-search` is untouched.** It parses the query language and knows
  nothing about execution. If this feature edits it, the design is wrong.
- Ranking still combines textual relevance with recency and sender affinity.
  The relevance term's *scale* changes with the engine, so the weights are
  re-derived rather than carried over — a task, not an accident.

## 4. Crate boundaries (enforced, and needing one edit)

`scripts/checks/check-crate-boundaries.py` bans `rusqlite` from `postio-gtk`,
`-model`, `-config`, `-search` and `-body`. It must ban `turso` in the same
places, or the boundary stops being checked the moment the engine changes
name. **That edit is a task, not a footnote** — a check that silently passes is
worse than no check.
