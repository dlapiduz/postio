---
description: "Task list for spec 010, search for Postio Focus on the Mac"
---

# Tasks: Search for Postio Focus on the Mac

**Input**: `specs/010-focus-search/` — [spec.md](spec.md), [plan.md](plan.md),
[research.md](research.md), [data-model.md](data-model.md),
[contracts/](contracts)

**This file is the tracker.** There are no issues for these tasks
(constitution, Development Workflow). Tick a box in the commit that
finishes it.

**Tests**: Required (constitution IV). Every implementation task is preceded
by the task that writes its test, and that task says what is observed red.
A test that is green on arrival (it pins behaviour that must not change)
says so, and is a guard, not a TDD step.

## Conventions

- `[P]` means the task can run in parallel: different files, and no
  dependency on an unfinished task.
- `[USn]` is the user story in spec.md the task serves.
- **Each phase is one build step of the brief**, ends runnable, and ends
  with a commit. Commits end `Refs: specs/010-focus-search` and the task
  ids. Never write a closing keyword.
- **Before any cargo command on this Mac**: `unset RUSTUP_TOOLCHAIN
  MAKEFLAGS`. Rust changed → `scripts/macos-build.sh --lib-only`; Swift
  changed → `cd macos && swift build`; Swift tests → `scripts/macos-test.sh`
  (`macos/CLAUDE.md`).
- **The GTK path is untouched** (S2, FR-046). Any change to a shared type or
  function is additive; `postio-gtk` does not build here, so before each
  phase's commit grep its call sites (`grep -rn "<name>" crates/postio-gtk`)
  and let CI's Linux run prove it
  (`docs/notes/2026-09-06-moving-code-out-of-a-crate-you-cannot-compile.md`).
