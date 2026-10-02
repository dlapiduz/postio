# ADR 0025 — Arbitrary headers are stored on the row and indexed as rows, not as text

- **Status:** Accepted (2026-09-03). Built (`postio-index/src/index.rs`);
  the size budget is [ADR 0027](0027-the-header-index-is-budgeted-per-message.md)'s
- **Date:** 2026-09-03
- **Decision by:** a `/ux-architect` session, on the question
  [#884](https://github.com/dlapiduz/postio/issues/884) raised: `header:` has
  nowhere to match against, and choosing where is a storage decision.
- **Issue:** [#884](https://github.com/dlapiduz/postio/issues/884)
- **Related:** [ADR 0008](0008-filters-and-rules.md) Q2 (`header:` is one
  `Field` row) and Q3 (a rule fires when every fact it needs exists),
  [ADR 0016](0016-full-mailbox-backfill-by-default.md) (every body ends up
  local), [ADR 0020](0020-where-message-bodies-live.md) (`body_headers`),
  [ADR 0027](0027-the-header-index-is-budgeted-per-message.md) (the budget),
  [ADR 0038](0038-the-store-is-turso-not-sqlcipher.md) (the engine),
  `PRODUCT.md` §6 (what is stored locally), §7 (search)
- **Decision:** **the header block is persisted to `messages.body_headers`; a
  normalized `message_headers(message_id, name, value, ordinal)` table in
  `postio-index`'s schema is what `header:` matches against; and *every*
  header is indexed, bounded by two structural caps rather than by any list of
  header names.** `header:` is a body-class fact, answerable exactly when
  `body:` is, and a header that must be answerable earlier earns a dedicated
  operator instead (Q4).

---

## Three facts that shape the answer

**1. The header block lives on the row.** `messages.body_headers` holds it,
as plain text under the engine's page encryption (ADR 0038). Both backfill
paths and `send` store it.

**2. The raw blob cannot be the source of truth.** The full header block is
recoverable from `raw_blob_id`, but `PRODUCT.md` §6 says the store "evicts
what it can refetch — **raw source first**". A `header:` answered out of the
raw blob would stop working the first time the store hit its size limit,
silently, on the oldest mail. And the section path (`fetch_parts`, the
`partial` state) never fetches a raw blob at all.

**3. `Message.headers` is the matcher's input.** `postio_model::Headers`
preserves wire order, duplicates and case-insensitive lookup, and the store
persists it, so a `Message` has the same header block on arrival and on
reload — the symmetry the differential test between the two evaluators
(ADR 0008 Q1) depends on.

---

## Q1 — Where the header text lives: `messages.body_headers`

It is stored **whole**, not filtered, and that is the load-bearing part: it is
what makes the index rebuildable and the indexing policy *revisable* without
touching the network. Change a cap in Q3, bump the index half's version, and a
local pass refills from `body_headers`. Store only what the current policy
indexes and every future change to that policy costs a re-download of the
mailbox — which means it never happens.

Bounded at **256 KiB** per block, the pathological case only; longer is
truncated and the row marked, in the same spirit as `BackfillPolicy`'s
`max_body_bytes`.

## Q2 — What `header:` matches against: a normalized table, in the index's schema

**Decision: `message_headers(message_id, name, value, ordinal)`, an ordinary
table created by `postio_index::index::ensure_schema` as a third half beside
`metadata` and `bodies`.** Not a full-text index.

**Tokenization destroys exactly the values people match on.** `header:` is
wanted for `x-mailer=mutt`, `x-spam-status=fail`, `authentication-results=spf=pass`,
`content-type=multipart/signed`, `precedence=bulk`. A word tokenizer splits
`spf=pass` and `1.5.24` and `<list.example.com>` into pieces and loses the
adjacency that made them meaningful. Header values are short, structured and
matched by substring; that is a `LIKE` on a column, not an inverted index.

**One full-text row per header cannot say which message it belongs to.** A
message has many headers, so the row key would be a header id, and mapping it
back needs a side table of `(row, message_id)` — a content table with the text
removed. Having built it, the honest thing is to put the value in it.

**A content table here is bounded, which is what makes it acceptable.** What
made a full copy of every body in the metadata index unacceptable (ADR 0016)
was that it was *unbounded*. A message's header rows cannot exceed 64 × 512
bytes however pathological the mail. They are not small — measured, the
header rows and their index cost about seventeen times the metadata half of
the index — and ADR 0027 Q4 holds them to a per-message ceiling.

**It belongs to `postio-index`, not `postio-storage`.** It is derived data with
a local generator: droppable, rebuildable from `body_headers` with no network,
versioned by a `headers` row in `search_schema` on the terms
`BODIES_SCHEMA_VERSION` documents. A bump drops the table; the catch-up pass in
Q5 refills it in the background. That is the whole mechanism by which the
policy in Q3 stays revisable.

Shape and indexes, as `crates/postio-index/src/index.rs` declares them:

```sql
CREATE TABLE IF NOT EXISTS message_headers (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,   -- lowercased; RFC 5322 names are case-insensitive
    value      TEXT    NOT NULL,   -- unfolded, RFC 2047-decoded, truncated per Q3
    ordinal    INTEGER NOT NULL,   -- occurrence index within the message, wire order
    PRIMARY KEY (message_id, name, ordinal)
);

CREATE INDEX IF NOT EXISTS idx_message_headers_name ON message_headers (name, message_id);
```

It is not `WITHOUT ROWID`: this engine puts that behind an experimental flag
and will not build a secondary index on such a table.

`header:name` compiles to `EXISTS (SELECT 1 FROM message_headers WHERE
message_id = m.id AND name = ?)`; `header:name=value` adds
`AND value LIKE '%' || ? || '%'`. Both are narrowed by `name` first, and
`idx_message_headers_name` makes that a range over one name rather than the
table. This is a different compilation shape from every other operator — the
rest become a full-text match (`fts_column_condition`) — so
`search_statement_budget.rs` asserts on it.

## Q3 — Which headers: all of them, bounded by structure, never by a name list

**Decision: every header is indexed, one row per occurrence, subject to two
caps. There is no allowlist and no denylist of header names.**

| Cap | Value | What it removes |
|---|---|---|
| Value length | **512 bytes**, truncated | DKIM/ARC signatures, long spam reports — high-entropy blobs nobody substring-matches |
| Rows per message | **64**, in wire order | The pathological block; a long `Received` chain past the 64th field |

A curated list of header names is a list somebody maintains forever, and every
name off it makes `header:x-whatever` answer "no such mail" — which is not "we
did not index that", it is a **lie**, and the search bar has no way to say
otherwise. Worse, a list of the headers worth indexing converges on `X-GM-*`,
`X-MS-Exchange-*`, `X-Google-*`: named constants for particular providers in the
one part of the code `PRODUCT.md` §3 is most explicit about.

Structural caps have neither problem. They are provider-neutral, they need no
maintenance, they bound the cost predictably, and what they exclude is
excluded for a stated reason a user can reason about.

**Truncation is a correctness hazard, not just a cost knob.** The in-memory
matcher holds the full value; the index holds a 512-byte prefix. They agree
only because normalization — lowercase the name, unfold (RFC 5322 §2.2.3),
decode encoded words (RFC 2047), collapse whitespace, truncate to 512 bytes —
is **one function in `postio-model::headers`** (`normalize_name`,
`normalize_value`), which both evaluators use and neither has its own.

**The budget** is [ADR 0027](0027-the-header-index-is-budgeted-per-message.md)'s:
`message_headers` and `idx_message_headers_name` together under 5 KiB a
message, measured by `header_index_size.rs` beside `body_index_size.rs`
(`crates/postio-index/tests/index_suite/`). A budget relative to the body
index was rejected: the body index holds no text, so the ratio moved with the
corpus's bodies rather than the header policy. If the ceiling is missed, the
lever is the two caps — not a list of names.

## Q4 — When `header:` can be answered: exactly when `body:` can

**Decision: `header:` is a body-class fact.** Headers arrive with the body, so a
message whose body is not local has no header block to match, and ADR 0008
Q3's machinery covers this case without inventing anything:

- a rule containing `header:` runs at backfill completion for that message, not
  on arrival, through the same path a `body:` rule takes;
- the config validator emits the same note it emits for `body:` — *"runs after
  the body is fetched, not on arrival"*;
- the search bar answers over what is indexed, and the backfill status line
  tells the truth about how much that is.

Under ADR 0016 every body ends up local, so every message eventually becomes
header-searchable.

**Note the naming trap.** ADR 0008 Q3 calls the two fact classes `HEADERS_ONLY`
and `NEEDS_BODY`. `header:` is `NEEDS_BODY`. `HEADERS_ONLY` means "the fields
the *envelope* carries" — `from`, `to`, `subject`, `list`, dates, size — not
"anything that is a header". Classifying `header:` by its name produces rules
that fire on arrival against an empty header block and file mail on `false`.

**Rejected: fetching a header allowlist at header-sync time.** It would make
`header:` answerable from the moment a message is listed, at a few lines and
no extra round trip beside the `REFERENCES` and `LIST-ID` items
`fetch_headers` already asks for. It is rejected because it puts the cost on
everyone and the benefit on almost nobody: every initial sync of every mailbox
grows by a dozen header fields per message, permanently. And the allowlist it
needs is exactly the curated name list Q3 refuses.

**The escape hatch is promotion.** A header that genuinely must be matchable
before the body arrives earns a *dedicated operator*, a column, and its own
`HEADER.FIELDS` fetch. `list:` and `References` are promoted headers, and
`ARCHITECTURE.md` §6 describes the shape. `header:` is the general,
late-answering operator; promotion is how a specific one becomes early and
cheap.

### The three headers Focus promotes

`List-Unsubscribe`, `Precedence` and `Auto-Submitted` are promoted, because
Focus's filing pass reads them to tell bulk and automated mail at arrival,
before any body exists (`specs/007-postio-focus`, research R8):

- **The columns** are `messages.unsubscribe_offered` (0 or 1) and
  `messages.automation` (a bitmask of the `Precedence` and `Auto-Submitted`
  keywords, spelled out beside the column in `postio-storage`'s schema).
  NULL means not known, and a write that does not know them never erases one
  that did. `postio_model::promoted` is the one reading of the three fields.
- **The operators** are `is:bulk` (an unsubscribe offered, or any
  `Precedence` bit) and `is:automated` (any `Auto-Submitted` bit). Mail whose
  headers nothing has read yet matches neither.
- **IMAP** asks for them on incremental passes only, as one more item on the
  same `UID FETCH`:
  `BODY.PEEK[HEADER.FIELDS (LIST-UNSUBSCRIBE PRECEDENCE AUTO-SUBMITTED)]`,
  kept apart from the `REFERENCES` and `LIST-ID` items because those parsers
  read a block holding exactly one field
  (`MailBackend::fetch_headers_for_filing`). A first sync's `fetch_headers`
  does not, so the cost refused above is not paid.
- **Gmail** reads them from the metadata response every sync already
  requests, at no extra cost.
- **JMAP** does not ask: io-jmap 0.3 cannot request single headers, so the
  trait's default `fetch_headers_for_filing` answers with the facts not known.
- **Mail that arrived without them** learns them from its body's header block
  when the body arrives (`postio-index`'s `index_headers`). That covers
  first-synced mail, JMAP accounts and anything older.

## Q5 — Stores, and the message that cannot be answered locally

Nothing may silently answer "no such mail". Three populations, three
behaviours:

1. **`body_headers` present** — a local pass fills `message_headers` from it
   (`postio_session::index_local_headers` over
   `messages_missing_header_rows`): batched, yielding, resumable, background
   lane, no network (#500's pattern).
2. **`body_headers` NULL but `raw_blob_id` present** — the block is extracted
   from the raw blob and **written back** to `body_headers` before indexing
   (`MessageRepository::messages_missing_headers`). A local repair; still no
   network.
3. **Neither** — a `partial` message fetched by section, or one whose raw blob
   was evicted. A header-block fetch is enqueued in the backfill lane, under
   its policy (`messages_needing_a_header_fetch`). It is the only case that
   touches the network.

## Q6 — What the operator means

| Query | Meaning |
|---|---|
| `header:x-mailer` | the message has a field with that name |
| `header:x-mailer=mutt` | it has one whose value **contains** `mutt`, case-insensitively |
| `header:x-mailer="mutt 1.5"` | the same, with a value containing a space |
| `-header:x-mailer=mutt` | negated, like every other operator |
| `header:x-mailer=` | a half-typed query: means presence, never an error |

- **`=` separates name from value, not a second `:`.** `split_operator` splits
  at the first colon, so `header:x-mailer=mutt` arrives as the value
  `x-mailer=mutt`. Split at the first `=`; later ones belong to the value,
  which matters for `authentication-results=spf=pass`.
- **Substring, not equality.** Consistent with `from:` and `subject:`:
  `x-mailer` is `Mutt 1.5.24 (2015-08-30)`, and an equality match would never
  fire.
- **The name is matched exactly** (case-insensitively), never as a substring.
  `header:x-mail` does not match `X-Mailer`. A name from one header never
  pairs with a value from another; a normalized row makes that structural.
- **Multiple occurrences: any of them matching is a match.** `Received` chains
  and repeated `References` are why `Headers` preserves duplicates, and
  `ordinal` is why the index can too.
- **`Filter::Header { name, value: Option<String> }`** is answered by both
  evaluators. A `Filter` variant nothing answers is worse than free text,
  because free text at least finds something.

---

## Alternatives

**A contentless full-text table over `(name, value)`.** Rejected in Q2: it
needs a row→message side table to be usable at all, and its tokenizer breaks
the structured values `header:` exists to match.

**A single-column full-text index over the whole header block.** It cannot
bind a name to a value, so `header:x-mailer=mutt` matches any message with an
`X-Mailer` header and the word "mutt" in some unrelated field. A wrong answer
that looks like a right one.

**Decompress `body_headers` in Rust and match there.** It works for a rule (one
message) and collapses for a search (`header:x-mailer` alone would read the
mailbox), splits the executor into a SQL half and a Rust half, and makes the
counting harness blind to the expensive part.

**Index the headers named by the user's own `[[rules]]` and `[filters]`.**
Adding a rule would trigger a reindex of the whole mailbox, and a `header:`
typed ad hoc in the search bar would answer nothing until saved. One query
language means one string means one thing (`PRODUCT.md` §7), including when it
has never been saved.

**A `[search] indexed_headers` setting.** A question asked of every user
forever to avoid making a decision. The caps in Q3 are the decision.

---

## Consequences

- `postio-sync`'s backfill paths and `send` store the header block in
  `messages.body_headers`, and `postio-storage` persists `Message.headers`.
- `postio-index` has `message_headers`, a `headers` row in `search_schema`, a
  catch-up pass, and one compilation shape in the executor beside the
  full-text ones.
- `postio-model::headers` holds the one normalization both evaluators use.
- `postio-search` has `Field::Header` and `Filter::Header`; it stays pure.
- `search_statement_budget.rs` asserts on the join, and `header_index_size.rs`
  holds ADR 0027's ceiling.
- ADR 0008 Q3's fact classification puts `header:` on the `NEEDS_BODY` side.
- `PRODUCT.md` §7's operator list includes `header:`, `is:bulk` and
  `is:automated`.
