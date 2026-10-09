# Research: Search for Postio Focus on the Mac

Each entry records a question, the answer taken, and what was rejected.
File references are to `feature/focus-search` at 91601c9f.

## R1. What the index can do: Turso `USING fts`

**Question.** The design needs prefix completion, a vocabulary, highlight
ranges and per-column weights. What does the engine offer?

**Findings** (`~/.cargo/git/checkouts/turso-*/3d7dac1/docs/fts.md`, and
`postio-storage/tests/turso_capabilities.rs`):
- The index is tantivy behind `CREATE INDEX … USING fts (cols)`. Three
  functions: `fts_match`, `fts_score`, `fts_highlight`. No vocabulary or
  term-dictionary function, no document frequency, no `snippet()`.
- Query syntax is tantivy's: `data*` prefix, `"phrase"`, `AND`/`NOT`, and
  `word~N` fuzzy (a term *beginning* within N edits, at most 50 expansions).
  `postio_search::suggest::widened` already builds `typed*`/`typed~N`.
- `fts_score` answers 0.0 inside any arithmetic, and unless its query is the
  same parameter as the `fts_match` (`executor.rs` `HITS_JOIN` docs; two
  capability tests hold it). Any new statement must project it bare.
- `WITH (weights = 'subject=2.0,…')` exists. Not used: the executor's
  `BODY_SCORE_WEIGHT` decides the metadata/body balance in Rust, and changing
  the index definition would change GTK's ranking (S2).
- `fts_highlight` exists but works on the *stored* text; the body index holds
  the folded copy (`postio_model::fold`), so highlights stay in Rust
  (`postio_search::highlight::find`), against the text the reader shows.

**Answers.**
- *Completion (D22)*: ask `fts_match(…, 'prefix*')` on `search_documents`,
  read the best `SUGGESTION_DOCUMENTS` rows, recover words the way
  `executor::words_near` does. Metadata only for the ghost completion (names,
  subjects, file names are what people type first); bodies only when the
  metadata gives fewer than three words. Measured in step 8; a
  `search_vocabulary(term PRIMARY KEY, documents)` table maintained by the
  indexers is the fallback if p95 > 20 ms, and only then.
- *Highlight ranges*: byte ranges from `highlight::find` over the passage,
  never markers in a string. The `Highlighted`/marker form stays for GTK.

**Rejected.** An `ngram` tokenizer index for completion: a second full index
on every metadata write, which #1587 measured at ~2.3 ms per index operation.

## R2. Facets and the histogram in one pass

**Question.** How do hits, totals, seven facets and a histogram come back
under 50 ms on 20k messages, when today's facets are a second read of up to
five statements (`executor::facets`, `Plan::current_scope`)?

**Answer.** One *match projection*: the capped match (`Plan::build`'s
condition, the same one `count` uses) joined to `messages` and projecting
narrow columns only: `id, thread_id, mailbox_id, received_at, seen, flagged,
answered, has_attachments, sender address id, open-marker exists`. Rows
stream into a Rust fold that builds, in one walk:
- conversations (key: `thread_id`, else the message id) with best score,
  newest matching message, match count, unread/flagged/answered;
- counts per sender, folder, attachment, unread, has-action, month;
- the candidate pool for ranking.