- **Screens** (FR-061): `scripts/macos-shot.sh <nn>-<slug> --seed search
  --size 1440x900 --light` (and `--dark` for 06 and 07), with
  `POSTIO_DEMO_KEYS` to reach the state. Compare with
  `Design/focus-macos-search/screens/<nn>-*.png` in the main checkout and
  **list every difference** in that step's note,
  `docs/notes/<date>-focus-search-step-<n>.md`, each fixed or explained by a
  decision (S*, D*, or 009's M*/C*). The PNGs are never copied into the
  repository.
- **Benchmarks** after steps 1, 4 and 8 go in the same note: p50/p95 per
  shape and statement counts. **Stop rule (FR-062)**: full search with
  facets and months over 50 ms p95 on 20k messages, or suggestions over
  20 ms, stops the work: commit what exists, and ask the maintainer with
  the numbers and the options in research R2 before working around it.
- **Storyboards** (FR-063): an interaction is written in
  `storyboards/search/` (spec 008's format) before it is built. Landings that
  change Mac interactions carry `interactions-unreviewed`.

---

## Phase 1: Step 1 — the engine: parser, conversation search, bench

**Goal**: everything search needs below the controller, minus completions
(step 8) and attachment contents (step 9), measured.

**Independent test**: `cargo nextest run -p postio-index --test
index_suite` green; the bench reports under budget, or the stop rule fires.

### Setup

- [x] T001 [P] Write a failing test in `crates/postio-demo/src/lib.rs` (tests module): `Seed::from_id("search")` round-trips, and the seed holds ≥ 400 messages over 24 months, threads of 1–12, the senders, labels (Atlas, Harbor, Receipts…), two open markers, and attachments with stored blobs (one each of PDF, XLSX, DOCX, PPTX, text) whose names and contents mention "atlas budget". Red: no such seed
- [x] T002 Add `Seed::Search` to `crates/postio-demo/src/lib.rs`: invented mail in the shape of screens 01–13 (names as the design invents them, every address `@example.com`, and no other real name the PNGs show), its attachment blobs built in memory. Make T001 green
- [x] T003 [P] Write a guard test in `crates/postio-index/tests/index_suite/executor.rs`, `gtk_search_path_is_pinned`: a fixed corpus and twelve queries (no `label:`/`has:action`) through `executor::search` and `executor::facets`, their ids, order, totals and refinements recorded inline. Green on arrival; it must stay green through every phase (FR-046)

### Operators

- [x] T004 [P] [US3] Write failing tests in `crates/postio-search/tests/parse.rs`: `label:Atlas`, `label:"Q3 close"`, `-label:atlas`, `label:` (a `Partial`), `has:action`, `has:actions`, `-has:action`, `has:act` (a `Partial`); every existing case unchanged. Red: `label:` parses as free text, `has:action` as a partial
- [x] T005 [US3] Add `Field::Label`, `Filter::Label(String)`, `Filter::HasAction` in `crates/postio-search/src/query.rs` (keyword, `parse`, `takes_free_text`, `field()`), and the `has:` values in `crates/postio-search/src/parser.rs`. Update the crate doc's operator list in `lib.rs`. Make T004 green
- [x] T006 [P] Write failing tests in `crates/postio-search/src/query.rs` for `spell(&Clause)` (D13): every filter, negated or not, spells to a form `parse` reads back to the same clause; `has:attachment`, `label:"Q3 close"`, ISO dates. Red: no `spell`
- [x] T007 Add `query::spell`. Make T006 green
- [x] T008 [P] Write a guard case in `crates/postio-index/tests/index_suite/digest_matcher.rs`: `Matcher::new` refuses `label:atlas` and `has:action` with `Unsupported::Token` naming the token (D12). Green on arrival after T005 (the matcher's `_` arm already refuses filters it does not know); it pins the refusal so a later matcher change cannot answer at filing time
- [x] T009 [P] [US3] Write failing tests in a new `crates/postio-index/tests/index_suite/label_and_action.rs` (add to `main.rs`): `label:atlas` finds exactly the messages carrying that label in any account, case-insensitive; a label that does not exist finds nothing; `has:action` finds messages with an undismissed marker and not a dismissed one; negations invert both. Red: free text and a partial match the wrong rows
- [x] T010 [US3] Add the `Filter::Label` and `Filter::HasAction` arms to `filter_condition` in `crates/postio-index/src/executor.rs` (EXISTS over `message_labels`⋈`labels` by lowercased name; EXISTS over `markers` with `dismissed_at IS NULL`). Make T009 green; T003 stays green

### Pure pieces

- [x] T011 [P] [US3] Write failing tests in a new `crates/postio-search/src/edit.rs`: `Add`, `Remove`, `Replace`, `Toggle`, `SetDates`, `ClearFilters`; a typed token is edited in place and never duplicated; the rest of the text keeps its raw form; `Toggle(HasAttachment)` on a query holding `has:attach` removes it. Red: no module
- [x] T012 [US3] Implement `edit::apply` (data-model.md). Make T011 green
- [x] T013 [P] [US7] Write the failing phrase table in `crates/postio-search/src/natural.rs` tests for `lower_with_origins`: "invoices from ada last month" → `invoices from:ada after:… before:…` with origins `from ‘ada’`, `from ‘last month’`; "since july", "2 weeks ago", "last spring", "with attachments", "unread from ada", a sentence with a quoted phrase, one with an operator typed as is. Assert also that `.query == lower(..)` for every row. Red: no function
- [x] T014 [US7] Implement `natural::lower_with_origins`, recording spans in `Lowering::step`; `lower` delegates. Make T013 green, existing `natural` tests unchanged
- [x] T015 [P] [US6] Write failing tests in a new `crates/postio-search/src/relax.rs`: four filters give four `Drop`s; `subject:"budget v4"` also gives `Anywhere`; `label:x` gives `FolderNotLabel`; free words drop one at a time only when there are two or more; at most 8; each relaxed query differs by exactly one token. Red: no module
- [x] T016 [US6] Implement `relax::relax`. Make T015 green
- [x] T017 [P] [US2] Write failing tests in a new `crates/postio-search/src/passage.rs`: a window of ~120 chars snapped to word edges; ellipsis flags; the match in the first line only → the window after the first line (D7); ranges land on the matched words in multibyte text; no match → `None`. Red: no module
- [x] T018 [US2] Implement `passage::cut` over `highlight::find`. Make T017 green
- [x] T019 [P] Write a failing test for `facets::months_ending(today)` in `crates/postio-search/src/facets.rs` (12 first-days, oldest first, crossing a year; red: no function), then add it and the result types of data-model.md to `crates/postio-search/src/{results,facets,suggest}.rs` (`Source`, `Location`, `Match`, `Passage`, `RankReason`, `ConversationKey`, `ConversationHit`, `ConversationResults`, `ConversationOrder`, `ResultsTab`, `FileHit`, `SearchFacets`, `Count`, `MonthCount`, `Suggestions`, `Completion`, `Person`) and `AddressId` to `crates/postio-model/src/ids.rs`. The types carry no behaviour of their own; their consumers' tests cover them

### Conversation search

- [x] T020 [P] [US2] Write failing tests in a new `crates/postio-index/tests/index_suite/conversations.rs`: three messages of one thread matching give one hit with `messages`, the best message, `Matches(3)`; an unthreaded message is its own conversation; `Replied` from `answered`, `Flagged`, `InSubject`, `InFileName`; Best match vs Newest orders; `offset`/`limit` page; `total` counts conversations; `messages_searched`. Red: no `search_conversations`
- [x] T021 [US2] Implement `executor::search_conversations` in `crates/postio-index/src/executor.rs`: `Plan::project` streaming the narrow projection (research R2) into a `Fold`; rank with `rank_score`; hydrate the page; per-page column checks for reasons and sources. `search` and `facets` untouched. Make T020 green; T003 stays green
- [x] T022 [P] [US3] Write failing tests in a new `crates/postio-index/tests/index_suite/facets_one_pass.rs`: every sender, recipient, label, folder, attachment, action and unread count equals the conversation total of the query with that term added (SC-008); months per D4; presets; a capped match marks `capped`. Red: facets empty
- [x] T023 [US3] Fill `SearchFacets` from the same `Fold` (correlated `group_concat` columns for recipients and labels). Make T022 green
- [x] T024 [P] Write failing budgets in `crates/postio-index/tests/index_suite/search_statement_budget.rs`: `search_conversations` issues ≤ 5 statements and the same number over corpora of 100 and 2,000 messages, for a word, an operator, both, and a common word. Red until the count is constant
- [x] T025 Make T024 green (fold any per-hit statement into the projection or the page hydrate). *Green on arrival: T021's walk already reads three statements whatever the corpus (projection, hydrate, folders); nothing to fold*
- [x] T026 [P] [US6] Write failing tests in a new `crates/postio-index/tests/index_suite/relaxations.rs`: `relaxation_counts` gives each variant's conversation total; ≤ 8 statements for 8 variants. Red: no function
- [x] T027 [US6] Implement `executor::relaxation_counts`. Make T026 green

### Session, client, host

- [x] T028 [P] [US2] Write failing tests in a new `crates/postio-session/tests/session_suite/search_passages.rs` over the `.eml` corpus: a word only in quoted history → `Source::Quoted` with its passage; only in the body → `Body`; in the subject → `Subject`; a message with no local body → sources without passages. Red: no `passages`
- [x] T029 [US2] Add `conversations` and `passages` to `crates/postio-session/src/search.rs` (`indexable_text`, `postio_body::quote::text_stretches`, `passage::cut`); resolve facet ids to names (≤ 50 per facet, one read per kind). Make T028 green
- [x] T030 [P] Write a failing test in `crates/postio-host/src/tests.rs`: over the search seed, `Client::conversations` and `Client::passages` answer; `Client::relaxations` drops zero counts and sorts. Red: no requests
- [x] T031 Add `Req::{Conversations, Passages, Relaxations}`, their `Resp`s, `family()` names and `Client` methods in `crates/postio-client/src/{protocol,api}.rs`; route them in `crates/postio-host/src/lib.rs` `answer` through new wrappers in `crates/postio-host/src/search.rs`. Make T030 green
- [x] T032 [P] Write a capability test in `crates/postio-storage/tests/turso_capabilities.rs`: a read future over an fts match dropped after its first row leaves the connection usable and the next read correct. Observe its result before relying on it (research R8). *Observed green, through the readers' pool and on one connection: the capability holds. Not covered: a drop while a step itself is pending on IO, which a small store never shows*
- [x] T033 [P] Write a failing test in `crates/postio-host/src/tests.rs`: a `Conversations` call whose future is dropped stops the host's work (a counting hook sees no hydrate statement after the drop). Red: the host finishes it. *Observed differently: `counting` is thread-local and cannot see the host's workers, so the test holds every reader turn (the search waits for one, never reading a row) and watches the waiting task's hold on the host (`Arc<Inner>`) go when the call is dropped*
- [x] T034 Add `Req::cancellable()` and the `select!` on `answer.closed()` in `Local::call` (`crates/postio-host/src/lib.rs`) for cancellable requests only (D9). Make T033 green

### Bench and report

- [x] T035 Write `crates/postio-bench/benches/search_focus.rs` (and its `[[bench]]` in `crates/postio-bench/Cargo.toml`): the deterministic 20k corpus and shapes of plan.md "Performance plan", timing `search_conversations` and `relaxation_counts` and asserting the 50 ms budget as `search_budget.rs` asserts its own
- [x] T036 Run it on the dev Mac (`cargo bench -p postio-bench --bench search_focus`), and write `docs/notes/<date>-focus-search-step-1.md` with p50/p95 per shape and the statement counts; list it in `docs/engineering-notes.md`. **Stop rule** if over budget
- [x] T037 Grep `crates/postio-gtk` for every changed shared name (`Filter`, `Field`, `filter_condition`, `natural::lower`); commit the phase

**Checkpoint**: the engine answers a conversation search with facets,
passages and relaxations, measured; GTK's path is pinned by T003.

---

## Phase 2: Step 2 — the dropdown: empty, words, show all (screens 01, 03)

**Goal**: the quick layer's empty and words states, with recents and saved
searches, on the Mac.

**Independent test**: US1's.

- [x] T038 [US1] Write storyboards in `storyboards/search/`: `dropdown-opens-empty.toml` (recents, saved, cheat sheet), `dropdown-words-show-all.toml` (top hits, narrow to, Show all focused), `dropdown-esc-returns-to-row.toml`, `dropdown-tab-narrows.toml`
- [x] T039 [P] [US1] Write failing tests in a new `crates/postio-storage/tests/storage_suite/searches.rs`: `remember` upserts by query and trims to 20; `recent` is newest first; `forget` deletes; `seen_up_to`/`mark_seen`; `forget_seen_except`. Red: no tables
- [x] T040 [US1] Add `recent_searches` and `saved_search_seen` to `HEAD` in `crates/postio-storage/src/schema.rs`, copy the old `HEAD` to `crates/postio-storage/tests/schemas/<fingerprint>.sql`, append the `Migration`, and add `crates/postio-storage/src/searches.rs`. Make T039 and the schema tests green
- [x] T041 [P] Write a failing test in `crates/postio-host/src/tests.rs` for `RecentSearches`, `RememberSearch`, `ForgetSearch` and `SavedCounts` (totals; "new" is zero until step 6). Red: no requests
- [x] T042 Add those requests in `crates/postio-client/src/{protocol,api}.rs` and `crates/postio-host/src/{lib,search}.rs`. Make T041 green
  - Deviation: `Req::SavedCounts` is `{ account, today, searches: Vec<(key, query text)> }` (the host parses the text; it needs the scope and today); `RecentSearch` lives in `postio_client::protocol`.
- [x] T043 [P] [US1] Write failing tests in `crates/postio-core/tests/core_suite/one_keymap.rs` and `command_registry.rs`: `ShowAllResults` (`mod+Return`, Search) and `ForgetRecent` (`alt+BackSpace`, Search) resolve on Apple; neither is offered on Freedesktop; `BackToWords`' `alt+BackSpace` is not offered on Apple (D23, D25); the whole keymap resolves on both platforms with no collision. Red: no commands
- [x] T044 [US1] Add the two commands and the `offered_on`/`alternate_offered_on` arms in `crates/postio-core/src/registry.rs`; regenerate `docs/keybindings.md`. Make T043 green
- [x] T045 [P] [US1] Write failing tests in a new `crates/postio-ui/src/search_view.rs`: `cheat_sheet()` is the design's eight entries; `recent_when(at, now)` ("yesterday", "Mon", "21 Sep"); `footer_count(n, elapsed, capped)` ("48 matches · 38 ms", "10,000+ matches · 41 ms"); `narrow_pill` labels. Red: no module
- [x] T046 [US1] Implement those words. Make T045 green
- [x] T047 [P] [US1] Write failing tests in `crates/postio-focus/tests/bar.rs` with `Policy.caps.results_view = true`: the empty bar shows ≤ 3 recents, the pinned saved searches with ⌥1–4 and counts, the cheat sheet and a live-lowered example; words ask `Conversations` (limit 4) and draw top hits, up to four narrow-to pills from the facets, and "Show all N results" highlighted; Tab adds the first pill's term via `edit::apply`; an answer with an old stamp is dropped; ⌥⌫ on a recent forgets it; ↩ on a hit opens it and remembers the query. With `results_view = false` every existing `bar.rs` test passes unchanged. Red: no dropdown
- [x] T048 [US1] Add `Policy.caps.results_view`, `DropdownState`/`Dropdown` and the requests to `crates/postio-focus/src/{lib,bar,perform}.rs`. Make T047 green
  - Deviation: the dropdown's types live in a new `crates/postio-focus/src/dropdown.rs` (state stays in `bar.rs`). `>`, `in:` and `@` keep 009's `BarLines` until their own states (step 8); the empty and words states are `Intent::Dropdown`. `Policy::for_platform(Apple)` has `results_view`; `tests/bar.rs`'s `mac()` turns it off so the 009 tests stay GTK's. Saved counts are keyed by the saved search's name (what `Input::SavedSearches` carries) until step 6 needs the config key. `DropdownView` has `select` (a run moved the highlight) beside `highlight` (the default), runs carry a `RunStyle`, pills an `op`, sections a `note_key`.
- [x] T049 [P] Write a failing test in `crates/postio-focus/src/lib.rs` tests that `Request::lane()` names the search requests' lanes, and in `crates/postio-ffi/tests/ffi_suite/focus_search.rs` that typing `a`,`at`,`atl`,`atla` quickly over the search seed completes at most two `Conversations` reads on the host (the rest aborted) and emits dropdown views in stamp order with no stale one. Red: every keystroke's search completes
- [x] T050 Keep `HashMap<Lane, AbortHandle>` in the driver's `ask` (`crates/postio-ffi/src/focus_list.rs`) and abort a lane's previous task (D8). Make T049 green
  - Deviation: `Request::lane()` landed with T048, so the lib.rs test is a guard (green on arrival). The FFI test types `atlas budget` a letter at a time over the search seed rather than `a`..`atla`: "at" lowers as plain English and two words that find nothing answer too fast to race. Observed red: 10 of 12 keystrokes' searches completed; green: at most 2. Completions are counted by the driver (`focus_search_reads_for_test`).
- [x] T051 [US1] Add `crates/postio-ffi/src/focus_search.rs` with `DropdownViewFfi` and friends (contracts/ffi-search.md), `focus_search_forget`, `focus_search_show_all` (in this step it remembers the query and keeps 009's behaviour), and append `UiEvent::FocusDropdown` in `crates/postio-ffi/src/event.rs`. Test first in `ffi_suite/focus_search.rs`: the empty bar emits a `FocusDropdown` with recents and saved sections
- [x] T052 [P] [US1] Write failing Swift tests in `macos/Tests/PostioKitTests/DropdownKeyboardTests.swift` over the `CommandBarEngine` fake: ↓/↑ walk the selectable rows and skip headers; ↩ calls `focusBarRun(token)` for the highlighted row; ⌘↩ calls `focusSearchShowAll`; Tab calls `focusBarTab`; ⌥⌫ on a recent calls `focusSearchForget`; Esc closes and asks for keyboard home; a new `DropdownViewFfi` keeps the highlight on the same token when it is still there, else takes the view's default. Red: no model
- [x] T053 [US1] Add `macos/Sources/PostioKit/DropdownModel.swift` and `DropdownView.swift` (sections, rows, pills, footer), route search mode in `CommandBarModel.swift`/`CommandBarView.swift` to them, and handle `FocusDropdown` in `macos/Sources/Postio/Engine.swift`. Make T052 green
- [x] T054 [US1] Widen the panel to 860 and grow the field leftward with its right edge 12 from the window's in `macos/Sources/PostioAppKit/CommandBarPanel.swift` and `macos/Sources/Postio/MainWindow.swift` (field 34 tall, radius 8, accent ring and halo; panel 6 below, radius 10, the design's shadow). Test first in `macos/Tests/PostioAppKitTests/` that the panel's frame follows the field's at 1440 and 1024 widths
  - Deviation: T052–T054 landed as one commit (the routing, the keys and the geometry share `CommandBarModel.swift` and `Engine.swift`). The geometry is `CommandBarGeometry` in PostioKit, tested from `PostioAppKitTests/CommandBarGeometryTests.swift`; 009's 640-wide, right-aligned panel tests went with it. The field grows by `NSSearchToolbarItem.preferredWidthForSearchField`; its 34pt height, 8pt radius, accent ring and halo are not built (the toolbar's search field draws its own): see the step-2 note.
- [ ] T055 [US1] Capture screens 01 and 03 (light) and write the step-2 note with every difference listed; fix or explain each
  - Pending: `screencapture` could not create an image from this terminal (no Screen Recording grant, or the display asleep). The note `docs/notes/2026-10-09-focus-search-step-2.md` lists the differences read from the code and says how to take the two captures; tick this when they are taken and the list amended.
- [ ] T056 Commit the phase
  - Everything but T055's captures is committed and pushed. `crates/postio-gtk` names none of the changed shared names (`Capabilities`, `results_view`, the new `Request`/`Reply`/`Intent`/`Input` variants, all `#[non_exhaustive]`); it builds its policy with `Policy::for_platform`, so `results_view` is off there. Tick with T055.

**Checkpoint**: the dropdown's empty and words states work on the Mac;
GTK's bar is unchanged.

---

## Phase 3: Step 3 — the results view (screens 06, 07)

**Goal**: ⌘↩ turns the main window into results, with history.

**Independent test**: US2's.

- [x] T057 [US2] Write storyboards in `storyboards/search/`: `results-enter-and-leave.toml`, `results-esc-ladder.toml`, `results-history-back-forward.toml`, `results-sort-toggles-top-hits.toml`
- [x] T058 [P] [US2] Write failing tests in `crates/postio-core/tests/core_suite/`: `HistoryBack` (`mod+bracketleft`), `HistoryForward` (`mod+bracketright`), `ResultsConversations`/`Files`/`People` (`mod+1`–`3`) resolve on Apple in `Context::Results`; the list's verbs (j k x X a e s l m r ⌫ ↩ !) resolve in `Context::Results`; `SaveSearch` resolves there; none of the new commands is offered on Freedesktop; the keymap resolves on both platforms. Red: no context
- [x] T059 [US2] Add `Context::Results` (`crates/postio-core/src/context.rs`), the commands and their contexts and `offered_on` arms (`crates/postio-core/src/registry.rs`); map it in `postio_ui::keymap`'s `KeyContext`; regenerate `docs/keybindings.md`. Make T058 green
  - Deviation: `toggle_result_order` (alt+o, the Sort toggle) and `search` (`/`, FR-021) also name `Context::Results`, beside the list's verbs and `save_search`. `docs/keybindings.md` documents Linux, so its generator leaves the Results context out of the Where column and the file is unchanged; `UiContext::Results` is appended at the FFI. The Linux golden table gains five unbound rows.
- [x] T060 [P] [US2] Write failing tests in a new `crates/postio-focus/tests/history.rs`: entering results pushes the inbox entry; back restores its cursor and selection; forward restores the results' query, tab, order, cursor and selection; a new search after back drops the forward list; ≤ 50 entries; back with one entry does nothing. Red: no module
- [x] T061 [US2] Add `crates/postio-focus/src/history.rs`. Make T060 green
  - Deviation: the inbox's history entry carries nothing -- the list keeps its own cursor and selection under the results, which never touch them, so going back redraws them as they are (`Cursor::redraw`). Esc's last rung is `History::home`: the inbox even after searches run from the results, with the results Esc left ahead of it for ⌘]. `HistoryBack`/`HistoryForward` name the List context too, so ⌘] reaches forward from the inbox.
