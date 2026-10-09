# Feature Specification: Search for Postio Focus on the Mac

**Feature Branch**: `feature/focus-search` (cut from `feature/focus-macos`;
lands on `main` after spec 009's #1803)

**Created**: 2026-10-08

**Status**: Draft

**Input**: A design handoff for search in the Mac Focus app
(`Design/focus-macos-search/`: `SPEC.md`, 13 screens at 1440×900,
`source/*.dc.html`), with the brief "two layers that share one query".
Refined by the maintainer on 2026-10-08 with the decisions recorded below as
S1 to S4.

## Context

Spec 009 gave the Mac Focus app a command bar that drops from the toolbar's
search field (its screens 07–10). It searches as you type and lists hits under
the search row, one per conversation. That is the whole of search today, on
both platforms. What the design asks for, and what the engine has:

| The design asks for | The engine has today |
|---|---|
| Operators `label:` and `has:action` | Neither: `postio_search::parse` (`parser.rs`) knows `from to subject in has:attach is: before after filename list larger smaller account group header`. The controller already writes `label:"<name>"` when a label is opened from the places popover (`postio-focus/src/bar.rs`, `go_to`), and the parser reads it as free text |
| Plain English shown as "what it became and what each part came from" (screen 05) | `natural::lower` returns the lowered query with no way back to the English it came from |
| One result per conversation, with why it ranked and where it matched | One result per *message* (`postio_index::executor::search`), collapsed to conversations in the UI (`postio_ui::command_bar::conversations`). A score, no reasons. One 12-token excerpt from the body only (`postio_session::search::snippet_hits`), for the first 50 hits |
| Facets by sender, recipient, label, folder, attachment, action, unread, and matches per month, in one pass | `executor::facets`: three scope counts and a fixed refine list (unread, flagged, attachment, `larger:1M`, two folders), asked as a second read |
| Completion from the mailbox's words, labels, lists, files and people | A did-you-mean for a query that found nothing (ADR 0037), and finders for contacts (by how often they wrote), labels and folders |
| Zero results answered with one-term relaxations and their counts | Only the did-you-mean |
| Attachment contents searched, with a location ("Sheet ‘Summary’, row 14") | File *names* only (`search_documents.filenames`) |
| Recent searches; saved searches that pin, notify and roll their dates | Saved searches in `[saved_searches]` (`postio_ui::saved_search`), name and query and pin. No recents |
| A results view that is a mode of the main window, with back and forward | Nothing: the bar is the only search surface, and Esc is the surface stack's ladder |
| The footer's real search time, under 50 ms on 20k messages with facets | `postio-bench/benches/search_budget.rs`: 120k messages against 100 ms, no facets |

### What this spec inherits

- **Spec 009** in full, and through it spec 007. Where this spec is silent,
  they govern. In particular 009's FR-002/FR-004 (behaviour in
  `postio-focus`, Swift decides nothing), FR-061 (screens compared at
  1440×900), and FR-063 (storyboards; the Mac runner is out of scope).
- **The design**: `Design/focus-macos-search/SPEC.md` and `screens/01`–`13`
  (untracked; the maintainer's reference set, read in place and never
  copied here), with the general Mac rules in `Design/focus-macos-design/`
  (`SPEC.md`, `KEYS.md`).
- **ADR 0037** (a misspelling is answered with a suggestion, never by
  widening the query), **ADR 0038** (Turso), **ADR 0045** (behaviour lives in
  `postio-focus`), **ADR 0044** (interactions are storyboarded).
- **The constitution**, in particular III (one query language), IV
  (test-first), V (performance, gated as counts) and VI (privacy).

### Decisions the maintainer took (2026-10-08)

| # | Topic | Decision |
|---|---|---|
| S1 | Base | Stack on `feature/focus-macos` |
| S2 | Platforms | **Build it for the Mac; Linux adopts later.** This Mac cannot build `postio-gtk`, so every engine change is additive: GTK's `search_hits` path is untouched and CI on Linux proves it. `label:` and `has:action` become operators everywhere, because the query language is shared |
| S3 | The design pack | Stays untracked in `Design/`, cited by path and screen number |
| S4 | Server search | "Search the server too ⌥↩" (screen 13) is **out of scope** |

### Decisions this spec takes

Each was open in the design or between the design and the code. The
alternatives rejected are recorded so they are not re-argued.

