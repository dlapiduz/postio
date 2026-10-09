# Data model: Search for Postio Focus on the Mac

Rust types by crate, then the new tables. Existing types are named as they
are on `feature/focus-search`; everything else is new. FFI mirrors are in
[contracts/ffi-search.md](contracts/ffi-search.md).

**A word on "term".** The design's *term* is the code's `query::Token` (a
span, its raw text and a `TokenKind`). `postio_search::suggest::Term` already
exists and means a vocabulary word with a document count; it keeps that
meaning. No new `Term` type is added.

## `postio-search` (pure)

### Operators (`query.rs`, `parser.rs`)

| Change | Type | Notes |
|---|---|---|
| `Field::Label` | keyword `label` | takes free text, quoted when it has spaces |
| `Filter::Label(String)` | the label's name, unresolved | resolved by the executor against `labels.name`, case-insensitive, per account (as `Account`/`Group` are) |
| `Filter::HasAction` | `has:action` | an open marker: a `markers` row with `dismissed_at IS NULL` |
| `parser` `has:` values | `action`, `actions` → `HasAction`; `act…` → `Partial` | beside today's `attach`/`file` forms |
| `query::spell(&Clause) -> String` | the canonical text of a clause (D13) | `has:attachment`, `label:"Q3 close"`, ISO dates; round-trips through `parse` |

`matcher::Matcher::new` returns `Unsupported::Token` for `label:` and
`has:action` (D12).

### Term edits (`edit.rs`, new)

```rust
pub enum Edit {
    Add(Clause),                 // appended, spelled by `spell`
    Remove { token: usize },     // by index into `ParsedQuery::tokens`
    Replace { token: usize, with: Clause },
    Toggle(Filter),              // add if absent, remove every positive clause of it if present
    SetDates { after: Option<NaiveDate>, before: Option<NaiveDate> }, // replaces every after:/before:
    ClearFilters,                // keep free text, drop every clause and partial (D24)
}
pub fn apply(query: &str, edit: Edit, today: NaiveDate) -> String;
```

Rules: tokens keep their order and raw text except the one edited; a token
the user typed is edited in place, never duplicated; the result is
whitespace-normalised. `facets::append` stays for GTK.

### Plain English with origins (`natural.rs`)

```rust
pub struct Lowered {
    pub query: ParsedQuery,        // == natural::lower(text, today, names)
    pub origins: Vec<Origin>,      // one per token of `query`, same order
}
pub struct Origin {
    pub token: usize,              // index into query.tokens()
    pub from: Option<Span>,        // byte span in the English text; None for a word kept verbatim
    pub words: String,             // the English, as typed: "last month"
}
pub fn lower_with_origins(text: &str, today: NaiveDate, names: &dyn Fn(&str) -> Option<String>) -> Lowered;
```

`lower` becomes `lower_with_origins(..).query`; its tests do not change.

### Relaxations (`relax.rs`, new)

```rust
pub enum Loosen {
    Drop { token: usize },          // "Remove “before March”", "Anyone, not just Ada Moreno"
    Anywhere { token: usize },      // subject:v → v                      ("Look for “v” anywhere…")
    FolderNotLabel { token: usize },// label:v → in:v
}
pub struct Relaxation { pub loosen: Loosen, pub query: String }
pub fn relax(query: &ParsedQuery, today: NaiveDate) -> Vec<Relaxation>; // ≤ 8, in token order
```

The count and the sentence are not here: counts are the executor's, words
are `postio-ui`'s.

### Matches and passages (`results.rs`, `passage.rs` new)

```rust
pub enum Source {
    Subject,
    Body,
    Quoted,
    FileName { attachment: AttachmentId, name: String },
    FileContent { attachment: AttachmentId, name: String, location: Location },
}
pub enum Location {
    Page(u32),                         // PDF, 1-based
    Sheet { name: String, row: u32 },  // XLSX, 1-based row
    Slide(u32),                        // PPTX
    Paragraph(u32),                    // DOCX
    Line(u32),                         // text/*
    Table { index: u32, row: u32 },    // reserved: tables in documents
    ImageText,                         // reserved: OCR, later
}
pub struct Passage {
    pub text: String,                  // ~120 chars, snapped to word edges
    pub ranges: Vec<Range<usize>>,     // byte ranges into `text` to highlight
    pub elided_start: bool,
    pub elided_end: bool,
}
pub struct Match { pub source: Source, pub passage: Option<Passage>, pub when: Option<DateTime<Utc>> }

pub fn passage::cut(text: &str, terms: &[String], skip_first_line: bool) -> Option<Passage>;
```