- [x] T062 [P] [US2] Write failing tests in a new `crates/postio-focus/tests/results.rs`: `ShowAllResults` enters results with the query; Best match gives Top hits (≤ 3) then month groups, Newest only month groups; rows page through a window; passages are asked for the visible page after it lands and fill it; a filter-button `Toggle` edits the query and re-asks; the Esc ladder (D18): open dropdown → closed; selection → cleared; then leave to the inbox with its cursor and selection; entering results remembers the query (`RememberSearch`). Red: no mode
- [x] T063 [US2] Add `crates/postio-focus/src/results.rs`, the Esc rungs in `surfaces.rs`, and the requests in `perform.rs`. Make T062 green
  - Deviation: T060–T063 landed as one commit (history's entries are the results' snapshots, and Esc's last rung is history's). Every conversation sits in its month group, Top hits too (D4), so the groups' positions come from the timeline's counts before any row is read; Best match asks Top hits (limit 3) and the month groups' first page (Newest, 50) together, and further pages when `results_wanted` asks (8 held). The reads are `Request::ResultsPage`/`ResultsPassages` with no lane: two of one query run at once, so a lane would abort its own. The query the field holds is the dropdown's words lowered and spelled (`ParsedQuery` tokens), so a chip's token is the one `edit::apply` counts. In the results, `x` checks, `!` toggles `has:action`, `mod+s` saves the query, `e`/`E`/`f` write to the focused result and the mail verbs act on it; ⇧X and the pickers wait for step 6. `Intent::ResultsCursor` (not in the contract) moves the focus ring; `Input::ResultsPoint` is a click.