| # | Topic | Decision | Rejected |
|---|---|---|---|
| D1 | Where the query lives | The query **string** in `postio-focus` is the one source of truth. Chips, filter-button states, the timeline band and every popover's checks are derived from its parse. A filter button sends a *term edit* (add, remove, replace one token), never a state of its own | Swift holding chip state and sending the result: two states that drift, and a Swift parser (009 FR-004) |
| D2 | Ranking unit | **Conversations.** The executor ranks messages as today, then groups by conversation (thread, or the message when unthreaded) inside the new request, keeping the best message's score and summing matches | Ranking conversations in SQL: the fts score is per row and cannot be aggregated (`fts_score` dies inside any expression, see `executor.rs` `HITS_JOIN`) |
| D3 | What a facet counts | **Conversations**, the same unit as the tab count and the list, over the same capped match as the total (`TOTAL_HITS_CAP`). A capped facet is shown as a floor ("10,000+") | Counting messages: the From popover would say 9 for a person with 3 rows |
| D4 | Months | The **last 12 calendar months** ending with today's, a conversation counted in the month of its newest matching message, which is also the month group the list puts it in | A rolling 365 days in 12 bins: labels that do not match the month groups |
| D5 | Facets in one pass | One request answers hits, totals and every facet, in a **fixed number of statements** whatever the match size, asserted by `postio_storage::test_support::counting` | A second read for facets, as today: the popovers' live preview would trail the list it describes |
| D6 | Quoted text | Decided **when the passage is cut**, with `postio_body::quote::text_stretches` over `indexable_text`, the same text the index holds. The index is unchanged | A separate quoted-text column: a reindex of every body, and a second fold to keep in step |
| D7 | Passages | About 120 characters around the first match, with ellipses, **never the message's first line** (which the row already shows as the preview); when the only match is in the first line, the passage is the window after it. Cut for the visible page only, as a read of its own after the results (#1613 is what 50 excerpts on a keystroke cost) | Cutting 50 excerpts per keystroke, as `search_hits` does |
| D8 | Typing | Swift keeps sending `focus_bar_typed` on every keystroke. The driver keeps **one abort handle per search lane** and aborts the superseded task, and the host stops a search whose caller has gone (D9). Debounce only if the step-1 bench says so | A Swift debounce: a timing policy in the frontend, and a slower answer for every query that was already fast |
| D9 | Cancellation reaches the host | The host answers reads on its own runtime (`postio-host/src/lib.rs`, `Local::call`), so dropping the client's future frees nothing. The new search requests are answered under a `select!` on the reply channel closing, so an abandoned search stops at its next await | Aborting only the driver task: the superseded SQL keeps the connection and the CPU |
| D10 | Attachment contents | A new pure leaf, `postio-extract` (PDF, OOXML, plain text), run by a background indexer over attachments **already downloaded**; text goes to `attachment_passages`, one row per located unit (page, sheet row, slide, paragraph). Never fetches an attachment to index it | Fetching every attachment to index it: network the user did not ask for (constitution VI); an inference or OCR engine (out of scope, and constitution "No AI") |
| D11 | What free text searches | In the new conversation search, free text matches metadata, bodies **and attachment contents**. The old `search_hits` keeps its corpus until Linux adopts this search. The query *language* is unchanged; a saved search or rule evaluated by the in-memory matcher already refuses free text (`matcher::Unsupported`) | Widening `search_hits` now: changes GTK's results in a branch that cannot build GTK |
| D12 | `label:` and `has:action` in the in-memory matcher | Both are **refused** by `postio_search::matcher` (`Unsupported::Token`), like free text: labels and markers are written after filing, which is when the matcher runs. The differential test (`index_suite/digest_matcher.rs`) is unchanged and gains a case proving the refusal | Teaching the matcher labels: it would answer at filing time from a state that does not exist yet |
| D13 | Spelling of written terms | Term edits write one canonical form per filter (`postio_search::query::spell`): `has:attachment`, `is:unread`, `has:action`, `label:"Q3 close"`, ISO dates. The parser still reads every form it reads today | Writing whatever the user would have typed: a relaxation or a chip's ✕ would rewrite text the user never typed in inconsistent forms |
| D14 | Rolling dates | **Keep the date rolling** stores the date term in relative form (`after:90d`), off stores it as ISO (`after:2026-07-01`). Both are the one language; nothing is added to the saved search but the string | A `rolling = true` flag beside the query: a second meaning for the same string (constitution III) |
| D15 | "New matches since last viewed" | A per-saved-search **seen-up-to instant** in the store; the badge is the count of the query's matches received after it, capped. `notify = true` in `[saved_searches.<key>]` turns the badge on. Never a banner, never a system notification | A notification: the design says "quiet badge, never a banner" |
| D16 | Recent searches | A store table of the last 20 distinct queries **run** (results view opened, a hit opened, a saved search run), with their count and when. Never each keystroke. Stored in the encrypted store, never logged | `config.toml`: a hand-edited file is not a history |
| D17 | Results view | A **mode** of the main window held by `postio-focus`, behind a new `Policy.caps.results_view` (true on the Mac, false on Linux until it adopts). History is the controller's: Inbox{cursor, selection} and Results{query, tab, sort, cursor, selection} entries, back and forward | A results window, or results as a surface on the stack: the design says "a mode of the main window, not a new window" (§3) |
| D18 | Esc in results | The ladder is: popover → dropdown → selection → results → inbox, one rung per press; back/forward (⌘[ ⌘], swipe) move through History without closing anything | Esc leaving results with a selection: the selection is the thing Esc must clear first (009, C19) |
| D19 | Flagged, not starred | The rank reason the design writes "you starred" reads **"you flagged"** (CLAUDE.md: the app says Flagged) | Following the PNG |
| D20 | Rank reasons | `Replied` (any matched message is `answered`), `Flagged`, `FrequentSender` (two-way frequency above the contact finder's threshold), `InSubject`, `InFileName`, `Matches(n)`. At most two shown, in that order, joined by " · " | A free-form explanation string from the executor: words are `postio-ui`'s |
| D21 | People frequency | **Two-way**: `contacts.times_seen` (they wrote to you) plus `correspondents.sent_count` (you wrote to them), both existing columns | `times_seen` alone (one way, today's finder) |
| D22 | Vocabulary for completion | The index exposes no term dictionary (`fts_match`, `fts_score`, `fts_highlight` only). Completion asks the index for `prefix*` and recovers words from the best few documents, as `executor::words_near` already does, metadata first. A vocabulary table is the fallback **only if** the step-8 bench misses 20 ms | A vocabulary table maintained on every write: a write cost every sync pays, for a read that may already be fast enough |
| D23 | ⌥⌫ forgets a recent search | `ForgetRecent` takes `alt+BackSpace` on the Mac; `BackToWords`' terminal alternate `alt+BackSpace` is not offered on Apple (`registry::alternate_offered_on`), and `ForgetRecent` is not offered on Freedesktop until Linux adopts the dropdown (`registry::offered_on`) | A different key: the design names ⌥⌫ |
| D24 | ⌘⌫ on no results | With no plain English to take back, `BackToWords` in the results' zero-hit state is answered as **clear filters** (keep the free words, drop every operator), and the field's hint says so (screen 13) | A second command on the same key in the same context |
| D25 | Results-only commands on Linux | New commands that only the results view or the dropdown uses are not offered on Freedesktop (`offered_on`) until Linux adopts, so GTK's keymap, palette and `docs/keybindings.md` rows for Linux do not change | Offering them on Linux as debt: they would be keys that do nothing on a surface GTK does not have |

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Find one message fast from the dropdown (Priority: P1)

⌘K or `/` focuses the toolbar field, which grows left to 860 wide. Empty, the
panel shows the last three searches with their counts, the saved searches as
pills with ⌥1–4, and a cheat sheet of operators with one plain-English
example. Typing words shows up to four top hits with their passage, folder
and date, four "Narrow to" pills with counts, and "Show all 48 results ⌘↩",
focused. The footer says how many matched and how long it took. ↩ opens the
focused message; Esc closes the panel and the list row is focused again.

**Why this priority**: It is the layer used most, and it replaces the bar's
search half on the Mac. Every other story starts from it.

**Independent Test**: Over the search demo store, open the field, see
screen 01; type `atlas budget`, see screen 03 (the footer's time is real);
press ↩ on a hit, the message window opens; Esc, the list row has the
keyboard. Screens 01 and 03 compared, differences listed.

**Acceptance Scenarios**:

1. **Given** an empty field, **When** the panel opens, **Then** it lists at
   most three recent searches newest first with count and when, the pinned
   saved searches with their ⌥ number and count, and the cheat sheet.
2. **Given** a recent search is focused, **When** ⌥⌫ is pressed, **Then** it
   is forgotten and the next one moves up.
3. **Given** words are typed, **When** each keystroke lands, **Then** the
   panel is re-run and reshaped for what is typed now, and an answer for an
   earlier keystroke never replaces it (stale answers dropped, D8).
4. **Given** "Narrow to" is shown, **When** Tab is pressed, **Then** the
   first pill's term is added to the query and the panel re-runs.
5. **Given** the panel is open, **When** a click lands outside it or Esc is
   pressed, **Then** it closes and the list row that had the keyboard has it
   again, with its selection.

---

### User Story 2 - Look through many results (Priority: P1)

⌘↩ (or "Show all") turns the main window into the results view: ‹ Inbox
with an Esc keycap, the query as chips and words, Save search ⌘S; a filter
bar with tabs (Conversations, Files, People) and filter buttons; a timeline
of matches per month; then results grouped as Top hits (Best match only)
and month groups. Each row says who, the subject with highlights, label
pills, a source tag (body, quoted text, subject, file name) and the matching
passage. Top hits say why they ranked. Esc goes back to the inbox with the
same row focused; ⌘[ and ⌘] (and the swipe) move between the two.

**Why this priority**: The second layer is the reason for the redesign;
without it search ends at four hits.

**Independent Test**: Run `atlas budget`, ⌘↩, see screen 06 in light and 07
in dark; toggle Sort to Newest and see the Top hits group go; Esc, the inbox
with the same row; ⌘], the results again with the same cursor. Screens 06
and 07 compared, differences listed.

**Acceptance Scenarios**:

1. **Given** results, **When** Sort is Best match, **Then** a Top hits group
   of at most three comes first, each with a reason line; **When** it is
   Newest, **Then** there are only month groups.
2. **Given** a match inside quoted history, **When** its row is drawn,
   **Then** its tag says "quoted text" and its passage is from the quote.
3. **Given** a match only in the first line of a body, **When** its passage
   is cut, **Then** it is not the first line (D7).
4. **Given** the results view, **When** Esc is pressed with nothing open and
   nothing selected, **Then** the inbox shows with the row and selection it
   had before; **When** ⌘] is then pressed, **Then** the results return with
   their query, tab, sort, cursor and selection.
5. **Given** the system appearance changes, **Then** highlights use the find
   yellow of that appearance and the accent appears only on focus.

---

### User Story 3 - Narrow results with filters, chips and the timeline (Priority: P1)

Every filter is both a chip in the field and a button in the filter bar.
From, To, Date, Anywhere and Label open popovers with a live preview: the
list, counts and timeline change while the popover is open; ↩ applies, Esc
restores. Attachment, Has action and Unread toggle. Dragging across months
on the timeline sets `after:`/`before:`; ⌥←/⌥→ step the range.

**Why this priority**: Narrowing is what the results layer is for.

**Independent Test**: From screen 06, open From, check Ada Moreno (screen
08), see the list narrow live, ↩; the chip appears and the button is solid.
Remove the chip with its ✕; the button is outline again. Open Date, type
"since july" (screen 09). Drag Jul–Sep on the timeline. Screens 08 and 09
compared, differences listed.

**Acceptance Scenarios**:

1. **Given** a filter button is applied, **When** its chip is removed in the
   field, **Then** the button returns to its outline state, and the reverse.
2. **Given** an open popover with a changed choice, **When** Esc is pressed,
   **Then** the query, list, counts and timeline are as before it opened.
3. **Given** the From popover, **When** it lists people, **Then** it lists
   only people who appear in the current results, with their counts; ⌥-click
   excludes (`-from:`).
4. **Given** a drag across three months, **When** it ends, **Then** the query
   holds `after:` the first month's first day and `before:` the day after the
   last month, and the selected months are drawn bold over a soft band.

---

### User Story 4 - Look inside a result without opening it (Priority: P2)

Space on a result opens Quick Look over the list: the subject, the sender
line, "4 matches in this conversation", and one card per match with where
(Body, an attachment, Earlier reply, Subject) and when. j/k move to the next
result with the panel open; ]/[ move between matches; ↩ opens the message;
`a` archives; Space or Esc closes.

**Why this priority**: It saves opening and closing messages to check them,
but results work without it.

**Independent Test**: From screen 06, Space on the first row: screen 10.
Press j, the panel updates in place; ] moves the ring to the next card.
Screen 10 compared, differences listed.

