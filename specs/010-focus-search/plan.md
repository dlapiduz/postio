# Implementation Plan: Search for Postio Focus on the Mac

**Branch**: `feature/focus-search` | **Date**: 2026-10-08 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/010-focus-search/spec.md`

## Summary

Search becomes two layers over one query string, built for the Mac and
ready for Linux.

1. **Engine first, additive.** `postio-search` learns `label:`,
   `has:action`, origins for plain English, term edits, relaxations,
   passages and rank reasons. `postio-index` answers a conversation search
   with every facet and a months histogram in a fixed number of statements,
   and completions for a prefix. A new pure leaf, `postio-extract`, reads
   PDF and OOXML. GTK's `search_hits` path is untouched.
2. **Behaviour in `postio-focus`.** The bar's search half grows the
   dropdown's states; a new results mode with History and an Esc ladder sits
   behind `Policy.caps.results_view` (Mac only).
3. **The Mac draws it.** AppKit holds the chip field, panels, tables, grid,
   popovers and Quick Look; SwiftUI holds their content (009's split).

Built in the brief's ten steps, each one runnable and committed, with
benchmarks after steps 1, 4 and 8.

## Technical Context

**Language/Version**: Rust 1.99.0 (`rust-toolchain.toml`; on this Mac
`unset RUSTUP_TOOLCHAIN MAKEFLAGS` first); Swift tools 6.0

**Primary Dependencies**:
- **Rust:** `postio-search`, `postio-index`, `postio-storage`,
  `postio-session`, `postio-host`, `postio-client`, `postio-focus`,
  `postio-ui`, `postio-core`, `postio-ffi` (UniFFI 0.32). New:
  `postio-extract` with `pdf-extract` 0.12, `zip` 8 (no default features,
  `deflate-flate2`), `quick-xml` 0.42, `encoding_rs` (research R5).
- **Swift:** AppKit, SwiftUI, Quartz (`QLPreviewPanel`), QuickLookThumbnailing
  (file previews from the temp copy).

**Storage**: the one Turso store. Two new store tables (`recent_searches`,
`saved_search_seen`) with a migration in `postio-storage/src/schema.rs`; a
new versioned half in `postio-index/src/index.rs` (`attachment_passages`,
`attachment_extraction`). `config.toml` gains `notify` per saved search.

**Testing**:
- **Rust:** `cargo test -p postio-search -p postio-extract -p postio-focus
  --lib`; `cargo nextest run -p postio-index --test index_suite`,
  `-p postio-storage --test storage_suite`, `-p postio-ffi --test ffi_suite`,
  `-p postio-search --test parse`, `-p postio-focus --test bar` and new
  `--test results`. Counts via `postio_storage::test_support::counting`.
- **Bench:** a new `postio-bench/benches/search_focus.rs`, 20k messages,
  run by hand on the dev Mac and nightly (compiled, not timed, on CI).
- **Swift:** Swift Testing via `scripts/macos-test.sh`.
- **Screens:** `scripts/macos-shot.sh` at 1440×900 against each PNG.

**Target Platform**: macOS 14+; Linux stays green on every commit (CI).

**Project Type**: desktop app (Rust engine + native Swift frontend)

**Performance Goals**: full search with facets and months < 50 ms p95 on
20k messages; suggestions < 20 ms; a screen of passages < 50 ms after
results; a keystroke < 16 ms on the main thread (spec SC-001–SC-004).

**Constraints**:
- GTK's search path untouched (FR-046); this Mac cannot build `postio-gtk`.
- `PostioKit` imports no AppKit (#1264).
- The controller does no I/O; Swift parses no query.
- Nothing reaches the network; attachments are never fetched to index them.
- No worktree path in anything rustc sees.

**Scale/Scope**: 13 screens; ~12 new client requests; ~10 new registry
commands; one new crate; ~25 Swift files new or changed.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-checked after Phase 1 design.*

| Principle | How this plan meets it | Status |
|---|---|---|
| I. Local-first | Every search is local; no server search (S4). Bulk verbs from results are the list's verbs, store-write-then-emit, undoable | Pass |
| II. One command table | Every new key is a `CommandId` in `postio-core/src/registry.rs` with a palette entry and accessible action; `docs/keybindings.md` regenerated. Mac-only commands use `offered_on` (D25), not a second keymap | Pass |
| III. One query language | `label:` and `has:action` join the one language everywhere (S2). Chips, buttons, timeline and popovers edit the one string (D1). Plain English lowers to it and shows its origins. Rolling dates are the language's relative dates (D14). **Watch item**: D11 widens what free text searches in the new request only, until Linux adopts it (Complexity Tracking) | Pass, with D11 tracked |
| IV. Test-first | Every task in tasks.md has a test task before it, observed red. Swift view models and keyboard paths are tested without a window. Storyboards written before interactions are built (009 FR-063) | Pass |
| V. Performance, gated as counts | Statement budgets per request asserted with `counting`; wall-clock bench reported after steps 1, 4, 8 with the stop rule (FR-062); the list stays windowed and ⇧X is a predicate | Pass |
| VI. Privacy | No network; extraction only of downloaded attachments; extracted text stays in the encrypted store; logs carry ids, counts and durations only; Quick Look gets a temp copy in the app's container (FR-053) | Pass |
| VII. Boundaries | `postio-extract` gets a rule in `check-crate-boundaries.py` (no store engine, runtime, network, toolkit, inference engine). `postio-search` stays pure. The FFI stays the only Swift boundary | Pass |

Re-check after Phase 1: no new violations. The contracts keep words in
`postio-ui`, behaviour in `postio-focus`, SQL in `postio-index`, and bodies
and blobs in `postio-session`.

## No ADR

The decisions D1–D25 are this feature's and live in spec.md. Nothing here
is a rule other work must obey beyond what ADRs 0037, 0041 and 0045 already
say, except the `postio-extract` boundary, which is a line in the boundary
script and its fixture, as `postio-calendar`'s is.

## Design by crate

### `postio-search` (pure)

- `query.rs`: `Field::Label`, `Filter::Label`, `Filter::HasAction`;
  `Field::keyword`/`parse`/`takes_free_text` arms; `spell(&Clause)`.
- `parser.rs`: `has:` values `action`/`actions`; `label:` as a free-text
  field (quoted values keep spaces, as `subject:` does).
- `matcher.rs`: no change in code; a test that `label:`/`has:action` are
  `Unsupported` (D12).
- `natural.rs`: `lower_with_origins`; the `Lowering::step` loop records, for
  every word or phrase it emits, the byte span of the words it consumed.
  `lower` delegates. Adds "with action", "needs action", "labelled X" only if
  the phrase table test asks for them (it does: design example list).
- `edit.rs` (new): `Edit`, `apply`.
- `relax.rs` (new): `relax`.
- `passage.rs` (new): `cut`, built on `highlight::find`.
- `results.rs`: `Source`, `Location`, `Match`, `Passage`, `RankReason`,
  `ConversationKey`, `ConversationHit`, `ConversationResults`,
  `ConversationOrder`, `ResultsTab`, `FileHit`.
- `facets.rs`: `SearchFacets`, `Count`, `MonthCount`, `months_ending(today)`.
- `suggest.rs`: `Suggestions`, `Completion`, `Person`, `rank_words`.

### `postio-index`

- `executor.rs`:
  - `filter_condition` (line ~1889) gains `Label` (EXISTS over
    `message_labels` joined to `labels` by name, lowercased) and `HasAction`
    (EXISTS over `markers` with `dismissed_at IS NULL`). Both paths use it.
  - `search_conversations` (new): reuses `Plan::build` for the match and
    count; a new `Plan::project` streams the narrow projection (research
    R2) into a `Fold` that builds conversations, facets and months; ranks
    the pool with `rank_score`; hydrates the page; learns per-page which
    columns matched (for `InSubject`/`InFileName` and `Source`).
  - Attachment content: a third arm in the match union for the new request
    only, `attachment_passages_fts` over `text_search`, joined back to
    `message_id`, weighted like the body (`BODY_SCORE_WEIGHT`).
  - `relaxation_counts`, `completions`, `files`, `people` (new).
- `index.rs`: `ATTACHMENTS_SCHEMA_VERSION` and the new half in
  `ensure_schema`; `index_attachment_text`, `attachments_missing_text`,
  `clear_account_attachment_index`.
- Tests in `tests/index_suite/`: new `conversations.rs`, `facets_one_pass.rs`,
  `label_and_action.rs`, `completions.rs`, `relaxations.rs`,
  `attachment_text.rs`; `search_statement_budget.rs` gains the new budgets;
  `digest_matcher.rs` gains the refusal case.

### `postio-storage`

- `schema.rs`: `recent_searches`, `saved_search_seen` in `HEAD`; the old
  `HEAD` copied to `tests/schemas/<fingerprint>.sql`; a `Migration` creating
  both (`IF NOT EXISTS`).
- `searches.rs` (new): `recent`, `remember`, `forget`, `seen_up_to`,
  `mark_seen`, `forget_seen_except(keys)`.

### `postio-extract` (new crate, pure leaf)

`src/lib.rs` (`extract`, `Limits`, `Extracted`, `Outcome`),
`src/pdf.rs`, `src/ooxml.rs` (docx, xlsx, pptx walkers), `src/text.rs`,
`src/limits.rs`. Depends on `postio-search` (for `Location`), `pdf-extract`,
`zip`, `quick-xml`, `encoding_rs`. Tests over fixtures in
`crates/postio-extract/tests/fixtures/` (made by a script in the crate's
`tests/`, invented content, reserved-domain names), including hostile files
(encrypted PDF, zip bomb, 200k-row sheet, truncated PDF).

### `postio-session`

- `search.rs`: `conversations`, `passages`, `conversation_matches`,
  `suggest`, `relaxations`, `files`, `people`, `saved_counts`, beside
  today's functions, which do not change.
- `attachment_text.rs` (new) and `spawn_attachment_indexer` in `lib.rs`.

### `postio-client` and `postio-host`

- `protocol.rs`/`api.rs`: the requests of
  [contracts/engine-search.md](contracts/engine-search.md), `Req::family`
  and `Req::cancellable`.
- `postio-host/src/lib.rs`: routing in `answer`; the `select!` in
  `Local::call` for cancellable requests (D9); starting the attachment
  indexer where the body indexer starts.
- `postio-host/src/search.rs`: thin wrappers, as `hits` and `facets` are.

### `postio-core`

`registry.rs`: the commands of research R10, `Context::Results`
(`context.rs`), the `offered_on`/`alternate_offered_on` arms (D23, D25);
list verbs add `Context::Results` to their contexts.

### `postio-ui` (words)

`search_view.rs` (new), every string in the design's screens that is not a
name or a passage; `hints.rs` for the footer hints from the registry;
`saved_search.rs` gains `notify` and a `Verb::Save { query, name, pin,
notify }` form; `postio-config/src/filters.rs` gains `notify`.

### `postio-focus`

- `lib.rs`: `Policy.caps.results_view`; new `Request`s, `Reply`s and
  `Intent`s; `Request::lane()`.
- `bar.rs`: in search mode with `results_view`, the dropdown's states
  (FR-011) replace the blend's search rows; without it (GTK) the bar is
  unchanged, which `tests/bar.rs` keeps proving.
- `results.rs` (new): the mode; term edits; popovers with restore; Quick
  Look; selection over hits (`postio_ui::selection::Selector`); the
  no-results page; bulk verbs routed to `verbs.rs` with a predicate aim for
  ⇧X.
- `history.rs` (new): back/forward.
- `surfaces.rs`: the Esc ladder's results rungs (D18).
- `perform.rs`: one arm per new request.
- Tests: `tests/bar.rs` (dropdown states, keys), new `tests/results.rs`,
  `tests/history.rs`.

### `postio-ffi`

`focus_search.rs` (new), `event.rs` (appended variants), the driver in
`focus_list.rs`: `ask` keeps `HashMap<Lane, AbortHandle>` and aborts the
previous task of a lane (D8). Tests in `tests/ffi_suite/focus_search.rs`.

### Swift

| Target | New or changed |
|---|---|
| `PostioKit` | `SearchQueryModel.swift` (chips and buttons from `QueryViewFfi`), `DropdownModel.swift` (sections, highlight walk), `DropdownView.swift`, `ResultsModel.swift`, `FilterBarView.swift`, `TimelineView.swift`, `ResultRowView.swift`, `FilterPopoverViews.swift`, `QuickLookBody.swift`, `SavePopoverView.swift`, `NoResultsView.swift`, `FileCardView.swift`, `PeopleListView.swift`, `SearchFooter.swift`; `CommandBarModel.swift` and `CommandBarView.swift` route search mode to the dropdown; `FocusIntents.swift` gains the new events |
| `PostioAppKit` | `ChipQueryField.swift`, `CommandBarPanel.swift` (860 wide, field growth), `ResultsTable.swift` (group rows), `FilesGrid.swift`, `FilterPopover.swift`, `QuickLookPanel.swift`, `FilePreview.swift` (`QLPreviewPanel` data source) |
| `Postio` | `MainWindow.swift` (results toolbar, swipe → History), `Engine.swift` (events) |
| Tests | `PostioKitTests/SearchQuerySyncTests.swift`, `DropdownKeyboardTests.swift`, `ResultsModelTests.swift`, `TimelineTests.swift`; `PostioAppKitTests/ChipQueryFieldTests.swift`, `ResultsTableTests.swift` |

## Threading and cancellation

- **Main thread (Swift):** `focus_bar_typed` and the other calls are
  synchronous and only update controller state under its lock; no search
  runs there (SC-004).
- **Driver (tokio, `postio-ffi`):** each `Request` is spawned; search lanes
  keep one `AbortHandle` each; aborting drops the client future.
- **Host:** cancellable requests stop when their reply channel closes
  (D9). Body decodes for passages run on the host's runtime as
  `snippet_hits` does; extraction runs on `spawn_blocking`.
- **Staleness:** the controller's stamps remain the correctness guarantee;
  cancellation is only about wasted work.
- **Order:** Conversations first; Passages for the visible page after it
  lands; Suggest in parallel with Conversations in the Prefix state.

## Performance plan

1. **Step 1** builds `benches/search_focus.rs`: a deterministic 20k-message
   corpus (threads of 1–12, 40 senders with a Zipf spread, 12 labels, 30%
   with attachments, 2,000 extracted attachment units, 24 months of dates),
   and the shapes: one word (~1%), two words, operator only, operator plus
   words, common word (most messages), zero hits with four filters (the
   relaxation path), and prefixes `a`, `at`, `atl`. It reports p50/p95 for
   `search_conversations`, `completions` and `relaxation_counts`, and the
   statement counts. Report goes in `docs/notes/<date>-focus-search-step-1.md`.
2. **Gate as counts on CI**: `search_statement_budget.rs` asserts each
   request's statement ceiling against corpora of 100 and 2,000 messages:
   the same count at both sizes.
3. **Step 4** repeats it with the popovers' preview pattern (the same
   request re-asked per check) and records it; **step 8** with completions
   and the people autocomplete.
4. **Stop rule** (FR-062): over 50 ms p95 for any full-search shape, or
   over 20 ms for suggestions, the step stops and the maintainer is asked
   with the measurements and the options in research R2.

## Test plan

| Layer | What | Where |
|---|---|---|
| Parser | `label:`, `has:action`, partials, negation, `spell` round-trip, every existing case unchanged | `postio-search/tests/parse.rs`, unit tests |
| Plain English | a phrase table: sentence → query → origins, including the design's examples ("invoices from ada last month", "since july", "2 weeks ago", "last spring") | `natural.rs` tests |
| Edits | add/remove/replace/toggle/set months/clear filters, in place, no duplicates | `edit.rs` tests |
| Relaxations | one term each, `subject:` widened, `label:`→`in:`, ≤ 8 | `relax.rs` tests |
| Passages | ~120 chars, word edges, ellipses, never the first line, ranges land on the matched words, multibyte text | `passage.rs` tests |
| Executor | conversations grouping, reasons, facets equal per-term counts (SC-008), months, capped floors, label/action conditions, statement budgets | `index_suite` |
| Extraction | each format's units and locations; every hostile fixture ends in its recorded outcome within its limit | `postio-extract` tests |
| Session | quoted vs body source over the `.eml` corpus; attachment indexer never fetches | `postio-session` tests, `ffi_suite` |
| Controller | dropdown states and keys; results mode; History; Esc ladder; popover restore; Quick Look walk; the GTK bar unchanged with `results_view` off | `postio-focus/tests/{bar,results,history}.rs` |
| FFI | a typed sequence over the demo store emits dropdown views in order with no stale one; cancel stops the host read; row reads | `ffi_suite/focus_search.rs` |
| Swift | query/chip sync field ↔ filter bar; dropdown keyboard paths; timeline drag → months; results model grouping | `macos/Tests/…` |
| Screens | 01–13 light, 06/07 dark at 1440×900 | `scripts/macos-shot.sh`, notes per step |
| Linux | CI's `postio-gtk` suites and `storyboards/search/` unchanged | CI |

## Project Structure

```text
specs/010-focus-search/
├── spec.md, plan.md, research.md, data-model.md, tasks.md
└── contracts/
    ├── engine-search.md   # client requests, host behaviour, budgets
    └── ffi-search.md      # what Swift calls and receives