- [x] T064 [P] [US2] Write failing tests in `crates/postio-ui/src/search_view.rs`: `reason_line` ("you replied · 3 matches", "you flagged · 2 matches" (D19), "frequent sender · file name"; at most two plus matches); `source_tag`; `month_group` ("September 2026 · 9"); `count_line`/`sub_line`; `results_footer` ("48 conversations · local index · 41 ms"); `filter_button_label` ("From: Ada Moreno", "Since July"); `accessible_row` (design §5). Red: missing
- [x] T065 [US2] Implement those words. Make T064 green
  - Deviation: `reason_line` leaves out "1 match" (screen 06's "frequent sender · file name"); `sources_tag` joins a row's first two places ("body + Atlas-Q3-budget.xlsx"); `month_title`, `FilterKind` (the bar's eight buttons, `holds`, `has_popover`), `results_hints`, `location`, `tab_count` and the group notes are added for the results view. `location` follows FR-025's single quotes (‘Summary’); screen 06 draws double ones. The move hint is `j/k`, as `hints::pair` spells every pair.
- [x] T066 [US2] Add `QueryViewFfi`, `ResultsViewFfi`, `ResultRowFfi`, `focus_search_edit`, `focus_search_tab`, `focus_search_order`, `focus_search_row(_count)` and the `FocusQuery`, `FocusResults`, `FocusResultsPage`, `FocusLeaveResults` events (`crates/postio-ffi/src/{focus_search,event}.rs`), highlight ranges converted to UTF-16 once. Test first in `ffi_suite/focus_search.rs`: show all over the search seed emits `FocusResults` with groups and `FocusQuery` with chips, and rows read back with runs on "atlas" and "budget"
  - Deviation: highlights cross as runs (`RunFfi { text, highlighted }`, as the dropdown's do), so no UTF-16 offsets are computed at all. `ChipFfi` and `GroupFfi` were taken by the classic search's types: the new records are `QueryChipFfi` and `ResultGroupFfi`. Added beyond the contract: `UiEvent::FocusResultsCursor { position }` (j/k move the focus ring), `focus_search_point(position)` (a click), `ResultsViewFfi.rows`/`cursor`, and `TabFfi`/`MonthBarFfi`/`FilterButtonFfi` field names as built. Show all keeps the query among the recent searches at once when the dropdown has counted it, so Esc before the results land still keeps it.
- [x] T067 [P] [US3] Write failing Swift tests in `macos/Tests/PostioKitTests/SearchQuerySyncTests.swift` over a fake engine: tapping Attachment sends `Toggle{has, attachment}`; the next `QueryViewFfi` shows the chip *and* the solid button; ✕ on a chip sends `Remove{token}` and the button returns to outline; a typed `from:ada` arriving from Rust makes From solid with its label; Swift never builds query text (no string concatenation reaches the engine). Red: no model
- [x] T068 [US3] Add `macos/Sources/PostioKit/SearchQueryModel.swift` and `FilterBarView.swift` (tabs with counts, buttons, Sort). Make T067 green
- [x] T069 [P] [US2] Write failing tests in `macos/Tests/PostioAppKitTests/ChipQueryFieldTests.swift`: chips are drawn before the words in token order; an excluded chip is struck through; Backspace at the start of the words selects the last chip, a second Backspace removes it (one `Remove` call); `/` focuses the field and asks for the dropdown. Red: no field
- [x] T070 [US2] Add `macos/Sources/PostioAppKit/ChipQueryField.swift` (an `NSTextView` with chip attachments, research R11) and the results toolbar in `macos/Sources/Postio/MainWindow.swift` (‹ Inbox with Esc keycap, the field, Save search ⌘S inert until step 6). Make T069 green
  - Deviation: the chip field draws and reports but does not edit text. A click, `/` or typing asks for the dropdown (`search`), and the query is edited as text in the bar's own `BarSearchField`, laid over the chips in the query box while the bar is up (the controller hands the bar the query text, so Swift never spells it); what was typed into the chips is handed to that field. The toolbar swaps its items (compose, sync, search ↔ ‹ Inbox, the query box, Save search) when `FocusResults` opens and `FocusLeaveResults` closes; the box's width is the window's less the other items, recomputed on resize. The chrome's words cross as `focus_search_words()`/`focus_search_checked(n)` (added). Save search is inert until step 6.
- [x] T071 [P] [US2] Write failing tests in `macos/Tests/PostioKitTests/ResultsModelTests.swift`: group headers precede their rows; Top hits rows are 66 tall and others 58; a `FocusResultsPage` re-reads only its range. Red: no model
- [x] T072 [US2] Add `macos/Sources/PostioKit/{ResultsModel,ResultRowView,TimelineView,SearchFooter}.swift` (timeline display only) and `macos/Sources/PostioAppKit/ResultsTable.swift` (view-based `NSTableView` with group rows, the gutter, focus ring and find-yellow runs). Make T071 green
  - Wiring (`Engine.swift`, `MainWindow.swift`) landed with T070: the results pane sits over the inbox column in the window's `ZStack`, as Filtered does, so the list keeps its scroll, cursor and selection; `FocusKeyboardHome` in the results gives the results table the keyboard; `mainContext` is `.results` while they are up.
- [x] T073 [US2] Wire `HistoryBack`/`HistoryForward` to the trackpad swipe (`NSEvent` swipe/`scrollWheel` gesture on the main window) in `macos/Sources/Postio/MainWindow.swift`; Swift reports the gesture as the command, decides nothing
- [x] T074 [US2] Capture screen 06 (light) and 07 (dark), list every difference in the step-3 note, fix or explain each
  - Captured with the demo's own `POSTIO_DEMO_SNAPSHOT` (`screencapture` still has no grant here); the note lists 23 differences. Owed to the Rust side before the review: passages over the search seed are empty or the quoted header rather than the match's window (14), Top hits' group count (13), label pill colours at the boundary (18). A capture with the grant should be taken before the review.
- [x] T075 Grep `crates/postio-gtk` for `KeyContext`, `Context`, `Policy`; commit the phase
  - `crates/postio-gtk` matches no `KeyContext`/`Context` exhaustively and builds its policy with `Policy::for_platform`; step 3's Swift, the `postio-ui` words and the two FFI exports are additive.

**Checkpoint**: results view, history and Esc ladder on the Mac.

---

## Phase 4: Step 4 — filter popovers, timeline and date (screens 08, 09)

**Goal**: narrowing with live preview.

**Independent test**: US3's.

- [x] T076 [US3] Write storyboards `storyboards/search/popover-preview-and-restore.toml`, `timeline-drag-narrows.toml`, `date-words.toml`
- [x] T077 [P] [US3] Write failing tests in `crates/postio-focus/tests/results.rs`: opening From records the query; checking a person re-asks with `from:` added and the list, counts and timeline update; Esc restores the recorded query exactly; ↩ keeps it; ⌥-click adds `-from:`; the popover's own filter narrows its rows locally; From/To list only people in the current facets; Anywhere and Label the same with folders and labels. Red: no popovers
- [x] T078 [US3] Implement popovers in `crates/postio-focus/src/results.rs`. Make T077 green
  - A row is checked by its own input, `Input::PopoverToggle { token, exclude }`, not a `TermEdit` from Swift: the controller decides what a check and a ⌥-click write (toggle; exclude, turn round, or take out). The rows and counts are the facets of the query the popover opened on (held still while its checks narrow the results). Closing is `Intent::Popover(None)`. Two people checked are two `from:` clauses, which the query language ANDs: an open question for the maintainer, recorded in the step-4 note.
- [x] T079 [P] [US3] Write failing tests: `StepRangeBack`/`StepRangeForward` (`alt+Left`/`alt+Right`, Results) in `crates/postio-core/tests/core_suite/`; in `crates/postio-focus/tests/results.rs`, `focus_search_months(3, 5)` sets `after:`/`before:` per US3 scenario 4, ⌥←/⌥→ shift both bounds by a month, and the selected months come back marked; the Date popover's words "since july" show "→ after:2026-07-01" and its presets carry counts. Red: missing
- [x] T080 [US3] Add the commands (`crates/postio-core/src/registry.rs`), the months edit and date words (`results.rs`, through `edit::SetDates` and `natural`), and `postio_ui::search_view::date_presets`. Make T079 green
  - The timeline's bars are the months of the query **without** its dates when the controller has seen them (the last frame of the same query undated), so a range can be dragged or stepped wider than what it narrowed to; a query that arrives dated draws only its own months until the undated one has been asked. Group positions still come from the dated answer. ⌥← with no range selects this month; an open `since` is closed at this month before it steps; nothing steps past this month. A preset writes ISO `after:` (D14 rolling is the Save popover's). `Input::DateWords`/`Input::DatePreset` are the Date popover's inputs.
- [x] T081 [US3] Add `PopoverViewFfi`, `focus_search_popover`, `_popover_filter`, `_popover_done`, `_date_words`, `_months` and `UiEvent::FocusPopover` (`crates/postio-ffi`). Test first in `ffi_suite/focus_search.rs`: open From, toggle a row, close with Esc: the final `FocusQuery` equals the first
  - Deviations from contracts/ffi-search.md: the popover's kind is `FilterKindFfi` (a toggle's kind opens nothing) rather than a `PopoverKindFfi`; a row is checked with `focus_search_popover_toggle(token, exclude)` (the controller decides what Space and ⌥-click write); `focus_search_date_preset(token)` picks a preset; `UiEvent::FocusPopover { view: Option<…> }` closes with `None`; `PopoverRowFfi` carries a label's `color`, `PopoverViewFfi` its `placeholder`, `filter`, `hints`, `words`, `words_hint` and `range` beside `result`; `ResultsViewFfi` gains `timeline_hint` and `timeline_step`.
- [x] T082 [P] [US3] Write failing Swift tests in `macos/Tests/PostioKitTests/TimelineTests.swift`: a drag from bar 3 to bar 5 reports `(3, 5)` once on release, never during; a drag right-to-left reports the same; a click on one bar reports `(n, n)`. Red: no gesture
- [x] T083 [US3] Add `macos/Sources/PostioAppKit/FilterPopover.swift` (`NSPopover` with arrow, anchored to its button, accent ring while open) and `macos/Sources/PostioKit/FilterPopoverViews.swift` (people rows 44 tall with bar and count, date two columns 470 wide, 90-tall chart); the timeline drag in `TimelineView.swift`. Make T082 green
  - `TimelineDrag` (PostioKit, in `TimelineView.swift`) reports a drag once on release; `MonthChart` is the bars for both the timeline and the Date popover. `FilterPopoverModel` holds the last `FocusPopover`; `FilterBarView` takes an `anchor` view per button so the AppKit presenter (`FilterPopover`, an `NSPopover`, transient, no animation) can hang from it without PostioKit naming AppKit. Space toggles the highlighted row only while the popover's field is empty. Avatars take a system hue by the name (drawing only). Demo builds accept `@from`/`@to`/`@date`/`@anywhere`/`@label` in `POSTIO_DEMO_KEYS` as a click on that button, and the snapshot composites the windows over the main one (popovers, the bar's panel).