The To facet, the Label facet and the People and Files counts need rows of
`recipients`, `message_labels` and `attachments` for every matched message,
not only the page's. They ride in the same projection as correlated
`group_concat` subqueries (`recipient address ids`, `label ids`, `attachment
count`), each a primary-key range lookup per row, so the statement count stays
fixed while the per-row cost is measured. A fixed statement count: **≤ 5**
(count, projection, the attachment-text arm, the page's per-column match
check, the page's hydrate), asserted in
`index_suite/search_statement_budget.rs`. If the correlated subqueries are
what pushes the bench over, the alternative is a keyed second walk per table
over the matched ids; that is a measured choice in step 1, not a guess now.

Ranking stays the executor's: past `RANK_BY_RELEVANCE_LIMIT` the projection
orders by recency, as `search_as` does; below it, by score.

**Risk and the stop rule.** A query matching ~10k messages streams 10k
narrow rows. If the step-1 bench shows p95 > 50 ms for the common-word shape,
work stops and the maintainer is asked (FR-062). Options to put to them then:
cap the projection lower than `TOTAL_HITS_CAP` for facets only (facets as
floors), or answer facets one frame after hits.

**Rejected.** Seven `GROUP BY` statements: seven walks of the match. A
single `GROUP BY` with grouping sets: the engine has none. A temp table of
matched ids: not proven on this engine, and a write on a read path.

## R3. Conversation ranking and rank reasons

**Answer.** Messages are scored by `rank_score` as today, then grouped: a
conversation's score is its best message's; ties broken by match count, then
recency. Reasons (D20) come from the fold: `answered` on any matched message
→ Replied; `flagged` → Flagged; two-way frequency (R6) ≥ the finder's
"frequent" threshold → FrequentSender; the metadata arm matched `subject` →
InSubject; matched `filenames` or attachment text → InFileName; Matches(n).
Which column matched is learnt per candidate with one `fts_match` per column
on the page's ≤ 50 ids, not per row of the match.

## R4. Passages, sources and quoted text

**Answer.** `postio_session::search` cuts passages, as `snippet_hits` cuts
excerpts, because it can read bodies and the index cannot (#408):
1. `indexable_text(body)`: the text the index holds;
2. `postio_body::quote::text_stretches(text)`: which byte ranges are quoted
   history (D6);
3. `postio_search::passage::cut(text, terms, first_line)`: the first match
   *outside* the first line, a ~120-character window snapped to word edges,
   with ellipsis flags and highlight ranges relative to the window (D7).
A match inside a quoted stretch is `Source::Quoted`; subject and file-name
matches come from the metadata row; attachment matches from
`attachment_passages` (R5), with their `Location`.

Passages are a read of their own (`Req::Passages`) for the visible page, so
the list's first frame never waits for a body decode (#1613).

## R5. Attachment text: crates and shape

**Question.** Which extraction crates are pure Rust and MIT-compatible?

**Survey (crates.io, 2026-10-08).** None of these is in `Cargo.lock` today
(it has `flate2`, `miniz_oxide`, `quick-xml` 0.41 via `wayland-scanner`,
`encoding_rs`, `nom`). Pimalaya has no document-extraction crate.

| Crate | Version | Licence | Native code |
|---|---|---|---|
| `pdf-extract` | 0.12.1 | MIT | none; depends on `lopdf ^0.42`, `adobe-cmap-parser` (MIT), `cff-parser` (MIT/Apache), `postscript` (MIT/Apache), `type1-encoding-parser` (MIT), `euclid`, `encoding_rs`, `unicode-normalization` |
| `lopdf` | 0.45.0 (0.42 through pdf-extract) | MIT | none; `flate2`, `weezl`, `brotli-decompressor` (BSD-3/MIT), RustCrypto `aes`/`cbc`/`md-5`/`sha2` for encrypted files |
| `zip` | 8.6.0 | MIT | none with `default-features = false, features = ["deflate-flate2"]` (flate2's Rust backend). Defaults pull bzip2/zstd/lzma/aes, not wanted |
| `quick-xml` | 0.42.0 | MIT | none |

Every licence above is in `deny.toml`'s allow list.

**Answer.**
- **PDF**: `pdf-extract`, per page (`extract_text_from_mem_by_pages`), which
  gives `Location::Page(n)`. Encrypted-with-a-password PDFs are recorded as
  `Skipped(Encrypted)`.
- **OOXML**: `zip` + `quick-xml`, with three small walkers written here:
  DOCX `word/document.xml` (`w:p`/`w:t` → `Location::Paragraph(n)`), XLSX
  `xl/workbook.xml` + `xl/sharedStrings.xml` + `xl/worksheets/sheetN.xml`
  (`Location::Sheet{name, row}`), PPTX `ppt/slides/slideN.xml`
  (`Location::Slide(n)`). Rejected `calamine` (MIT, pure Rust): it reads
  values and formulas into typed cells, more than needed, and a second XML
  stack beside `quick-xml`.
- **Text**: `text/*` decoded with `encoding_rs` from the part's charset,
  `Location::Line(n)`.
- **Limits**: 25 MB in, 1 MB of text out, 5,000 units, 2 s per file, zip
  entries ≤ 50 MB uncompressed and a ratio ≤ 100 (zip bombs). Over a limit:
  what was read is kept and the row says `Truncated`.
- **Panics**: `pdf-extract` panics on some malformed files. Extraction runs
  on a blocking thread under `catch_unwind` (the Mac's release profile
  unwinds; only `release-tui` aborts). A stack overflow cannot be caught: if
  the corpus fixture of hostile files shows one, extraction moves to a child
  process (`postio-extract` as a binary), and that is reported, not hidden.

**Shape.** A new crate `postio-extract`: `extract(bytes, mime, name,
&Limits) -> Extracted { units: Vec<Unit{location, text}>, outcome }`. Pure,
synchronous, no store, no runtime. The host's indexer (`postio-session`)
reads the blob, calls it on `spawn_blocking`, folds each unit, and writes
`attachment_passages`. Only attachments with `blob_id IS NOT NULL` (D10).

## R6. Two-way people frequency

**Answer.** `contacts.times_seen` counts mail *from* an address;
`correspondents.sent_count` counts mail *to* it (spec 007). People rank by
`times_seen + sent_count`, ties by the later of `last_seen_at` and
`last_sent_at`. One statement, prefix-matched on name and address. No new
column.

## R7. Relaxations

**Answer.** Pure in `postio_search::relax`: for each complete clause of a
query, one variant with it dropped; for `subject:v`, one with `v` as free
text (anywhere); for `label:v`, one with `in:v`. Free-text words are dropped
one at a time only when the query has two or more. The host counts each
variant with the projection's `count` (one statement each, at most 8
variants, so ≤ 8 statements, asserted). Zero-count variants are dropped,
the rest sorted by count, first four offered (FR-030).

## R8. Typing and cancellation

**Findings.** `focus_bar_typed` is synchronous and cheap; the controller
stamps each answer and drops stale ones (`bar.rs` `answer`). The FFI driver
spawns every `Request` on tokio (`focus_list.rs` `ask`) with no handle kept.
The host answers each read on its own runtime (`postio-host/src/lib.rs`
`Local::call`), so dropping the caller cancels nothing.

**Answer (D8, D9).** `Request::lane() -> Option<Lane>` names the search lanes
(`Dropdown`, `Results`, `Passages`, `Suggest`, `Preview`). The driver keeps
`HashMap<Lane, AbortHandle>` and aborts the previous task of a lane before
spawning the next. In the host, the new search requests are answered as
`select! { _ = answer.closed() => {}, r = inner.answer(..) => answer.send(r) }`.
A read-only turso statement dropped mid-step returns its connection to the
pool; a capability test proves it before anything relies on it. No debounce
unless the step-1 bench shows keystrokes queueing.

## R9. History and the results mode

**Answer.** Two new controller modules: `history.rs`, a bounded back/forward
list of `Entry::{Inbox{cursor, selection}, Results{query, tab, sort, cursor,
selection}}` (50 entries), and `results.rs`, the mode itself. The list
already pages through `ListWindow`; results get their own window over the
conversation hits, paged by the same feed rules. Esc's ladder (D18) is the
surface stack first (popover, dropdown), then the results' own rungs
(selection, then leave). Gated by `Policy.caps.results_view`, so the GTK
controller never enters it.

## R10. Keys

**Answer.** New `CommandId`s in `postio-core/src/registry.rs`:
`ShowAllResults` (`mod+Return`, Search), `HistoryBack` (`mod+bracketleft`),
`HistoryForward` (`mod+bracketright`), `ResultsConversations`/`Files`/`People`
(`mod+1`–`3`), `QuickLook` (`space`), `NextMatch`/`PrevMatch` (`]`/`[`, Quick
Look), `ExcludeSuggestion` (`alt+Return`, Search), `ForgetRecent`
(`alt+BackSpace`, Search, D23), `StepRangeBack`/`Forward` (`alt+Left`/`Right`,
Results), `PickRelaxation1`–`4` (`1`–`4`, Results), `SaveFile` (`mod+Down`,
Files). A new `Context::Results` that the list's verbs also name. All are
`offered_on(.., Freedesktop) = false` until Linux adopts (D25), so
`docs/keybindings.md`'s Linux rows do not change; its regeneration test
runs. `SaveSearch` gains the Results context.

## R11. Where Swift draws what

**Answer**, following 009's split (#1264: PostioKit has no AppKit):

| PostioAppKit (AppKit) | PostioKit (SwiftUI and models) |
|---|---|
| `ChipQueryField` (an `NSTextView`-based field with attachment chips; `NSTokenField` cannot strike through or hold free words between chips), the 860-wide panel (`CommandBarPanel`), the results `NSTableView` with group rows, the Files `NSCollectionView`, filter `NSPopover`s, the Quick Look floating `NSPanel`, `QLPreviewPanel` | dropdown content, filter bar and tabs, timeline (drag), popover contents, Quick Look body, Save popover, no-results page, People list, footer, and the view models they read (`SearchQueryModel`, `DropdownModel`, `ResultsModel`) |

Swift decides nothing but the arrow highlight in the dropdown (009 FR-004)
and the timeline's drag gesture, whose end it reports as two months.