**Acceptance Scenarios**:

1. **Given** Quick Look is open, **When** `a` is pressed, **Then** the
   conversation is archived, the panel moves to the next result, and ⌘Z
   brings it back.

---

### User Story 5 - Act on many results, and keep the search (Priority: P2)

`x` selects the focused result, ⇧X selects all of them. The footer becomes
the bulk bar: "5 selected", Archive a, Label l, Move m, Mark read r, Snooze
s. ⌘S opens the Save popover: a name prefilled from the query, the terms as
chips, Pin (on), Notify when new mail matches (off), Keep the date rolling
(off).

**Why this priority**: Triage over a result set is how a search pays off
twice; saving it is how it pays off again.

**Independent Test**: Select five results, archive them, undo. ⌘S, save
with Notify on; a new matching message arrives in the demo; the saved
search shows a quiet badge with 1. Screen 12 compared, differences listed.

**Acceptance Scenarios**:

1. **Given** ⇧X in results, **When** Archive is pressed, **Then** every
   conversation the query matches is archived, as a predicate (constitution
   V), and one undo restores them.
2. **Given** Keep the date rolling is on, **When** the search is saved,
   **Then** its date term is relative (`after:90d`) in `config.toml`; off, it
   is ISO.
3. **Given** a saved search with notify on, **When** it is viewed, **Then**
   its badge clears.