crates/
├── postio-search/src/{query,parser,natural,facets,results,suggest}.rs + edit.rs, relax.rs, passage.rs
├── postio-index/src/{executor,index}.rs, tests/index_suite/…
├── postio-storage/src/{schema.rs, searches.rs}, tests/schemas/<fp>.sql
├── postio-extract/                       # NEW
├── postio-session/src/{search.rs, attachment_text.rs, lib.rs}
├── postio-client/src/{protocol,api}.rs
├── postio-host/src/{lib,search}.rs
├── postio-core/src/{registry,context}.rs
├── postio-config/src/filters.rs
├── postio-ui/src/{search_view.rs, saved_search.rs, hints.rs}
├── postio-focus/src/{lib,bar,perform,surfaces}.rs + results.rs, history.rs
├── postio-ffi/src/{focus_search.rs, event.rs, focus_list.rs}
├── postio-demo/src/lib.rs                # + Seed::Search
└── postio-bench/benches/search_focus.rs  # NEW

macos/Sources/{PostioKit,PostioAppKit,Postio}/…, macos/Tests/…
scripts/checks/check-crate-boundaries.py  # + postio-extract
storyboards/search/…                      # new, toolkit-neutral
```

## Complexity Tracking

| Violation | Why needed | Simpler alternative rejected because |
|---|---|---|
| A new crate (`postio-extract`) | PDF and OOXML parsing are large, panicky and slow to compile; they must not enter `postio-search` (pure, compiled by everything) or the store crates | A module in `postio-session`: it would put two parsers in the crate every frontend links, with no boundary saying they never reach the store |
| Two search paths for a while (D11) | GTK cannot be built here, and S2 says GTK's path is untouched | Moving GTK onto the new request in this branch: unverifiable locally, and against S2. Linux adoption removes the old path |
| A second walk of the match for relaxations | Each variant is a different query; there is no way to count them in one walk | Guessing counts from facets: wrong for `subject:`→anywhere and for every negated term |