`AttachmentId` already exists (`postio_model::ids`). `AddressId`
(`addresses.id`) is a new newtype beside it, for facet keys.

### Ranking (`results.rs`)

```rust
pub enum RankReason { Replied, Flagged, FrequentSender, InSubject, InFileName, Matches(u32) }

pub struct ConversationHit {
    pub key: ConversationKey,          // Thread(ThreadId) | Lone(MessageId)
    pub best: MessageId,               // the message the row shows and opens
    pub mailbox_id: MailboxId,         // the best message's folder
    pub subject: Option<String>,
    pub from: Option<EmailAddress>,
    pub newest_match: DateTime<Utc>,   // decides the month group (D4)
    pub messages: u32,                 // the thread count badge
    pub unread: bool,
    pub has_attachments: bool,
    pub labels: Vec<LabelId>,
    pub score: f64,                    // lower is better, as SearchHit
    pub reasons: Vec<RankReason>,      // Matches(n) always last
    pub matches: Vec<Match>,           // sources known now; passages filled by Req::Passages
}

pub struct ConversationResults {
    pub hits: Vec<ConversationHit>,    // this page
    pub total: u64,                    // conversations, capped
    pub capped: bool,
    pub messages_searched: u64,        // "Searched all 18,204 messages"
    pub corpus_complete: bool,         // bodies (existing meaning)
    pub contents_complete: bool,       // every downloaded attachment extracted
    pub facets: SearchFacets,
    pub files: u64,
    pub people: u64,
    pub elapsed: Duration,
}
pub enum ConversationOrder { BestMatch, Newest }   // maps onto ResultOrder
pub enum ResultsTab { Conversations, Files, People }
```

### Facets (`facets.rs`)

`facets::Facets` (scopes and refinements) stays for GTK. New beside it:

```rust
pub struct SearchFacets {
    pub senders: Vec<Count<AddressId>>,     // top 50 by count
    pub recipients: Vec<Count<AddressId>>,  // top 50
    pub labels: Vec<Count<LabelId>>,
    pub folders: Vec<Count<MailboxId>>,
    pub attachment: u64,
    pub action: u64,
    pub unread: u64,
    pub months: [MonthCount; 12],           // oldest first, ending with today's month
    pub presets: [u64; 5],                  // Date popover: last 7 days, last 30 days, this quarter, this year, any time
    pub capped: bool,                       // every count is a floor
}
pub struct Count<T> { pub id: T, pub conversations: u64 }
pub struct MonthCount { pub month: NaiveDate /* first day */, pub conversations: u64 }
```

Names and addresses for display come with the hydrate, not with each
count: `postio-session` resolves the ids of the counts it returns (≤ 50 a
facet) in one read per facet kind.

The results' ⇧X aims at a predicate. `postio_focus::verbs::Everything` is
over the inbox today; it gains a query form, `Aim::Matching { query:
String, except: Vec<MessageId> }`, which the host resolves with the same
match `search_conversations` walks.

### Suggestions (`suggest.rs`)

```rust
pub struct Suggestions {
    pub ghost: Option<String>,          // the rest of the best word: "las" after "at"
    pub words: Vec<Completion>,         // vocabulary words
    pub labels: Vec<Completion>,
    pub lists: Vec<Completion>,
    pub files: Vec<Completion>,
    pub people: Vec<Person>,
}
pub struct Completion { pub text: String, pub query: String, pub count: u64 }
pub struct Person {
    pub name: Option<String>, pub address: String,
    pub received: u64, pub sent: u64,   // two-way (D21)
    pub last: Option<DateTime<Utc>>,
}
pub fn rank_words(prefix: &str, vocabulary: impl Iterator<Item = Term<'_>>) -> Vec<Completion>;
```