---

### User Story 6 - Zero results is never a dead end (Priority: P2)

When nothing matches, the centre says "Nothing matches all four filters",
one sentence, and up to four relaxations: each drops or loosens exactly one
term, shows the resulting query and its count, and takes a number key.
Relaxations that would still find nothing are not offered. Below: "Searched
all 18,204 messages on this Mac, including attachment contents."

**Why this priority**: A search that ends in nothing is the moment a person
gives up on local search.

**Independent Test**: Run `from:ada has:attachment before:2026-03-01
subject:"budget v4"`: screen 13 without the server line (S4). Press 2: the
query is the relaxed one and results show. Screen 13 compared.

**Acceptance Scenarios**:

1. **Given** a zero-hit query of n terms, **When** relaxations are measured,
   **Then** each offered one differs from the query by exactly one term, the
   list is ordered by count, largest first, and none has a count of 0.
2. **Given** attachment contents are not yet fully extracted, **When** the
   sentence is drawn, **Then** it does not claim "including attachment
   contents".

---

### User Story 7 - The field understands what is being typed (Priority: P3)

A short prefix gets a ghost completion after the caret (Tab accepts) and
suggestions: the completed word, a label, a mailing list, files, each with a
count. Typing `from:` turns the list into people ranked by how often you
email each other, with the latest two messages from the focused one; ↩ makes
the chip, ⌥↩ an exclusion, ⌫ on an empty value goes back to words; `label:`
and `in:` do the same with labels and folders. Plain English shows an
"Understood as" bar: one tile per term, under it what it came from ("from
‘last month’"); Tab makes chips, ⌘⌫ keeps words.

**Why this priority**: These make typing faster and plain English
trustworthy, but every search works without them.

**Independent Test**: Type `at` (screen 02), `from:` (04), and "invoices
from ada last month" (05). Screens compared, differences listed. Every
suggestion's count is the count the query would give.

**Acceptance Scenarios**:

1. **Given** plain English, **When** it is understood, **Then** each tile's
   origin is the exact words of the sentence it came from, and running the
   tiles returns what typing those operators returns (constitution III).
2. **Given** `from:` with an empty value and a person focused, **When** ⌥↩ is
   pressed, **Then** the chip is `-from:<address>` and struck through.

---

### User Story 8 - Find files by what is in them (Priority: P3)

The Files tab is a grid of cards: a preview, the type, the name with
highlights, "sender · date · size", the matching line from the contents
("Sheet ‘Q3’, row 3: …") and "in ‘<message subject>’". Space opens the file
in the system Quick Look; ↩ opens its message; ⌘↓ saves it.

**Why this priority**: It needs a new extraction pipeline; the rest of
search does not depend on it.

**Independent Test**: Over the demo store with a PDF, an XLSX, a DOCX and a
PPTX attachment already on disk, search a word only inside the XLSX: the
Conversations tab shows the row with the file-name tag and "Sheet
‘Summary’, row 14: …"; the Files tab shows its card. Screen 11 compared.

**Acceptance Scenarios**:

1. **Given** an attachment whose bytes were never downloaded, **When** the
   indexer runs, **Then** it is not fetched; it is indexed when it arrives.
2. **Given** a malformed or hostile PDF, **When** it is extracted, **Then**
   the indexer records it as failed, moves on, and the app never stalls or
   crashes.

---

### User Story 9 - Find people in results (Priority: P4)

The People tab lists the people in the current results: avatar, name,
address, message count, last date. ↩ runs `from:<person>`.

**Why this priority**: Not drawn yet (design §3.11); a simple list.

**Independent Test**: Run `atlas`, ⌘3, ↩ on a person: the query is
`from:<address>` and the Conversations tab shows.

### Edge Cases

- The match is broader than `TOTAL_HITS_CAP`: counts and facets show as
  floors ("10,000+"), the months histogram is drawn and labelled as a floor,
  and Best match falls back to recency as the executor already does past
  `RANK_BY_RELEVANCE_LIMIT`.
- Bodies are still backfilling (`corpus_complete` false): the footer says
  so, and the no-results sentence does not say "all".
- A query names a label or folder that does not exist: zero results with
  relaxations, never an error.
- `label:` on an account whose labels are folders: `label:` matches
  `message_labels` only; `in:` names folders. The relaxation for a `label:`
  that found nothing offers `in:` with the same name when that folder exists.
- The user types while a popover is open: the popover closes and the typing
  goes to the field.
- A filter button is pressed for a term the user also typed by hand: the
  button edits the typed token in place (D13), never adds a second one.
- An excluded and an included term for the same value (`from:ada
  -from:ada`): parsed as typed; zero results; relaxations remove one each.
- Quick Look is open and the result under it is archived from the bulk bar:
  the panel moves to the next result, or closes if none is left.
- A PDF is encrypted, an XLSX has 200,000 rows, a DOCX is a zip bomb: the
  extractor stops at its byte, unit and time limits and records why.
- A saved search's query no longer parses to anything (an operator was
  removed): it runs as free text, as the parser already degrades.
