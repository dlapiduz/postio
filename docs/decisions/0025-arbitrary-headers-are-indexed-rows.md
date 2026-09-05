# ADR 0025 — Arbitrary headers are stored on the row and indexed as rows, not as text

- **Status:** Accepted (2026-09-03), **Q2's "metadata scale" and Q3's budget amended by [ADR 0027](0027-the-header-index-is-budgeted-per-message.md) (2026-09-04)**
- **Date:** 2026-09-03
- **Decision by:** a `/ux-architect` session, on the question
  [#884](https://github.com/dlapiduz/postio/issues/884) raised: `header:` has
  nowhere to match against, and choosing where is a storage decision.
- **Issue:** [#884](https://github.com/dlapiduz/postio/issues/884)
- **Related:** [ADR 0008](0008-filters-and-rules.md) Q2 (`header:` is one
  `Field` row) and Q3 (a rule fires when every fact it needs exists),
  [ADR 0016](0016-full-mailbox-backfill-by-default.md) (every body ends up
  local), [ADR 0020](0020-where-message-bodies-live.md) (`body_headers` and
  the trained dictionary), [ADR 0014](0014-encryption-at-rest.md),
  `PRODUCT.md` §6 (what is stored locally), §7 (search),
  [#478](https://github.com/dlapiduz/postio/issues/478) (the language slice),
  [#479](https://github.com/dlapiduz/postio/issues/479) (the matcher and the
  differential test), [#480](https://github.com/dlapiduz/postio/issues/480)
  (`[[rules]]`)
- **Decision:** **the header block is persisted to `messages.body_headers`,
  which the schema already has and nothing has ever written; a normalized
  `message_headers(message_id, name, value, ordinal)` table in
  `postio-index`'s schema is what `header:` matches against; and *every*
  header is indexed, bounded by two structural caps rather than by any list of
  header names.** `header:` is a body-class fact, answerable exactly when
  `body:` is, and a header that must be answerable earlier earns a dedicated
  operator instead.

---

## Three things in the tree that change the answer

#884 frames the choice as "normalized content table vs contentless FTS". Both
options assume facts that are not true today, and reading the code first moves
the decision.

**1. `messages.body_headers` is always NULL.** The column exists, is
zstd-compressed against the trained dictionary, and is covered by SQLCipher —
and *nothing in the workspace writes it*. Both backfill paths construct
`StoredBody { headers: None, .. }` on purpose
(`crates/postio-sync/src/backfill.rs:1108`, `:1299`), with the reason stated at
the call site:

```rust
// The header block has no reader of its own yet: everything that wants
// headers has the row, and everything that wants all of them has the
// raw blob. A copy nobody reads is a copy that can go stale.
headers: None,
```

That reasoning was right when it was written and this ADR is what retires it:
`header:` is the reader. So the question is not only "where does the index
live" but "where does the *data* live", and the answer to the second one is
already in the schema, already compressed, already encrypted.

**2. The raw blob cannot be the source of truth.** `fetch_body` stores
`raw_blob_id`, so the full header block *is* recoverable from it — but
`PRODUCT.md` §6 says the store "evicts what it can refetch — **raw source
first**". A `header:` answered out of the raw blob stops working the first time
the store hits its size limit, silently, on the oldest mail. And the section
path (`fetch_parts`, the `partial` state that ADR 0017 says is 15% of the
reference mailbox) never fetches a raw blob at all.

**3. `Message.headers` exists in memory and is never persisted.**
`postio_model::Headers` already preserves wire order, duplicates and
case-insensitive lookup; `ParsedMessage::into_message` fills it
(`mime.rs:206`). `MessageRepository` never reads or writes it, so a `Message`
loaded from the store always has an empty header block. The in-memory matcher
#479 will build therefore already has the right input shape on arrival and
nothing on reload — which is precisely the asymmetry the differential test
exists to catch.

---

## Q1 — Where the header text lives: `messages.body_headers`

**Decision: the sync paths stop passing `None` and store the block.**

It is the cheapest place available. Bodies pay for the dictionary already, and
a header block is the most compressible text in a mailbox — the same `Received`
boilerplate, the same `Content-Type` lines, the same DKIM field names, on every
message from a given provider. ADR 0020 measured 2.19x on bodies; headers
should do better, and the size test below is what says so.

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
`metadata` and `bodies`.** Not an FTS table.

Three reasons, in order of weight.

**FTS tokenization destroys exactly the values people match on.** `header:` is
wanted for `x-mailer=mutt`, `x-spam-status=fail`, `authentication-results=spf=pass`,
`content-type=multipart/signed`, `precedence=bulk`. A `unicode61` tokenizer
splits `spf=pass` and `1.5.24` and `<list.example.com>` into pieces and loses
the adjacency that made them meaningful. Header values are short, structured
and matched by substring; that is a `LIKE` on a column, not an inverted index.

**A contentless FTS table cannot say which message a match belongs to.**
`message_bodies_fts` gets away with `content = ''` because its rowid *is* the
message id. One FTS row per header cannot do that — a message has many headers
— so the rowid has to be a header id, and mapping it back to a message needs a
side table holding `(rowid, message_id)`. That side table is a content table
with the text removed; having built it, the honest thing is to put the value in
it and delete the FTS table.

**A content table here is not the duplication ADR 0016 refused.**
`search_documents` already holds plaintext `sender`, `recipients`, `subject`,
`filenames` and `list_id` — the project's settled position is that
*metadata*-scale duplication is fine and *corpus*-scale duplication is not.
What made `search_documents.body` unacceptable was that it was the entire text
of every message, a second time, on the pages ADR 0014 encrypts under a 100 ms
budget. Headers under Q3's caps are metadata scale. The size test in the
acceptance criteria is what holds that claim to account rather than assuming
it.

> **"Metadata scale" is wrong, and
> [ADR 0027](0027-the-header-index-is-budgeted-per-message.md) Q4 replaces it
> (2026-09-04).** Measured, `message_headers` and its index cost 3,809 bytes a
> message where `search_documents` and `messages_fts` together cost 184 — it is
> seventeen times the metadata half of the index it sits in. The conclusion
> survives on a better reason than the one given here: what made
> `search_documents.body` unacceptable was that it was *unbounded*, and a
> message's header rows cannot exceed 64 x 512 bytes however pathological the
> mail. Boundedness, not smallness, is what the size test should have been
> holding to account.

**It belongs to `postio-index`, not `postio-storage`.** It is derived data with
a local generator: droppable, rebuildable from `body_headers` with no network,
versioned by a `headers` row in `search_schema` on exactly the terms
`BODIES_SCHEMA_VERSION` documents. A bump drops the table; the catch-up pass in
Q5 refills it in the background. That is the whole mechanism by which the
policy in Q3 stays revisable.

Shape and indexes:

```sql
CREATE TABLE IF NOT EXISTS message_headers (
    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,   -- lowercased; RFC 5322 names are case-insensitive
    value      TEXT    NOT NULL,   -- unfolded, RFC 2047-decoded, truncated per Q3
    ordinal    INTEGER NOT NULL,   -- occurrence index within the message, wire order
    PRIMARY KEY (message_id, name, ordinal)
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS idx_message_headers_name ON message_headers (name, message_id);
```

`ON DELETE CASCADE` rather than a trigger: unlike `message_bodies_fts`, this is
an ordinary table and the foreign key does the work `trg_message_bodies_fts_ad`
had to be written by hand to do.

`header:name` compiles to `EXISTS (SELECT 1 FROM message_headers WHERE
message_id = m.id AND name = ?)`; `header:name=value` adds
`AND value LIKE '%' || ? || '%'`. Both are narrowed by `name` first, and
`idx_message_headers_name` is what makes that a range scan over one name rather
than the table. This is a different compilation shape from every other operator
— the rest become an FTS `MATCH` (`fts_column_condition`) — and
`search_statement_budget.rs` must gain an assertion for it, because a new join
on the read path is exactly what the counting harness exists to bound.

## Q3 — Which headers: all of them, bounded by structure, never by a name list

**Decision: every header is indexed, one row per occurrence, subject to two
caps. There is no allowlist and no denylist of header names.**

| Cap | Value | What it removes |
|---|---|---|
| Value length | **512 bytes**, truncated | DKIM/ARC signatures, long spam reports — high-entropy blobs nobody substring-matches |
| Rows per message | **64**, in wire order | The pathological block; a long `Received` chain past the 64th field |

#884 is right that `Received` chains are most of the cost for the least-wanted
match, and a name list is still the wrong instrument.

A curated list of header names is a list somebody maintains forever, and every
name off it makes `header:x-whatever` answer "no such mail" — which is not "we
did not index that", it is a **lie**, and the search bar has no way to say
otherwise. Worse, a list of the headers worth indexing converges on `X-GM-*`,
`X-MS-Exchange-*`, `X-Google-*`: named constants for particular providers in the
one part of the code `PRODUCT.md` §3 is most explicit about. Postio is not an
iCloud client and it is not a Gmail client, and a hard-coded list of Google's
header names in `postio-index` would say otherwise in the most durable way
available.

Structural caps have neither problem. They are provider-neutral, they need no
maintenance, they bound the cost predictably, and what they exclude is
excluded for a stated reason a user can reason about — a 900-byte base64
signature is not something anyone was going to find a substring in.

**Truncation is a correctness hazard, not just a cost knob.** The in-memory
matcher holds the full value; the index holds a 512-byte prefix. They will
disagree on any long header unless the matcher applies the *identical*
normalization before comparing. So normalization — lowercase the name, unfold
(RFC 5322 §2.2.3), decode encoded words (RFC 2047), collapse whitespace,
truncate to 512 bytes — is **one function in `postio-model::headers`**, which
both `postio-index` and `postio-search` already depend on, and neither
evaluator is permitted its own. This is the first thing #479's differential
test should be pointed at.

**The budget, and what happens if it is missed.** The size test required by
#884 measures `message_headers` against `message_bodies_fts` on the same
corpus with `dbstat`, exactly as `body_index_size.rs` does. The budget is
**≤ 25% of what the body index costs on the same corpus**. A relative figure
rather than an absolute one, because it is the ratio that stays true on another
machine and another mailbox. If the measurement misses it, the lever is the two
caps — not a list of names, and not shipping it anyway.

> **Superseded by
> [ADR 0027](0027-the-header-index-is-budgeted-per-message.md) (2026-09-04),
> and wrong in a way worth leaving visible.** The instinct — relative rather
> than absolute — is this project's own, and the wrong relative figure was
> picked: `message_bodies_fts` is `content = ''` and holds no text at all,
> while `message_headers` holds every value verbatim, so the ratio measures how
> well FTS5 compresses the fixture's *bodies* and charges the answer to
> headers. It moves for reasons the header policy does not control — a corpus
> of short mail measured 269% where the same policy measured 107% on long mail
> — and the two caps reach about half the cost where the target needed a
> twentieth. #1041 is the measurement; ADR 0027 replaces the budget with a
> ceiling of 5 KiB a message over `message_headers` and
> `idx_message_headers_name`, and leaves the operator and both caps exactly as
> this ADR specifies them.

## Q4 — When `header:` can be answered: exactly when `body:` can

**Decision: `header:` is a body-class fact.** Headers arrive with the body, so a
message whose body is not local has no header block to match, and ADR 0008 Q3's
machinery already covers this case without inventing anything:

- a rule containing `header:` runs at backfill completion for that message, not
  on arrival, through the same path a `body:` rule takes;
- the config validator emits the same note it emits for `body:` — *"runs after
  the body is fetched, not on arrival"*;
- the search bar answers over what is indexed, and the backfill status line
  already tells the truth about how much that is.

Under ADR 0016 every body ends up local, so "eventually every message is
header-searchable" is a promise the product already makes for `body:` and now
makes here.

**Note the naming trap.** ADR 0008 Q3 calls the two fact classes `HEADERS_ONLY`
and `NEEDS_BODY`. `header:` is `NEEDS_BODY`. `HEADERS_ONLY` means "the fields
the *envelope* carries" — `from`, `to`, `subject`, `list`, dates, size — not
"anything that is a header". An implementer who classifies `header:` by its
name will produce rules that fire on arrival against an empty header block and
file mail on `false`, which is the failure mode ADR 0008 Q3 was written to
prevent.

**Rejected: fetching a header allowlist at header-sync time.** It is
tempting and it is already precedented — `fetch_headers` issues
`BODY.PEEK[HEADER.FIELDS (REFERENCES)]` and `(LIST-ID)` beside `ENVELOPE`
today (`crates/postio-account/src/imap/fetch.rs:191-227`), so adding a dozen
more field names is a few lines and no extra round trip. It would make
`header:` answerable from the moment a message is listed, and header-only
rules could fire on arrival.

It is rejected because it puts the cost on everyone and the benefit on almost
nobody: every user's initial sync of every mailbox grows by the wire bytes of
a dozen header fields per message, permanently, so that the few who write a
`header:` rule get it a little sooner. And the allowlist it needs is exactly
the curated, provider-flavoured name list Q3 refuses.

**The escape hatch is promotion, and it already exists.** A header that
genuinely must be matchable before the body arrives earns a *dedicated
operator*, a column, and its own `HEADER.FIELDS` fetch. That is not a
hypothetical: `list:` is a header that was promoted, `References` is another,
and `ARCHITECTURE.md` §6 already describes the shape. `header:` is the general,
late-answering operator; promotion is how a specific one becomes early and
cheap. Keeping those two paths distinct is what stops `header:` from becoming
a reason to enlarge every sync.

## Q5 — Existing stores, and the store that cannot answer locally

Nothing may silently answer "no such mail". Three populations, three
behaviours:

1. **`body_headers` present** — a local pass fills `message_headers` from it.
   Shaped like `messages_missing_body_text` / `index_local_bodies`: batched,
   yielding, resumable, background lane, no network (#500's pattern).
2. **`body_headers` NULL but `raw_blob_id` present** — every store that exists
   today. The pass extracts the block from the raw blob and **writes it back**
   to `body_headers` before indexing. A local repair; still no network.
3. **Neither** — a `partial` message fetched by section, or one whose raw blob
   was evicted. The pass enqueues a header-block fetch in the existing backfill
   lane, under the existing policy. It is the same lane that fetched the body,
   throttled the same way, and it is the only case that touches the network.

## Q6 — What the operator means

Settled here so that two evaluators and one parser cannot each decide
separately:

| Query | Meaning |
|---|---|
| `header:x-mailer` | the message has a field with that name |
| `header:x-mailer=mutt` | it has one whose value **contains** `mutt`, case-insensitively |
| `header:x-mailer="mutt 1.5"` | the same, with a value containing a space |
| `-header:x-mailer=mutt` | negated, like every other operator |
| `header:x-mailer=` | a half-typed query: means presence, never an error |

- **`=` separates name from value, not a second `:`.** `split_operator` already
  splits at the first colon, so `header:x-mailer=mutt` arrives as the value
  `x-mailer=mutt` and this parses cleanly with no change to the tokenizer. It
  is also ADR 0008 Q2's spelling. Split at the first `=`; later ones belong to
  the value, which matters for `authentication-results=spf=pass`.
- **Substring, not equality.** Consistent with `from:` and `subject:`, and
  demanded by the motivating example — `x-mailer` is `Mutt 1.5.24 (2015-08-30)`
  and an equality match would never fire. Sieve's `:is`/`:contains`
  distinction is not worth a second syntax here.
- **The name is matched exactly** (case-insensitively), never as a substring.
  `header:x-mail` does not match `X-Mailer`. This is the binding #884's
  acceptance criterion is about: a name from one header must never pair with a
  value from another, and a normalized row makes that structural rather than
  something a test has to hope for.
- **Multiple occurrences: any of them matching is a match.** `Received` chains
  and repeated `References` are why `Headers` preserves duplicates, and
  `ordinal` is why the index can too.
- **Do not land the parser without the executor.** #884 says it and it is the
  same defect `check-uncalled-pub-fn.py` was added for in #421: a `Filter`
  variant nothing answers is worse than free text, because free text at least
  finds something. `Filter::Header { name, value: Option<String> }`,
  both evaluators, one commit.

---

## Alternatives

**A contentless FTS table over `(name, value)`.** The shape #884 leans toward,
and the one that matches `message_bodies_fts`. Rejected in Q2: it needs a
rowid→message side table to be usable at all, at which point it is a content
table with the text taken out, and its tokenizer breaks the structured values
`header:` exists to match.

**A single-column FTS index over the whole header block.** Cheapest of all and
the reason the issue exists: it cannot bind a name to a value, so
`header:x-mailer=mutt` matches any message with an `X-Mailer` header and the
word "mutt" in some unrelated field. A wrong answer that looks like a right one.

**Decompress `body_headers` in Rust and match there.** No index at all: SQL
narrows on the other clauses, Rust decompresses and matches the survivors. It
works for a rule (one message) and collapses for a search (`header:x-mailer` on
its own would decompress the mailbox). It also splits the executor into a
SQL half and a Rust half, which is a much larger architectural change than an
index, and would make the counting harness blind to the expensive part.

**Index the headers named by the user's own `[[rules]]` and `[filters]`.**
Genuinely appealing — "did the user ask for it" as an indexing policy, and it
makes the cost proportional to use. Rejected: adding a rule would trigger a
reindex of the whole mailbox, and a `header:` typed ad hoc in the search bar
would answer nothing until it had been saved somewhere, so the search bar's
answer would depend on config the user was not thinking about. One query
language means one string means one thing (`PRODUCT.md` §7), including when it
has never been saved.

**A `[search] indexed_headers` setting.** A question asked of every user
forever to avoid making a decision, and the answer for every user who never
opens settings is the wrong one. The caps in Q3 are the decision that setting
would have been avoiding.

---

## Consequences

- `postio-sync`'s two backfill paths and `send` stop passing `headers: None`;
  `messages.body_headers` starts holding data. No migration: the column,
  the compression and the dictionary reference are already there.
- `postio-storage` gains header persistence on `Message.headers`, which has
  been parsed and dropped on the floor since the model was written.
- `postio-index` gains `message_headers`, a `headers` row in `search_schema`,
  a catch-up pass, and one new compilation shape in the executor.
- `postio-model::headers` gains the single normalization function both
  evaluators are required to use.
- `postio-search` gains `Field::Header` and
  `Filter::Header { name, value: Option<String> }`; it stays pure.
- `search_statement_budget.rs` gains an assertion for the new join;
  a `header_index_size.rs` measurement is added beside `body_index_size.rs`,
  with the ≤ 25%-of-the-body-index budget from Q3.
- ADR 0008 Q2's `header:` row is now specified rather than named, and its Q3
  fact classification gains `header:` on the `NEEDS_BODY` side.
- `PRODUCT.md` §7's operator list gains `header:`.