- [x] T084 Re-run the bench with the preview pattern (the same request re-asked per check) and record it in `docs/notes/<date>-focus-search-step-4.md`. **Stop rule** if over budget
  - **Over budget, stop rule applied, nothing worked round**: every word with `after:` (timeline range 97.7 ms, Date "since" 88.5 ms, relaxations 2.5 s) is #1809 (since #1805); the common word (50.9) and `at` (50.2) are #1809's "few ms slower" on every free-text shape. The preview pattern itself (a person, label or folder checked) is 1-7 ms. Table in docs/notes/2026-10-09-focus-search-step-4.md.
- [x] T085 [US3] Capture screens 08 and 09, list every difference in the step-4 note, fix or explain each; commit the phase
  - Captured with the demo snapshot, which now composites the popover window and takes `@from`/`@date` as clicks; the note lists 19 differences (2 fixed: the previewing line's width, the Date chart's cut labels). Open for the maintainer: two people checked are two ANDed `from:` clauses; "You" among the senders. A capture with the Screen Recording grant (popover material and arrow) is still owed before the review.

**Checkpoint**: filters, popovers and the timeline narrow live.

---

## Phase 5: Step 5 — Quick Look (screen 10)

**Goal**: look inside a result without opening it.

**Independent test**: US4's.