- An input method is composing in the field: no key is taken as a command
  (009 FR-032).
- Back/forward when History has one entry: nothing happens, no sound.

## Requirements *(mandatory)*

### Functional Requirements

**The query (both layers)**

- **FR-001**: The query MUST be one string held by `postio-focus`. Chips,
  filter-button states, the timeline's selected months and every popover's
  checks MUST be derived from its parse, and every control MUST change the
  query only by a term edit (D1).
- **FR-002**: `label:<name>` (quoted when it has spaces) and `has:action`
  MUST be operators of the one language, on every platform, each optionally
  negated (S2). `label:` matches a message carrying a label of that name;
  `has:action` matches a message with an open marker (not dismissed).
- **FR-003**: The parser MUST stay total: `label:` with no value and
  `has:act` are partials, as `is:` is today.
- **FR-004**: Plain English MUST be lowered on the Mac with no network, and
  every lowered term MUST carry the span of the English it came from.
  Lowering MUST give the same query `natural::lower` gives today.
- **FR-005**: Matched words MUST be marked with the find highlight (light
  `rgba(255,204,0,0.38)`, dark `rgba(255,214,10,0.28)`), never the accent.
  Highlight ranges MUST come from the engine.

**The dropdown (screens 01–05)**

- **FR-010**: ⌘K or `/` MUST focus the toolbar field and grow it leftward to
  860 wide, right edge 12 from the window's; the panel MUST hang 6 below it
  with the same left edge and width, and MUST NOT dim the inbox.