## `postio-index`

```rust
pub struct ConversationRequest<'a> {
    pub account: AccountScope,
    pub query: &'a ParsedQuery,
    pub order: ConversationOrder,
    pub offset: u32,
    pub limit: u32,              // 4 for the dropdown, a page for results
    pub today: NaiveDate,        // months (D4)
}
pub async fn executor::search_conversations(&Connection, &ConversationRequest<'_>, now: DateTime<Utc>) -> Result<ConversationResults>;
pub async fn executor::relaxation_counts(&Connection, AccountScope, &[Relaxation], today) -> Result<Vec<u64>>;
pub async fn executor::completions(&Connection, AccountScope, prefix: &str, field: Option<Field>) -> Result<Suggestions>;
pub async fn executor::files(&Connection, &ConversationRequest<'_>) -> Result<Vec<FileHit>>;
pub async fn executor::people(&Connection, &ConversationRequest<'_>) -> Result<Vec<Person>>;
pub async fn index::index_attachment_text(&Connection, AttachmentId, MessageId, &Extracted) -> Result<()>;
pub async fn index::attachments_missing_text(&Connection, limit: u32) -> Result<Vec<(AttachmentId, MessageId, String /*blob*/, String /*mime*/, Option<String>)>>;
```

`filter_condition` gains `Filter::Label` and `Filter::HasAction`, used by
both `search` (GTK) and `search_conversations` (S2).

```rust
pub struct FileHit {
    pub attachment: AttachmentId, pub message: MessageId,
    pub name: String, pub mime_type: String, pub size: u64,
    pub from: Option<EmailAddress>, pub received_at: DateTime<Utc>,
    pub subject: Option<String>,
    pub matched: Option<Match>,        // FileName or FileContent with location
}
```

## `postio-extract` (new, pure leaf)

```rust
pub struct Limits { pub max_input: u64, pub max_text: usize, pub max_units: usize, pub max_time: Duration, pub max_ratio: u32 }
pub struct Unit { pub location: Location, pub text: String }
pub enum Outcome { Complete, Truncated(Limit), Skipped(Skip), Failed }
pub enum Skip { Encrypted, Unsupported, Empty }
pub struct Extracted { pub units: Vec<Unit>, pub outcome: Outcome }
pub fn extract(bytes: &[u8], mime_type: &str, name: Option<&str>, limits: &Limits) -> Extracted;
pub const EXTRACTOR_VERSION: u32;       // a bump re-extracts everything
```

`Location` is `postio_search::results::Location`; `postio-extract` depends
on `postio-search` for it (both pure leaves).

## `postio-focus`

| Type | Meaning |
|---|---|
| `Policy.caps.results_view: bool` | true on the Mac; the GTK controller never enters results (D17) |
| `Dropdown { state: DropdownState, stamp, focused_default }` | in `bar.rs`; `DropdownState::{Empty, Prefix, Words, Operator(Field), PlainEnglish(Lowered), Commands}` decided from the typed text |
| `Results { query: String, parsed: ParsedQuery, tab, order, page: ListWindow<ConversationHit>, cursor, selector, facets, stamp, popover: Option<Popover>, quick_look: Option<QuickLook> }` | `results.rs`, the mode (R9) |
| `Popover { kind: From\|To\|Date\|Anywhere\|Label, before: String }` | `before` is the query to restore on Esc (FR-027) |
| `QuickLook { position, matches: Vec<Match>, current: usize }` | |
| `History { back: Vec<Entry>, forward: Vec<Entry> }`, `Entry::{Inbox{cursor, selection}, Results{query, tab, order, cursor, selection}}` | `history.rs`, ≤ 50 each way |
| `Request::{Conversations, Passages, Suggest, Relaxations, Files, People, ConversationMatches, RecentSearches, Remember, Forget, SavedCounts, MarkSeen}` and `Request::lane()` | perform.rs maps each to one client call (R8) |
| `Intent::{ShowDropdown(DropdownView), ShowResults(ResultsView), ResultsPage, ShowPopover, ShowQuickLook, ShowRelaxations, ShowSave, LeaveResults}` | words already composed by `postio-ui` |