- [x] T086 [US4] Write storyboard `storyboards/search/quick-look-walks-results.toml`
- [x] T087 [P] [US4] Write failing tests in `crates/postio-session/tests/session_suite/search_passages.rs`: `conversation_matches` returns every match in a conversation, oldest first, with "Earlier reply" for quoted ones and each passage. Red: no function
- [x] T088 [US4] Add `conversation_matches` (`crates/postio-session/src/search.rs`) and `Req::ConversationMatches` (client, host). Make T087 green
- [x] T089 [P] [US4] Write failing tests: `QuickLook` (`space`, Results), `NextMatch`/`PrevMatch` (`]`/`[` while Quick Look is open) in `crates/postio-core/tests/core_suite/`; in `crates/postio-focus/tests/results.rs`: Space opens it on the cursor's result; j/k move the cursor and the panel follows; ]/[ move the current card; ↩ opens the message window and closes the panel; `a` archives and the panel shows the next result, or closes when none is left; Space or Esc closes; ⌘Z restores the archived one. Red: missing
- [x] T090 [US4] Add the commands and `QuickLook` state (`crates/postio-core/src/registry.rs`, `crates/postio-focus/src/results.rs`); `QuickLookViewFfi` and `UiEvent::FocusQuickLook` in `crates/postio-ffi`. Make T089 green
- [x] T091 [US4] Add `macos/Sources/PostioAppKit/QuickLookPanel.swift` (floating `NSPanel`, 780×470, radius 14, no dimming) and `macos/Sources/PostioKit/QuickLookBody.swift` (header, subject 22/28, sender line, cards with the current one ringed). Test first in `macos/Tests/PostioKitTests/` that a new view keeps the panel and changes only its content
- [x] T092 [US4] Capture screen 10, list every difference in the step-5 note; commit the phase

---

## Phase 6: Step 6 — selection, bulk actions, save search (screen 12)

**Goal**: act on many results and keep the search.

**Independent test**: US5's.

