---
description: "Task list for spec 014, compose formatting"
---

# Tasks: Compose formatting

**Input**: `specs/014-compose-formatting/`: [spec.md](spec.md), [plan.md](plan.md),
[research.md](research.md), [data-model.md](data-model.md),
[contracts/](contracts), [quickstart.md](quickstart.md)

**This file is the tracker.** These tasks get no issues (CLAUDE.md,
spec-driven work). Tick a box in the commit that finishes it.

**No task starts until the maintainer has reviewed the spec** and answered
Q1–Q4, or accepted the assumed options. `plan.md` ("If Q1–Q4 go the other
way") says which tasks change.

**Tests are required** (constitution IV). Each implementation task comes
after the task that writes its test, and that test task names what is seen
red. A test that is green on arrival is a guard and says so.

## Conventions

- `[P]` means the task touches different files and depends on no
  unfinished task. `[USn]` is the spec's user story.
- **Commits** end with `Refs: specs/014-compose-formatting` and the task
  ids. Never write a closing keyword.
- **The fast tier carries most of this feature.** `scripts/test-fast.sh`
  after each edit; `cargo nextest run -p <crate> --test <suite> <case>` to
  confirm. The landing is `scripts/issue-land.sh --detach --full-suite
  --title 'feat(body,ui,widgets): compose every Markdown construct'`.
- **Swift** tasks run on the Mac (`macos/CLAUDE.md`); GTK tasks on Linux.
  CI's `Frontends` check proves the combination.
- **Storyboards** (ADR 0044) are written from the story, seen red on
  `main`, then built. `/ux-review` runs before landing.
- **Fixtures** use `example.com` and invented names only.

---

## Phase 1: Setup

- [ ] T001 Make GTK load the one editing bridge. `crates/postio-widgets/src/composer/editor.rs` uses `postio_ui::compose::editor_script()`, and `crates/postio-widgets/src/composer/editor.js` and its private `markdown_table_js` are deleted. First, a guard in `crates/postio-widgets/tests/widgets_suite/composer_document.rs` asserts the script GTK injects equals `postio_ui::compose::editor_script()`; it is red only if the copies have drifted, so say which.
- [x] T002 [P] Correct `docs/PRODUCT.md` §10 to this spec's scope (FR-022), citing `specs/014-compose-formatting`
- [x] T003 [P] Comment on #1485 with D3 (seven heading commands, `mod+alt+0`–`6`, six levels), so its implementer can land that shape directly (research R8)
- [ ] T004 [P] Create `storyboards/compose/formatting-inline.toml`, `formatting-blocks.toml`, `formatting-picker.toml` and `formatting-table.toml` from US1, US2, US4 and US5's acceptance scenarios, and see each red on `main` with `scripts/storyboards.sh`

---

## Phase 2: Foundational (blocks every story)

**Purpose**: the widened document and its HTML, the registry rows, the
sequence table's new shape and the picker frame's keys. After this phase,
every new construct exists in the model and on the wire with no surface.

### The document

- [ ] T005 Write failing unit cases in `crates/postio-body/src/document.rs` for:
  - `HeadingLevel` 1–6 round-tripping through `digit` and `from_digit`, with no narrowing;
  - `ListKind::Ordered { start }`;
  - `ListItem.task`;
  - `Language::parse` (accepts `rust`, `c++`, `objective-c`; refuses `a b`, `"x"`, 33 characters, an empty string);
  - `Table::new` padding and truncating ragged rows;
  - `Inline::Strikethrough`.

  The existing `headings_narrow_to_three_levels` case in `markdown.rs` is rewritten to expect level 4.
- [ ] T006 Implement the types of `data-model.md` in `crates/postio-body/src/document.rs`. `Block::Pre` becomes `Block::Code { language, text }`, and every match in the workspace follows (`grep -rn "Block::Pre\|ordered:" crates/`). The crate-level doc's closed-set count is updated.
- [ ] T007 Write failing cases in `crates/postio-body/tests/body_suite/document.rs` for `parse(to_html(d)) == d` and `to_html(parse(h)) == h`, over every row of `data-model.md`'s HTML subset table. Include the `☐ `/`☑ ` glyph being removed on the way in, a GitHub-style `<input type=checkbox checked>` list read as tasks, and a `colspan` table narrowing to paragraphs.
- [ ] T008 Implement `to_html` and `editor_html` for the new shapes in `crates/postio-body/src/document.rs`, and their reading in `crates/postio-body/src/parse.rs` (`<del>`, `<s>`, `<strike>`, `h4`–`h6` kept, `<ol start>`, `data-postio-task`, `data-postio-lang`, simple tables). Truncated cells are reported through `crates/postio-body/src/narrow.rs`'s `Lost`.
- [ ] T009 Widen `harden` in `crates/postio-body/src/outgoing.rs` (research R3): `h4`–`h6`, `del`, `start` on `ol`, and the `data-postio-*` attributes on `pre`, `ul` and `li`. First widen the property generator in `crates/postio-body/tests/body_suite/outgoing.rs` so it produces the new shapes, and see "harden changes nothing" fail.
- [ ] T010 Write `crates/postio-body/tests/body_suite/formatting_wire.rs` with the HTML half of `contracts/wire.md`'s golden document, red before T008 and T009 and green after. The plain-text half is T034's to add, as its failing case. It is not added here behind `#[ignore]`, which means only "this machine may lack what the test needs" (CLAUDE.md).

### The registry

- [ ] T011 Write failing cases in `crates/postio-core/src/registry.rs`'s tests for:
  - every id in `data-model.md`'s registry table, with its title, keys, context and requirement;
  - no collision in `Composer`, `Everywhere`, `Picker` or `TablePicker` (FR-005);
  - every `Composer` id offered in the terminal having a `legacy_deliverable` binding.
- [ ] T012 Add the ids to `crates/postio-core/src/command.rs`, the rows to `crates/postio-core/src/registry.rs`, and `Context::TablePicker` to `crates/postio-core/src/context.rs`. If #1485 has landed one `heading` command, reshape it into `heading_1`–`heading_6` and `normal_text` here (D3).
- [ ] T013 Put every new `Composer` id in `M::Format` in `crates/postio-core/src/menu.rs`, and the table picker's and choose keys in no menu. The menu test is updated first and seen red.
- [ ] T014 Regenerate `docs/keybindings.md` (`POSTIO_UPDATE_DOCS=1 cargo test -p postio-ui --test ui_suite keybindings_doc`). The diff is only the new rows.

### The sequence table and the picker frame

- [ ] T015 Write failing cases in `crates/postio-ui/src/editor/markdown.rs` for:
  - `Marker` and `Trigger` as in `data-model.md`;
  - longest-first across patterns;
  - the widened FR-068 set (`every_sequence_reaches_a_formatting_command_and_nothing_else`);
  - retiring `no_heading_sequence_until_there_is_a_heading_command` once the heading ids exist.
- [ ] T016 Implement the new `Sequence` shape in `crates/postio-ui/src/editor/markdown.rs`. Extend the generator in `crates/postio-ui/src/compose.rs` (`markdown_table_js`) to emit patterns and the two new triggers. `editor.js` still ignores them, which is a guard for the next phases.
- [ ] T017 [P] Widen the picker frame's keys to `picker_choose_0`–`9` in `crates/postio-widgets/src/widgets/pickers/mod.rs`. A guard in the existing picker cases shows snooze, remind, label and move ignore `5`–`9` and `0`.

**Checkpoint**: every construct is in the model, on the wire (HTML), in the
registry and the key map. Nothing is typed or drawn yet.

---

## Phase 3: User Story 1 — type the inline constructs (P1) 🎯 MVP

**Goal**: strikethrough, inline code, `__bold__`, `***both***`, links and
autolinks, and escapes, typed and by key. Intraword `_` is safe.

**Independent test**: `crates/postio-widgets/tests/widgets_suite/composer_formatting.rs`
types each inline row of `contracts/sequences.md`. It asserts the
document, the sent parts and one-undo, plus `storyboards/compose/formatting-inline.toml`.

- [ ] T018 [P] [US1] Write failing unit cases for `postio_body::autolink` in `crates/postio-body/src/autolink.rs` (research R2): GFM §6.9's examples, trailing punctuation, parenthesis balance, `www.` getting `http://`, and an address becoming `mailto:`
- [ ] T019 [US1] Implement `crates/postio-body/src/autolink.rs`, and export it from `crates/postio-body/src/lib.rs`
- [ ] T020 [US1] Add the inline rows to `SEQUENCES` in `crates/postio-ui/src/editor/markdown.rs`, with the flanking rule as a pure function the script calls through the generated table. Unit cases first: `snake_case_name` is not converted, `2 * 3 * 4` is not, `~5` is not.
- [ ] T021 [US1] Write failing cases in `crates/postio-widgets/tests/widgets_suite/composer_formatting.rs` for US1's acceptance scenarios 1–7, through the real composer
- [ ] T022 [US1] Implement in `crates/postio-ui/data/editor.js`:
  - the `~~`, `__`, `***`, `___` and backtick-run wrappings;
  - `[text](url)` and `<url>` closing;
  - `WordComplete` autolinks, calling a generated predicate that mirrors R2, or posting the word to the host for `autolink`;
  - escapes and the "never inside code or a carried quote" guard.

  Strikethrough runs through `execCommand('strikeThrough')`, with the element form pinned to `<del>` by a dialect case in `crates/postio-widgets/tests/widgets_suite/composer_document.rs`.
- [ ] T023 [US1] Add `strikethrough`, `inline_code` and `clear_formatting` to `postio_ui::compose::mark_script` in `crates/postio-ui/src/compose.rs`. Inline code over a selection is an edit op (T024).
- [ ] T024 [US1] Write failing cases, then implement `toggle_inline_code(&Document, Selection) -> Document`, in a new `crates/postio-body/src/edit_ops.rs` (research R6)
- [ ] T025 [US1] Wire the commands in `crates/postio-widgets/src/composer/mod.rs`. Add the Strikethrough and Inline code toolbar buttons (`contracts/formatting-picker.md`, "The toolbar"). Extend the reflection channel in `editor.js` and `crates/postio-widgets/src/composer/editor.rs` for `strikethrough` and `inline_code` (FR-014). A toolbar case in `crates/postio-widgets/tests/widgets_suite/composer_toolbar.rs` is first and red.
- [ ] T026 [US1] Make `storyboards/compose/formatting-inline.toml` green, and render the composer with each inline construct (`/gtk-design`'s PNG loop), light and dark

**Checkpoint**: US1 is usable on GTK. The Mac gets the sequences through
the shared script (verified in US6).

---

## Phase 4: User Story 2 — type the block constructs (P1)

**Goal**: headings 1–6 (ATX and setext), nested quotes, list starts,
nesting by Tab, task lists, fenced code with a language, dividers and line
breaks.

**Independent test**: `composer_formatting.rs`'s block cases, and
`storyboards/compose/formatting-blocks.toml`.

- [ ] T027 [US2] Add the block rows to `SEQUENCES` in `crates/postio-ui/src/editor/markdown.rs` (`LinePrefix` headings, tasks, `+`, ordered pattern; `LineComplete` setext, divider, fence). Unit cases first: `---` under a one-line paragraph after a line break is H2, and alone in a block is a divider.
- [ ] T028 [US2] Write failing edit-op cases, then implement in `crates/postio-body/src/edit_ops.rs`:
  - `toggle_task_list`;
  - `toggle_task`;
  - `set_list_start`;
  - `set_code_language`;
  - `indent` and `outdent` over lists and quotes, with quote nesting.
- [ ] T029 [US2] Write failing cases in `composer_formatting.rs` for US2's acceptance scenarios 1–8. Then implement:
  - `editor.js`'s `LineComplete` trigger, the ordered pattern, and `> ` inside a quote reaching `indent`;
  - Tab and Shift+Tab inside list items running `indent` and `outdent`, with Tab elsewhere leaving the body as today;
  - the heading, `normal_text`, `divider`, `code_block` and `line_break` marks through `formatBlock`, `insertHorizontalRule` and `insertLineBreak` in `mark_script`;
  - the edit ops through the host's re-render-and-restore-caret path in `crates/postio-widgets/src/composer/editor.rs`.
- [ ] T030 [US2] Draw the new blocks in `crates/postio-ui/data/editor.css`:
  - the heading scale for h4–h6 from the reader's tokens;
  - the code block ground and its language label;
  - nested quote rules;
  - the task box as a `role="checkbox"` control over the glyph, which a click toggles through `toggle_task`.

  Check them with the PNG loop, light and dark.
- [ ] T031 [US2] Add the Task list toolbar button. Extend the reflection channel for headings, `task_list`, `code_block` and `divider`.
- [ ] T032 [US2] Write failing cases for the code language picker's words in `crates/postio-ui/src/format_picker.rs` (`acts_on` "Code block", the last four languages as presets, the refusal reason for a bad name). Then implement it, and its GTK use on the picker frame in `crates/postio-widgets/src/composer/mod.rs`.
- [ ] T033 [US2] Make `storyboards/compose/formatting-blocks.toml` green

---

## Phase 5: User Story 3 — a recipient sees what was written (P1)

**Goal**: D6's plain text, the golden document whole, and Postio's own
reader showing it.

**Independent test**: `crates/postio-body/tests/body_suite/formatting_wire.rs`,
both halves.

- [ ] T034 [US3] Write failing cases in `crates/postio-body/src/document.rs` for `to_text` and `to_flowed_text` over every row of research R4. Cover:
  - setext underlines at the heading's width;
  - `~~`;
  - `- [x] `;
  - `N. ` from a start;
  - fenced code, stuffed and never reflowed;
  - the padded pipe table with its alignment row;
  - nested quotes as `>> `.

  Add the golden plain-text half to `formatting_wire.rs` here too, red.
- [ ] T035 [US3] Implement them in `crates/postio-body/src/document.rs`. `to_search_text` drops every marker (one case). T034's cases, the golden half included, go green.
- [ ] T036 [US3] Add a `postio-render` snapshot of the golden document's HTML through the reader's sanitiser, in `crates/postio-render/tests/render_suite/`. Every construct must be visible: the task glyphs, the table borders, the struck words.
- [ ] T037 [US3] Add a send case in `crates/postio-session/tests/session_suite/` against the `MailBackend` mock. The golden document goes out as `multipart/alternative`, and a plain document as `text/plain; format=flowed` alone (FR-018).

**Checkpoint**: P1 complete. Everything typed reaches recipients correctly.

---

## Phase 6: User Story 4 — the formatting picker and toolbar (P2)

**Goal**: every construct findable from `mod+alt+f` and the toolbar's last
button, with keys and Markdown taught in the rows.

**Independent test**: `crates/postio-gtk/tests/focus_suite/formatting_picker.rs`,
and `storyboards/compose/formatting-picker.toml`.

- [ ] T038 [US4] Write failing unit cases for `crates/postio-ui/src/format_picker.rs` (`contracts/formatting-picker.md`, "Model"):
  - the groups and their order;
  - filtering by title prefix and by sequence;
  - `acts_on` for each caret kind;
  - `in_force`;
  - the unavailable reasons;
  - the empty line.
- [ ] T039 [US4] Implement `crates/postio-ui/src/format_picker.rs`, and export it from `crates/postio-ui/src/lib.rs`
- [ ] T040 [US4] Write failing cases in `crates/postio-gtk/tests/focus_suite/formatting_picker.rs` for US4's acceptance scenarios 1–6, with real keys
- [ ] T041 [US4] Build the formatting picker on the picker frame in `crates/postio-widgets/src/composer/mod.rs`, anchored at the caret's line (from the bridge's caret rectangle) or below the toolbar button. Each row draws its keycap with `hints::short` from the keymap in force.
- [ ] T042 [US4] Add the toolbar's Formatting button, and the overflow rule (buttons leave from Image leftwards; Formatting stays). A `composer_toolbar.rs` case at the 615 px column is first and red.
- [ ] T043 [US4] Make `storyboards/compose/formatting-picker.toml` green. Run `/ux-review` on US1, US2 and US4's storyboards, and fix or contest every finding.

---

## Phase 7: User Story 5 — tables (P2)

**Goal**: type a table, edit its structure and alignment, Tab through
cells.

**Independent test**: `postio-body` edit-op cases, `composer_formatting.rs`'s
table cases, and `storyboards/compose/formatting-table.toml`.

- [ ] T044 [P] [US5] Write failing cases, then implement in `crates/postio-body/src/edit_ops.rs`:
  - `insert_table` (two by two, with a header);
  - `add_row` above and below;
  - `add_column` left and right;
  - `delete_row` and `delete_column`, where the last one deletes the table;
  - `align_column`, where repeating clears it;
  - `delete_table`.

  Each is pinned linear in its table on a 1,000-block document (research R12).
- [ ] T045 [US5] Add the `DelimiterRow` pattern's row to `SEQUENCES` (unit cases first: the cell count must match the pipe row above), and its recogniser in `crates/postio-ui/data/editor.js`
- [ ] T046 [US5] Write failing cases in `composer_formatting.rs` for US5's acceptance scenarios 1–5. Then implement in `crates/postio-widgets/src/composer/`:
  - `table` (insert, or open the table picker);
  - the table picker on the frame, with `Context::TablePicker`'s commands;
  - Tab and Shift+Tab between cells, with Tab in the last cell adding a row;
  - Enter moving down a column, and Shift+Enter breaking a line in a cell.
- [ ] T047 [US5] Draw tables in `crates/postio-ui/data/editor.css` (hairline grid, bold header, alignment), and check them with the PNG loop at 1440 × 900 and 1024 × 768
- [ ] T048 [US5] Make `storyboards/compose/formatting-table.toml` green, and run `/ux-review` on it

---

## Phase 8: User Story 6 — paste Markdown, reopen, every composer (P2)

**Goal**: Paste as Markdown. Drafts reopen identical. The terminal and the
Mac author every construct.

**Independent test**: `postio-body` round trips, TUI unit cases, the Mac's
`ComposeCommandsTests`.

- [ ] T049 [US6] Write failing cases in `crates/postio-body/src/markdown.rs`'s `writing` and `reopening` modules, and in `crates/postio-body/tests/body_suite/markdown_parity.rs`. Every construct, reference links, indented code and entities must read in. `from_document` must write the GFM forms. `to_document(from_document(d)) == d` over the golden document. Rewrite `a_table_stays_the_text_that_was_typed` to expect a table.
- [ ] T050 [US6] Implement them in `crates/postio-body/src/markdown.rs`: the `pulldown-cmark` options (research R1), the new events, and the autolink post-pass
- [ ] T051 [US6] Write failing cases in `crates/postio-widgets/tests/widgets_suite/composer_formatting.rs` for `paste_markdown` and for an ordinary paste staying literal (US6 scenarios 1–2). Then implement `paste_markdown` in `crates/postio-widgets/src/composer/mod.rs`, inserting `to_document`'s blocks at the caret through an edit op.
- [ ] T052 [US6] Write a failing case in `crates/postio-widgets/tests/widgets_suite/composer_resume.rs`: a draft with the golden document reopens to the same document (FR-003)
- [ ] T053 [P] [US6] Write failing cases in `crates/postio-tui/src/composer.rs`'s tests. Then add `Format` arms for every new command the terminal offers (`~~` wraps, `- [ ] ` prefix, `#`×n prefix, fences, `---`, a table template, indent/outdent by two spaces), and route them in `crates/postio-tui/src/app.rs`.
- [ ] T054 [US6] Mac: add the new marks to `ComposeCommands.marks` and `handled` in `macos/Sources/PostioKit/ComposeCommands.swift`, with `ComposeCommandsTests` red first. Expose the edit ops and `format_picker` through `crates/postio-ffi/src/session.rs`. Draw the formatting, table and code language pickers in SwiftUI from `postio_ui::format_picker` (`macos/Sources/PostioKit/`), with Swift tests first.

---

## Phase 9: Polish and cross-cutting

- [ ] T055 [P] Cite this spec beside spec 002 in the module docs of `crates/postio-ui/src/editor/markdown.rs` and `crates/postio-body/src/document.rs`, as the owner of the sequence set and of the closed set's size. `docs/archive/` is frozen (`check-archive-frozen.py`), so spec 002 itself is not edited.
- [ ] T056 [P] Add ADR 0039 and ADR 0004 cross-references to this spec. ADR 0039's "What would falsify this" gains tables in a native text view as a second thing to spike. ADR 0004's closed-set count is updated, and D5 cited.
- [ ] T057 [P] Record recipient-client results (`quickstart.md` §7) in `docs/notes/2026-<mm>-<dd>-formatting-in-recipient-clients.md`, and list it in `docs/notes/README.md`. The maintainer runs the send.
- [ ] T058 Run `quickstart.md` §1–6 end to end, and `scripts/target-sweep.py`. Land with `scripts/issue-land.sh --detach --full-suite --title 'feat(body,ui,widgets): compose every Markdown construct'`.

---

## Dependencies and order

- **Setup** (T001–T004) first. T001 comes before any `editor.js` edit.
- **Foundational** (T005–T017) blocks every story. Within it:
  - T005 → T006 → T007 → T008 → T009 → T010;
  - T011 → T012 → T013 → T014;
  - T015 → T016;
  - T017 is independent.
- **US1** (T018–T026) then **US2** (T027–T033): they share `editor.js` and
  `composer_formatting.rs`, so they run in order.
- **US3** (T034–T037) depends only on Foundational. It can run beside US1
  and US2, because it touches `document.rs`'s text half, `postio-render`
  and `postio-session` tests.
- **US4** (T038–T043) needs US1 and US2's commands to list. T038–T039 can
  start after Foundational.
- **US5** (T044–T048): T044 can start after Foundational. The rest needs
  US2's edit-op path (T029).
- **US6**: T049–T050 and T053 need only Foundational. T051 and T052 need
  T050. T054 needs US1, US2 and US4.
- **Polish** last.

## Parallel examples

- After Foundational: T018 (autolink), T034 (plain text), T038 (picker
  words), T044 (table ops), T049 (Markdown) and T053 (terminal). These are
  six files in four crates.
- Within US1: T018 beside T020 and T021.

## Implementation strategy

- **MVP**: Setup, Foundational and US1. A person can strike through, code
  and link by typing, and recipients get correct HTML.
- **Then** US2 and US3, which complete P1: every construct typed and sent
  well.
- **Then** US4 (discoverability), US5 (tables) and US6 (paste, terminal,
  Mac).
- **Each phase ends runnable**, committed and green at the fast tier.
  The branch lands once, with `--full-suite`.