## `postio-ui` (words)

`search_view.rs` (new): `reason_line(&[RankReason]) -> String` ("you replied
· 3 matches"), `source_tag(&Source) -> String` ("body", "quoted text",
"subject", the file name), `location(&Location) -> String` ("Sheet
‘Summary’, row 14", "Page 2"), `relaxation_line(&Relaxation, names) ->
String`, `nothing_matches(n_filters) -> String`, `searched(n, contents) ->
String`, `footer(total, elapsed, capped) -> String`, `month_group(month, n)`,
`filter_button_label(&Filter, names) -> String` ("From: Ada Moreno", "Since
July"), `date_presets(today) -> [DatePreset; 5]`, `cheat_sheet() -> [(op,
hint); 8]`, `origin_line(&Origin) -> String` ("from ‘last month’").

## Tables

### `postio-storage` schema (HEAD, with a migration)

```sql
-- The searches a person ran (D16): the results view opened, a hit opened
-- from the dropdown, or a saved search run. Never each keystroke. The last
-- 20 distinct queries are kept; older rows are deleted on insert.
CREATE TABLE recent_searches (
    query        TEXT    PRIMARY KEY,   -- exactly as run, the one language
    last_run_at  INTEGER NOT NULL,      -- UTC ms
    hits         INTEGER NOT NULL       -- conversations, as the footer said
);

-- Per saved search, the newest message the person has seen its results up
-- to (D15). The badge counts matches received after it.
CREATE TABLE saved_search_seen (
    key          TEXT    PRIMARY KEY,   -- the [saved_searches.<key>] identity
    seen_up_to   INTEGER NOT NULL       -- UTC ms, a received_at
);
```

Both are new; a fresh table in a migration needs no backfill. A saved
search deleted from `config.toml` leaves an orphan row that the next
`saved_searches` read deletes.

### `postio-index` schema (a new versioned half, `attachments`)

```sql
-- One located unit of an attachment's text (D10): a PDF page, a sheet row,
-- a slide, a paragraph, a line. Derived data: a version bump drops and
-- re-extracts.
CREATE TABLE IF NOT EXISTS attachment_passages (
    id             INTEGER PRIMARY KEY,
    attachment_id  INTEGER NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
    message_id     INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    ordinal        INTEGER NOT NULL,         -- order within the attachment
    location       TEXT    NOT NULL,         -- `Location`, encoded: "page:2", "sheet:Summary:14"
    text           TEXT    NOT NULL,         -- as extracted, for passages
    text_search    TEXT    NOT NULL          -- folded (postio_model::fold), for the index
);
CREATE INDEX IF NOT EXISTS idx_attachment_passages_attachment ON attachment_passages (attachment_id, ordinal);
CREATE INDEX IF NOT EXISTS attachment_passages_fts ON attachment_passages USING fts (text_search);

-- Which attachments have been tried, and how it went: the indexer's queue
-- is "downloaded and not here".
CREATE TABLE IF NOT EXISTS attachment_extraction (
    attachment_id  INTEGER PRIMARY KEY REFERENCES attachments(id) ON DELETE CASCADE,
    version        INTEGER NOT NULL,         -- EXTRACTOR_VERSION
    outcome        TEXT    NOT NULL CHECK (outcome IN ('complete','truncated','skipped','failed')),
    units          INTEGER NOT NULL
);
```

`text` is kept beside `text_search` because passages are cut from the text
as written, and the fold is not reversible. Storage cost is bounded by
`Limits.max_text` per attachment. The store is encrypted at rest; extracted
text never leaves it and is never logged.

### `config.toml`

`[saved_searches.<key>]` gains one field:

```toml
notify = true   # show a quiet badge with new matches; default false
```

`pinned`, `name`, `order` and `query` keep their meaning; ⌥1–4 are the first
four pinned by `order`. Rolling dates live in `query` (D14).