- [x] T093 [US5] Write storyboards `storyboards/search/results-select-and-archive.toml`, `save-search-popover.toml`
- [x] T094 [P] [US5] Write failing tests in `crates/postio-focus/tests/results.rs`: `x` toggles the cursor's result; ⇧X selects every conversation the query matches as `Aim::Matching { query, except }`; the bulk verbs aim at it; the footer view becomes the bulk bar with the count and keys. And in `crates/postio-host/src/tests.rs`: an archive aimed `Matching` archives exactly what the query matches and one undo restores it. Red: `Everything` is the inbox
- [x] T095 [US5] Add `Aim::Matching` to `crates/postio-focus/src/verbs.rs` and its resolution in the host's verb path (the same match `search_conversations` walks). Make T094 green. *Built as `Everything { query: Some(..) }` (the inbox's predicate gained its query form) sent as `Req::SendMatching`; the host resolves it on the command queue (`postio_session::search::matching`) into one `Messages` target, so one undo takes it back. A verb landing re-asks the results: search spans every folder, so an archived row stays and says `in:Archive`.*
- [x] T096 [P] [US5] Write failing tests in `crates/postio-config/src/filters.rs` and `crates/postio-ui/src/saved_search.rs`: `notify` round-trips and defaults to false; `Verb::Save { query, name, pin, notify }` writes all four and the next free order; a rolling save rewrites date terms to relative (`after:90d`) and a fixed one to ISO (D14), via `edit`. Red: no field
- [x] T097 [US5] Add `notify` to `FilterConfig`, extend `postio_ui::saved_search::Verb`, and `postio_ui::search_view::save_name(query)` ("Atlas budget from Ada"). Make T096 green. *`Verb::Save` also carries `dates: Dates` (as typed, rolling or fixed against a day), applied by `postio_search::edit::Edit::Dates`; `Verb::save(query)` is the sidebar's plain form; `SavedSearch` gained `notify`.*
- [x] T098 [P] [US5] Write failing tests in `crates/postio-host/src/tests.rs`: `SavedCounts` "new" counts matches received after `seen_up_to`; `MarkSeen` clears it; and in `crates/postio-focus/tests/bar.rs`: a notify search's pill shows its badge; running it sends `MarkSeen`. Red: new is zero
- [x] T099 [US5] Implement the "new" counts and `MarkSeen` (client, host, session); refresh the dropdown's counts on `NewMail` for notify searches only. Make T098 green. *Counts are keyed by the `[saved_searches.<key>]` key now (`Input::SavedSearches` carries `postio_ui::saved_search::SavedSearch`, which gained `notify`). A search never viewed has no badge; running a notify one sends `MarkSeen` (`seen_up_to` = the app clock's now). The recount on `NewMail` runs while the dropdown is open.*
- [x] T100 [US5] Add `SaveViewFfi`, `focus_search_save`, `UiEvent::FocusSavePopover` (`crates/postio-ffi`, test first in `ffi_suite/saved_search.rs`: save with notify writes `notify = true` in the session's `config.toml`), and `macos/Sources/PostioKit/SavePopoverView.swift` (364 wide, name, read-only chips, three switches, Cancel / Save ↩), the bulk bar in `SearchFooter.swift`, checkboxes and selection tint in `ResultRowView.swift` *`SaveViewFfi.chips` are worded strings ("from:Ada Moreno"), not `ChipFfi`: they are read-only. The controller owns the words and the switches' first state (`Intent::SavePopover`, `Input::SaveSearchAs`); the FFI writes the save (`Intent::SaveSearch(SaveSearch)`) and a notify search starts seen. Checkboxes and the selection tint were step 3's; the bulk bar gained "⇧X select all N".*
- [x] T101 [US5] Capture screen 12, list every difference in the step-6 note; commit the phase

---

## Phase 7: Step 7 — no results and relaxations (screen 13)

**Goal**: zero results is never a dead end.

**Independent test**: US6's.

- [x] T102 [US6] Write storyboard `storyboards/search/no-results-relaxations.toml`
- [x] T103 [P] [US6] Write failing tests: `PickRelaxation1`–`4` (`1`–`4`, Results) in `crates/postio-core/tests/core_suite/`; in `crates/postio-focus/tests/results.rs`: a zero-hit answer asks `Relaxations` and shows them ordered by count, none zero, at most four; a number key runs that query; `BackToWords` in this state clears filters and keeps the words (D24); the hint becomes "⌘⌫ clears filters"; the chip the focused relaxation loosens is marked focused. Red: missing
- [x] T104 [US6] Implement the no-results state in `crates/postio-focus/src/results.rs` and the commands. Make T103 green
- [x] T105 [P] [US6] Write failing tests in `crates/postio-ui/src/search_view.rs`: `nothing_matches(4)` ("Nothing matches all four filters"), `relaxation_line` for each `Loosen` ("Remove “before March”", "Look for “budget v4” anywhere, not just the subject", "Anyone, not just Ada Moreno"), `searched(18204, contents)` with and without "including attachment contents" (US6 scenario 2), never the server line (S4). Red: missing
- [x] T106 [US6] Implement those words; add `NoResultsViewFfi` and `UiEvent::FocusRelaxations` (`crates/postio-ffi`), and `macos/Sources/PostioKit/NoResultsView.swift` (560 wide, numbered rows in SF Mono). Make T105 green
- [x] T107 [US6] Capture screen 13 (the server line is absent by S4: list it as an explained difference), write the step-7 note; commit the phase

---

## Phase 8: Step 8 — typing intelligence (screens 02, 04, 05)

**Goal**: completion, people autocomplete, and plain English that says what
it understood.

**Independent test**: US7's.

- [x] T108 [US7] Write storyboards `storyboards/search/ghost-completion.toml`, `from-people-autocomplete.toml`, `understood-as-tab-chips.toml`
- [x] T109 [P] [US7] Write failing tests in a new `crates/postio-index/tests/index_suite/completions.rs`: `completions("at")` offers "atlas" as the ghost and first word with its count, the label "Atlas", a list whose id holds it, and files whose names match; `field = Some(From)` offers people ranked by `times_seen + sent_count` (D21), ties by the latest; ≤ 4 statements; bodies are read only when metadata gives fewer than three words. Red: no function
- [x] T110 [US7] Implement `executor::completions` (research R1, D22) and `postio_search::suggest::rank_words`. Make T109 green
- [x] T111 [P] Write a failing test in `crates/postio-host/src/tests.rs` for `Req::Suggest`; add it (client, host, session, cancellable). Red: no request
- [x] T112 [P] [US7] Write failing tests in `crates/postio-focus/tests/bar.rs`: 1–3 characters → Prefix state (ghost, suggestions, top hits so far, Show all), Tab accepts the ghost; `from:` → Operator state listing people, and `focus_search_highlighted` on a person asks for the latest two from them; ↩ makes the chip, `ExcludeSuggestion` (⌥↩) the excluded chip; ⌫ on an empty value returns to words; `label:` and `in:` list labels and folders; plain English → PlainEnglish state with one tile per token and its origin (T014), Tab turns them into chips, `BackToWords` keeps the words. Red: missing
- [x] T113 [US7] Implement those states in `crates/postio-focus/src/bar.rs`, `ExcludeSuggestion` (`alt+Return`, Search, Apple only) in `crates/postio-core/src/registry.rs` (test first in `core_suite`), and `origin_line` in `postio_ui::search_view`. Make T112 green
- [ ] T114 [US7] Add `focus_search_highlighted`, `focus_search_exclude`, the ghost and understood tiles to `DropdownViewFfi` (`crates/postio-ffi`), and draw them in `macos/Sources/PostioKit/DropdownView.swift` and `ChipQueryField.swift` (ghost in tertiary after the caret; SF Mono 14 while an operator is typed). Extend `DropdownKeyboardTests.swift` first: ⌥↩ calls `focusSearchExclude`; moving the highlight onto a person calls `focusSearchHighlighted`; Tab with a ghost calls `focusBarTab`
- [ ] T115 Extend `search_focus.rs` with `completions` for `a`, `at`, `atl` and `from:a`; run it and record it in `docs/notes/<date>-focus-search-step-8.md`. **Stop rule** for 20 ms; a vocabulary table (D22) only after asking
- [ ] T116 [US7] Capture screens 02, 04 and 05, list every difference in the step-8 note; commit the phase

---

## Phase 9: Step 9 — attachment contents and the Files tab (screen 11)

**Goal**: find files by what is in them.

**Independent test**: US8's.

- [ ] T117 [US8] Write storyboard `storyboards/search/files-tab.toml`
- [ ] T118 [P] [US8] Add a fixture to the test of `scripts/checks/check-crate-boundaries.py` proving a `postio-extract` that depends on `turso`, `tokio`, `reqwest` or `gtk4` fails it, then the `postio-extract` rule (FR-052). Red: no rule
- [ ] T119 [US8] Create `crates/postio-extract/` (workspace `members` and `default-members` in the root `Cargo.toml`, from a worktree), depending on `postio-search`, `pdf-extract`, `zip` (`default-features = false`, `deflate-flate2`), `quick-xml`, `encoding_rs`. Run `cargo deny check licenses` and record the result in the commit body (research R5)
- [ ] T120 [P] [US8] Write a fixture generator test in `crates/postio-extract/tests/fixtures.rs` that builds, from invented text, a 3-page PDF, a DOCX, an XLSX with sheets "Summary" and "Q3", a PPTX, a UTF-16 text file, and the hostile set (encrypted PDF, truncated PDF, zip bomb, 200k-row sheet); then failing tests: each format's units and `Location`s ("atlas" at `Sheet{Summary, 14}`, `Page(2)`); each hostile file ends in its `Outcome` within `Limits.max_time`, without a panic escaping. Red: no `extract`
- [ ] T121 [US8] Implement `crates/postio-extract/src/{lib,pdf,ooxml,text,limits}.rs` (`catch_unwind` around `pdf-extract`). Make T120 green. If a hostile fixture overflows the stack, stop and report (research R5)
- [ ] T122 [P] [US8] Write failing tests in a new `crates/postio-index/tests/index_suite/attachment_text.rs`: the `attachments` half creates its tables; `index_attachment_text` writes one row per unit, folded; deleting the message cascades; a version bump drops and rebuilds; `attachments_missing_text` lists only attachments with a `blob_id` and no current row. Red: no half
- [ ] T123 [US8] Add the half to `crates/postio-index/src/index.rs`. Make T122 green
- [ ] T124 [P] [US8] Write failing tests in a new `crates/postio-session/tests/session_suite/attachment_index_pass.rs`: the indexer extracts every downloaded attachment of the search seed; an attachment with no blob is never fetched (the mock backend sees no request); a failed extraction is recorded and the pass moves on; a blob stored later is indexed on the event that says so. Red: no indexer
- [ ] T125 [US8] Add `crates/postio-session/src/attachment_text.rs` and `spawn_attachment_indexer` (`lib.rs`), started by the host beside the body indexer. Logs carry ids and outcomes only. Make T124 green
- [ ] T126 [P] [US8] Write failing tests in `crates/postio-index/tests/index_suite/conversations.rs` and `session_suite/search_passages.rs`: a word only inside the XLSX finds its conversation with `Source::FileContent{location: Sheet{Summary, 14}}` and reason `InFileName`; `contents_complete` is false while any downloaded attachment is unextracted; `executor::search` (GTK) still does not find it (D11, T003 green). Red: no arm
- [ ] T127 [US8] Add the attachment-text arm to `search_conversations` only, and attachment passages to `postio_session::search::passages` with `postio_ui::search_view::location` ("Sheet ‘Summary’, row 14", "Page 2"). Make T126 green; re-run T024's budgets
- [ ] T128 [P] [US8] Write failing tests for `executor::files` (`index_suite/conversations.rs`: one card per matching attachment, with its match) and `Req::Files`/`Req::AttachmentCopy` (`postio-host/src/tests.rs`: the copy lands in the given temp directory and nowhere else). Red: missing
- [ ] T129 [US8] Implement them (index, session, client, host), the Files tab in `crates/postio-focus/src/results.rs` (arrows move, Space asks for a copy and a system preview, ↩ opens the message, `SaveFile` `mod+Down` saves; test first in `tests/results.rs` and `core_suite`), and `FileCardFfi`/`focus_search_file`/`focus_search_quick_look_file` in `crates/postio-ffi`. The copy is deleted when Quick Look closes (FR-053). Make T128 green
- [ ] T130 [US8] Add `macos/Sources/PostioAppKit/FilesGrid.swift` (`NSCollectionView`, 4 columns, 12 gaps) and `FilePreview.swift` (`QLPreviewPanel` data source over the temp copy; ⌘↓ through `NSSavePanel`), and `macos/Sources/PostioKit/FileCardView.swift` (preview from `QLThumbnailGenerator` of the temp copy, type tile, name runs, lines). Test first in `macos/Tests/PostioAppKitTests/` that arrow keys move the grid's selection by one and by four
- [ ] T131 [US8] Capture screen 11, list every difference in the step-9 note (iWork and image text are out of scope: explained); commit the phase

---

## Phase 10: Step 10 — the People tab

**Goal**: people in the results, one key from their mail.

**Independent test**: US9's.

- [ ] T132 [P] [US9] Write failing tests for `executor::people` (`index_suite/conversations.rs`: the distinct correspondents of the matched messages, the user's own addresses excluded, with counts and last dates) and, in `crates/postio-focus/tests/results.rs`, ↩ on a person replaces the query with `from:<address>` and selects the Conversations tab. Red: missing
- [ ] T133 [US9] Implement `executor::people`, `Req::People`, the tab in `results.rs`, `PersonRowFfi`/`focus_search_person`, and `macos/Sources/PostioKit/PeopleListView.swift` (avatar, name, address, count, last date). Make T132 green
- [ ] T134 [US9] Capture the People tab at 1440×900 light; there is no PNG, so compare with design §3.11's text and the Files and Conversations rows' rhythm; note it; commit the phase

---

## Phase 11: Polish and landing

- [ ] T135 [P] Write failing tests in `crates/postio-ui/src/search_view.rs` and `macos/Tests/PostioKitTests/` for accessibility (design §5): the dropdown is a combobox with a listbox, each result row's accessibility label is `accessible_row`, popovers and Quick Look are operable by keys alone. Then make them green
- [ ] T136 [P] Add the search vocabulary (term, chip, relaxation, passage, facet, results view) to `CONTEXT.md`, citing this spec
- [ ] T137 Final sweep: capture all 13 screens at 1440×900 light, and 06 and 07 dark, against their PNGs; every difference listed in `docs/notes/<date>-focus-search-final.md`, none unexplained (SC-005)
- [ ] T138 Final bench run on the dev Mac recorded in the same note (SC-001–SC-003)
- [ ] T139 Run `/speckit-analyze` over spec.md, plan.md and tasks.md and fix what it finds
- [ ] T140 Rebase onto `main` (after #1803 lands), confirm `docs/keybindings.md` regenerates clean and T003 is green, and land with `scripts/issue-land.sh --detach --full-suite` (the Linux CI run is what proves GTK); label the PR `interactions-unreviewed`

---

## Dependencies

- Phase 1 blocks everything. Phases 2 → 3 → 4 are in order (the dropdown,
  then the view it opens, then its filters). Phases 5, 6, 7 each need only
  Phase 3 and may run in any order. Phase 8 needs Phase 2. Phase 9 needs
  Phase 3 (the Files tab) and Phase 1 (the search arm). Phase 10 needs
  Phase 3. Phase 11 last.
- Within a phase, a test task precedes its implementation; `[P]` test tasks
  in one phase can be written together.
- The brief's order is kept for building and screenshots (FR-060); the
  freedom above is for parallel sessions only.