- **FR-011**: The panel MUST be re-run on every keystroke and show the state
  of design §2's table for what is typed: empty, short prefix (1–3
  characters), words, an operator being typed, plain English, or `>`
  commands (009's bar, unchanged).
- **FR-012**: The empty state MUST list at most three recent searches with
  count and when, the pinned saved searches with ⌥1–4 and their counts, and
  the 8-item cheat sheet with one plain-English example lowered live.
- **FR-013**: The words state MUST show up to four top hits with passage,
  folder and date, up to four "Narrow to" pills with counts (top senders, has
  attachment, top label), and "Show all N results ⌘↩", focused by default.
- **FR-014**: The footer MUST show key hints and "N matches · T ms", T being
  the measured time of the search that produced the panel.
- **FR-015**: Keys MUST be as design §2 "Keys in the dropdown": ↑↓ move
  (Swift's highlight, 009 FR-004 exception), ↩ open or apply, ⌘↩ results,
  Tab complete or narrow or chip, ⌥↩ exclude, ⌘⌫ keep words, Esc close.

**The results view (screens 06–13)**

- **FR-020**: ⌘↩ or "Show all" MUST turn the main window into the results
  view, a mode of the main window (D17). Esc MUST follow D18's ladder; ⌘[,
  ⌘] and the trackpad swipe MUST move through History.
- **FR-021**: The toolbar MUST hold ‹ Inbox with an Esc keycap, the query
  field (chips, then words, "/ to edit"), and Save search ⌘S. `/` MUST focus
  the field and open the dropdown anchored to it.
- **FR-022**: The filter bar MUST hold the tabs with counts (⌘1–3), the
  filter buttons of design §3.2 (solid when applied, labelled with their
  value, accent ring while their popover is open), and Sort (Best match
  default when the query has words, else Newest).
- **FR-023**: The timeline MUST show the count line, 12 monthly bars (D4),
  and update whenever the query changes; dragging MUST set `after:`/`before:`
  and ⌥←/⌥→ MUST step the range by a month.
- **FR-024**: Results MUST be grouped as design §3.4: Top hits (≤ 3, Best
  match only) then month groups; rows MUST show sender (bold when unread),
  the reason line on top hits (D20), subject with highlights, label pills,
  paperclip, thread count, the source tag and the passage (D7), folder and
  date.
- **FR-025**: When the match is inside an attachment, the tag MUST name the
  file and the passage MUST say where ("Page 2: …", "Sheet ‘Summary’, row
  14: …").
- **FR-026**: Selection MUST use `x`, ⇧X selects every conversation the
  query matches (a predicate), and the footer MUST become the bulk bar with
  the list's verbs and their keys.
- **FR-027**: Filter popovers MUST preview live and MUST restore the
  previous query on Esc; ↩ applies. From/To list only people in the current
  results; Space toggles; ⌥-click excludes. Date offers the presets with
  counts, a plain-words field showing the operator it became, a draggable
  90-tall month chart and "12 of 21 · Jul – Sep 2026".
- **FR-028**: Space MUST open Quick Look (design §3.7); j/k move results
  with it open; ]/[ move between match cards; ↩ opens; `a` archives.
- **FR-029**: ⌘S MUST open the Save popover (design §3.9). Pin gives the
  next free ⌥ number; Notify shows a quiet badge (D15); Keep the date
  rolling writes relative dates (D14). Saved searches are written to
  `[saved_searches]` in `config.toml`.
- **FR-030**: A zero-hit query MUST show the no-results page (design §3.10)
  with up to four relaxations of exactly one term each, ordered by count,
  none of count 0, picked by number keys; and the sentence naming how many
  messages were searched and whether attachment contents were included.
- **FR-031**: The Files tab MUST show the grid of design §3.8; Space opens
  the system Quick Look on the file; ↩ opens its message; ⌘↓ saves it.
- **FR-032**: The People tab MUST list the people in the results (design
  §3.11, D21); ↩ runs `from:<address>`.

**Engine (shared with Linux, additive)**

- **FR-040**: A new request MUST answer, for one query, sort, tab and page:
  conversation hits with score, rank reasons and match counts; the total;
  facets by sender, recipient, label, folder, attachment, action and unread;
  the 12-month histogram; the Files and People counts; and the elapsed time,
  in a fixed number of statements (D5).
- **FR-041**: Each match MUST carry its source (subject, body, quoted, file
  name, file content with location) and a passage with highlight ranges.
- **FR-042**: A prefix request MUST answer completions from the mailbox's
  words, labels, lists, file names and people ranked by two-way frequency
  (D21, D22), each with a count.
- **FR-043**: A relaxation request MUST answer the count of every
  one-term-relaxed variant of a query (drop a term; widen `subject:` to
  anywhere; `label:` to `in:`).
- **FR-044**: Attachment contents MUST be extracted for PDF, DOCX, XLSX,
  PPTX and plain text from attachments already on disk, with a location per
  unit; the location model MUST also hold image text and iWork for later.
- **FR-045**: Recent searches and the saved searches' seen-up-to instants
  MUST be kept in the store (D15, D16).
- **FR-046**: GTK's search path (`Req::SearchHits`, `Req::Facets`,
  `postio_session::search::execute*`, `executor::search`, `executor::facets`)
  MUST keep its behaviour; changes there are limited to `label:` and
  `has:action` (S2).
- **FR-047**: A search whose caller has gone MUST stop at its next await
  (D9), and a superseded search MUST never draw (stale stamps dropped).

**Privacy and boundaries**

- **FR-050**: Nothing in this feature may reach the network: no server
  search (S4), no fetch of an attachment to index it, no remote preview.
- **FR-051**: Logs MUST NOT carry a query, a passage, extracted text or a
  file name: ids, counts, durations and outcomes only.
- **FR-052**: `postio-extract` MUST be a pure leaf: no store engine, no
  async runtime, no network, no toolkit, no inference engine, enforced by
  `check-crate-boundaries.py`.
- **FR-053**: Quick Look on a file MUST hand the system a file in the app's
  own temporary directory, removed when the panel closes; never a path
  outside the app's container and never a URL.

**Order and verification**

- **FR-060**: The work MUST be built in the order of the brief's ten steps
  (tasks.md phases 1–10); each step MUST leave the app runnable and be
  committed.
- **FR-061**: A screen MUST NOT be called done until captured at 1440×900 in
  light (and dark for 06 and 07) and compared with its PNG, every difference
  listed and fixed or explained.
- **FR-062**: The benchmark MUST be run and reported after steps 1, 4 and 8.
  If full search with facets and histogram exceeds 50 ms on 20k messages,
  work MUST stop and the maintainer be asked before working around it.
- **FR-063**: Interactions this spec adds MUST be written as storyboards in
  `storyboards/search/` before they are built (009 FR-063); until the Mac
  runner exists, landings are labelled `interactions-unreviewed`.

### Key Entities

- **Query**: the one string; its parse is a list of tokens (the design's
  *terms*), each an operator clause, a partial or free text, with its span.
- **Lowered**: a plain-English sentence's query plus, per token, the span of
  the English it came from.
- **Conversation hit**: a conversation, its best message, score, rank
  reasons, match count and matches.
- **Match**: where a query matched (source, location) and the passage with
  highlight ranges.
- **Search facets**: counts by sender, recipient, label, folder, attachment,
  action, unread, and 12 monthly counts, over the same capped match.
- **Suggestions**: ghost completion and completions by kind, each with a
  count.
- **Relaxation**: one term dropped or loosened, the resulting query, its
  count.
- **Recent search**: a query run, when, and its count.
- **Saved search**: a named query (`[saved_searches]`), pinned, notify, and
  its seen-up-to instant.
- **Attachment passage**: one located unit of an attachment's text.
- **History**: the main window's back/forward list of Inbox and Results
  entries.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Full search with facets and the months histogram answers in
  **under 50 ms** at p95 on a 20,000-message store on the dev Mac, for each
  of the bench's query shapes, and its statement count is constant in the
  match size.
- **SC-002**: Suggestions for a prefix answer in **under 20 ms** at p95 on the
  same store.
- **SC-003**: The passages for one screen of results (≤ 20 conversations)
  arrive within 50 ms of the results, so a keystroke to a filled screen stays
  inside the constitution's 100 ms local-search budget.
- **SC-004**: A keystroke in the field is answered within one frame (16 ms)
  on the main thread; no search runs on it.
- **SC-005**: All 13 screens are captured at 1440×900 in light, and 06 and 07
  in dark, compared with their PNGs, every difference listed; none
  unexplained.
- **SC-006**: A person can find, narrow, act on and save a search over the
  demo store without touching the pointer.
- **SC-007**: Linux's suites pass on CI after every engine change; GTK's
  search storyboards (`storyboards/search/`) are unchanged and pass.
- **SC-008**: Every relaxation offered has a non-zero count, and every
  facet's count equals what its term would return.

## Out of scope

- **Searching the server** ("search the server too ⌥↩", screen 13) (S4).
- **Linux/GTK adoption** of the dropdown, the results view, the conversation
  search and attachment-content search (S2). The engine is ready for it;
  the GTK surfaces are later work.
- **iWork and image text (Vision OCR)**: a follow-up on the same location
  model (`Location::ImageText`, and iWork as pages/sheets).
- **Fetching attachments to index them**, and any cloud or model-based
  understanding of queries or files.
- **Syncing saved searches across Macs** ("if settings sync exists"): there
  is no settings sync; they stay in `config.toml`.
- **The Mac storyboard runner** (009 FR-063).

## Assumptions

- Spec 009's bar, controller and FFI are in place; this spec extends
  them and does not re-plan them.
- The search demo store is invented mail with reserved-domain addresses;
  no name in it is taken from the design PNGs beyond the invented ones the
  design itself uses (the PNGs also show a real first name, which is not
  used).
- The dev Mac is the reference machine for SC-001–SC-003; CI gates the
  counts, not the timings (constitution V).
