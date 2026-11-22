---
description: "Task list for spec 014, compose formatting"
---

# Tasks: Compose formatting

**Input**: `specs/014-compose-formatting/`: [spec.md](spec.md), [plan.md](plan.md),
[research.md](research.md), [data-model.md](data-model.md),
[contracts/](contracts), [quickstart.md](quickstart.md)

**This file is the tracker.** These tasks get no issues (CLAUDE.md,
spec-driven work). Tick a box in the commit that finishes it.

**Two parts** (spec D8, the maintainer's Q4 = C):
- **Part A — Build now.** This branch lands it, as one PR with
  `--full-suite`.
- **Part B — Held for ADR 0039.** Every task in it is **blocked on ADR
  0039's native composer**, steps 1 to 3: `EditIntent`/`apply`, the
  composer routed through it, and the native surface passing the composer
  suites. Part B is a later branch (`feature/compose-formatting-native`),
  cut when ADR 0039 has landed. ADR 0039 has no tracking issue yet (plan
  Q5).

**Tests are required** (constitution IV). Each implementation task comes
after the task that writes its test, and that test task names what is seen
red. A test that is green on arrival is a guard and says so.

## Conventions

- `[P]` means the task touches different files and depends on no
  unfinished task. `[USn]` is the spec's user story. `⛔ ADR 0039` marks a
  held task.
- **Commits** end with `Refs: specs/014-compose-formatting` and the task
  ids. Never write a closing keyword.
- **The fast tier carries most of this.** Run `scripts/test-fast.sh`
  between edits, and `cargo nextest run -p <crate> --test <suite> <case>`
  to confirm.
- **Swift** tasks run on the Mac (`macos/CLAUDE.md`), and GTK tasks on
  Linux. CI's `Frontends` check proves the combination.
- **Storyboards** (ADR 0044) are seen red on `main` before they are built.
  `/ux-review` runs before landing.
- **Fixtures** use `example.com` and invented names only. No test reaches
  the network: the fetch is tested against a counting fake.

---

# Part A — Build now

## Phase A1: Setup

- [x] T001 Make GTK load the one editing bridge. `crates/postio-widgets/src/composer/editor.rs` uses `postio_ui::compose::editor_script()`, and `crates/postio-widgets/src/composer/editor.js` and its private `markdown_table_js` are deleted. It is needed now because T033 changes the bridge. First, a guard in `crates/postio-widgets/tests/widgets_suite/composer_document.rs` asserts the script GTK injects equals `postio_ui::compose::editor_script()`; it is red only if the copies have drifted, so say which.
- [x] T002 [P] Correct `docs/PRODUCT.md` §10 to this spec's scope (FR-022), citing `specs/014-compose-formatting`
- [x] T003 [P] Comment on #1485 with D3 (seven heading commands, `mod+alt+0`–`6`, six levels), so its implementer can land that shape directly (research R8)
- [x] T004 [P] Update `docs/PRODUCT.md` §10 for the maintainer's answers: the source view, the chip, and the rich editing held for ADR 0039
- [ ] T005 [P] Write `storyboards/compose/markdown-source.toml` (US7) and `storyboards/compose/remote-image.toml` (US8) from their acceptance scenarios, and see both red on `main` with `scripts/storyboards.sh`

## Phase A2: Foundational (blocks every now-phase story)

### The document

- [x] T006 Write failing unit cases in `crates/postio-body/src/document.rs` for:
  - `HeadingLevel` 1–6, with no narrowing;
  - `ListKind::Ordered { start }`;
  - `ListItem.task`;
  - `Language::parse` (accepts `rust`, `c++`, `objective-c`; refuses `a b`, `"x"`, 33 characters, an empty string);
  - `Table::new` padding and truncating;
  - `Inline::Strikethrough`;
  - `Inline::RemoteImage` accepting only `http`/`https`.

  Rewrite `markdown.rs`'s `headings_narrow_to_three_levels` to expect level 4.
- [x] T007 Implement `data-model.md`'s types in `crates/postio-body/src/document.rs`. `Block::Pre` becomes `Block::Code { language, text }`, and every match in the workspace follows (`grep -rn "Block::Pre\|ordered:" crates/`). The crate doc's closed-set count is updated.
- [x] T008 Write failing cases in `crates/postio-body/tests/body_suite/document.rs` for `parse(to_html(d)) == d` and `to_html(parse(h)) == h` over every row of `data-model.md`'s HTML subset table. Include:
  - the `☐ `/`☑ ` glyph removed on the way in;
  - a GitHub-style checkbox list read as tasks;
  - a `colspan` table narrowing to paragraphs;
  - `data-postio-remote-image` read back as a remote image, while a remote `<img>` still narrows away.
- [x] T009 Implement `to_html` and `editor_html` for the new shapes in `crates/postio-body/src/document.rs`, and their reading in `crates/postio-body/src/parse.rs`. Truncated cells go to `crates/postio-body/src/narrow.rs`'s `Lost`.
- [x] T010 Widen `harden` in `crates/postio-body/src/outgoing.rs` (research R3), adding `data-postio-remote-image` on `a`. First widen the property generator in `crates/postio-body/tests/body_suite/outgoing.rs` to the new shapes, and see "harden changes nothing" fail.
- [x] T011 Write the HTML half of `crates/postio-body/tests/body_suite/formatting_wire.rs` (`contracts/wire.md`), red before T009–T010 and green after. The plain-text half is T019's.

### The registry (now-phase rows only)

- [x] T012 Write failing cases in `crates/postio-core/src/registry.rs`'s tests for:
  - every **Now** row in `data-model.md`'s registry table;
  - no collision in `Composer` or `Everywhere` (FR-005);
  - every terminal-offered id having a `legacy_deliverable` binding;
  - no **Held** id registered.
- [x] T013 Add the ids to `crates/postio-core/src/command.rs` and the rows to `crates/postio-core/src/registry.rs`. If #1485 has landed one `heading` command, reshape it here (D3).
- [x] T014 Put the new `Composer` ids in `M::Format` in `crates/postio-core/src/menu.rs`, with the menu test red first. Regenerate `docs/keybindings.md` (`POSTIO_UPDATE_DOCS=1 cargo test -p postio-ui --test ui_suite keybindings_doc`).

### The sequence table, as a contract

- [x] T015 Write failing cases in `crates/postio-ui/src/editor/markdown.rs` for:
  - `Marker` and `Trigger` (`data-model.md`);
  - longest-first across patterns;
  - the widened FR-068 set;
  - the generated `POSTIO_MARKDOWN` staying filtered to the rows today's bridge implements (research R15).
- [x] T016 Implement the new `Sequence` shape and every row of `contracts/sequences.md` in `crates/postio-ui/src/editor/markdown.rs`. Filter the generator in `crates/postio-ui/src/compose.rs` (`markdown_table_js`).

**Checkpoint**: every construct is in the model, in HTML on the wire, and
in the registry where something answers it.

## Phase A3: User Story 3 — a recipient sees what was written (P1)

- [x] T017 [US3] Write failing cases in `crates/postio-body/src/document.rs` for `to_text` and `to_flowed_text` over every row of research R4, plus the remote image's `alt <url>`
- [x] T018 [US3] Implement them in `crates/postio-body/src/document.rs`. `to_search_text` drops every marker (one case).
- [x] T019 [US3] Add the golden plain-text half to `formatting_wire.rs`, red before T018 and green after
- [x] T020 [P] [US3] Add a `postio-render` snapshot of the golden HTML through the reader's sanitiser, in `crates/postio-render/tests/render_suite/`, with every construct visible
- [x] T021 [P] [US3] Add a send case in `crates/postio-session/tests/session_suite/` against the `MailBackend` mock. The golden document goes out as `multipart/alternative`, and a plain document as `text/plain; format=flowed` alone (FR-018).

## Phase A4: User Story 6 — Markdown in and out, and the terminal (P2, now part)

- [x] T022 [P] [US6] Write failing unit cases for `postio_body::autolink` in `crates/postio-body/src/autolink.rs` (research R2)
- [x] T023 [US6] Implement `crates/postio-body/src/autolink.rs`, and export it
- [x] T024 [US6] Write failing cases in `crates/postio-body/src/markdown.rs`'s `writing` and `reopening` modules, and in `crates/postio-body/tests/body_suite/markdown_parity.rs`, covering:
  - every construct;
  - reference links, indented code and entities;
  - `![x](https://…)` becoming a remote image and `![x](cid:…)` an image;
  - the GFM forms written by `from_document`.

  Rewrite `a_table_stays_the_text_that_was_typed` to expect a table.
- [x] T025 [US6] Implement them in `crates/postio-body/src/markdown.rs`: the `pulldown-cmark` options (research R1), the new events, and the autolink post-pass
- [x] T026 [US6] Move the terminal's marker edits (`wrap`, `toggle_prefix`) from `crates/postio-tui/src/composer.rs` into a new `crates/postio-ui/src/editor/markdown_edit.rs`, as pure functions over text and a selection. The TUI's existing cases move with them first, as guards.
- [x] T027 [US6] Write failing cases in `markdown_edit.rs` for every now-phase formatting command:
  - strikethrough;
  - inline code;
  - headings 1–6 and normal text;
  - code block (a fence);
  - task list, and toggling a task;
  - indent and outdent of a list item;
  - divider;
  - a two-by-two table template;
  - clear formatting (markers removed from the selection).

  Then implement them.
- [x] T028 [US6] Route the new commands to `markdown_edit` in `crates/postio-tui/src/app.rs` and `crates/postio-tui/src/composer.rs`. A TUI case per command is red first (US6 scenario 4).
- [x] T029 [US6] Write a failing case in `crates/postio-widgets/tests/widgets_suite/composer_resume.rs`: a draft holding the golden document reopens to the same document (FR-003)

## Phase A5: User Story 7 — the Markdown source view (P1) 🎯 the interim surface

**Independent test**: `contracts/source-view.md`. `postio-body` property and
unit cases, `composer_markdown_source.rs`, and
`storyboards/compose/markdown-source.toml`.

- [x] T030 [US7] Write the FR-024 property test and the golden spellings table's cases in `crates/postio-body/tests/body_suite/markdown_parity.rs` (`contracts/source-view.md`, "The round trip"). See it fail on the awkward shapes.
- [x] T031 [US7] Implement `normalize`, and the spellings in `from_document`/`to_document`, in `crates/postio-body/src/markdown.rs`. This covers `<br>`, a `<br>` line, `<!-- -->`, `&#32;`, and the `U+E001` quote line.
- [x] T032 [US7] Write failing source-map cases in `crates/postio-body/src/markdown.rs` (`contracts/source-view.md`, "The caret"). Then implement `from_document_mapped` and `to_document_mapped` with `into_offset_iter`.
- [x] T033 [US7] Report the caret as a `DocPosition`, and set it from one, in `crates/postio-ui/data/editor.js`, with the host side in `crates/postio-widgets/src/composer/editor.rs` (research R14). A `composer_document.rs` case is red first. This is the phase's only bridge change.
- [x] T034 [US7] Write failing cases in a new `crates/postio-widgets/tests/widgets_suite/composer_markdown_source.rs` for every row of `contracts/source-view.md`'s surface table, except the Mac
- [x] T035 [US7] Build the view in a new `crates/postio-widgets/src/composer/source_view.rs`: a `GtkTextView` in the editor slot, the strip, and formatting commands through `markdown_edit`. Wire `toggle_markdown_source` and the toolbar's toggle button in `crates/postio-widgets/src/composer/mod.rs`.
- [x] T036 [US7] Add the quote chip: a `GtkTextChildAnchor` on the `U+E001` line, atomic deletion with undo, and `U+E001` stripped from typed and pasted text (FR-025)
- [x] T037 [US7] Add the narrowing notice line with Undo on return (FR-026). Make the view session one undo step in `EditHistory`, and save `body_markdown` from the view and reopen into it (FR-028).
- [x] T038 [US7] Answer every now-phase formatting command in the rich view with its reason and the view's key (FR-029), from one word in `crates/postio-ui/src/compose.rs`. The registry's coverage sweep and a `composer_formatting.rs` case are red first.
- [ ] T039 [US7] Mac:
  - add the now-phase ids to `ComposeCommands.handled` in `macos/Sources/PostioKit/ComposeCommands.swift`, with `ComposeCommandsTests` red first;
  - expose `from_document_mapped`, `to_document_mapped` and `markdown_edit` through `crates/postio-ffi/src/session.rs`;
  - build the source view as an `NSTextView` with the quote chip as an `NSTextAttachment`, with Swift tests first.
- [ ] T040 [US7] Make `storyboards/compose/markdown-source.toml` green. Run the PNG loop at 1440 × 900 and 1024 × 768, light and dark, and `/ux-review`.

## Phase A6: User Story 8 — remote images, on a press (P2)

**Independent test**: `contracts/remote-image.md`. A counting fake
`Fetcher`, `composer_remote_image.rs`, and
`storyboards/compose/remote-image.toml`.

- [x] T041 [US8] Write the privacy cases (`contracts/remote-image.md`, "The privacy cases") in `crates/postio-session/tests/session_suite/remote_image.rs`, against a counting fake `Fetcher`. They are red because nothing exists yet.
- [x] T042 [US8] Implement the `Fetcher` trait and `attaching::fetch_remote_image` in `crates/postio-session/src/attaching.rs`, with the production `Fetcher` on `io-http` (research R13). Add the limits, and the magic-byte check refusing SVG.
- [x] T043 [US8] Write failing cases for every row of "The outcomes", then implement the refusals. A remote image is replaced by an `Image` as one undo step.
- [x] T044 [P] [US8] Add the call-site check: `fetch_remote_image`'s only caller is the `AttachRemoteImage` handler. Put it in `scripts/checks/check-remote-image-fetch-site.py`, wired into `scripts/check.sh`, with its own test in `scripts/tests/`.
- [x] T045 [US8] Write failing cases in a new `crates/postio-widgets/tests/widgets_suite/composer_remote_image.rs` for "The composer". Then build the "Not attached yet" rows in the attachment list, the button with its keycap, remove-to-link, "Downloading…", and the warning line before Send, in `crates/postio-widgets/src/composer/mod.rs`. The words go in `crates/postio-ui/src/compose.rs`.
- [x] T046 [US8] Add a log case: the fetch logs the draft id, the outcome and the byte count, and never the address
- [ ] T047 [P] [US8] Draw the same row in the terminal composer's attachment list, `crates/postio-tui/src/view/composer.rs`, with TUI cases first
- [ ] T048 [US8] Mac: claim `attach_remote_image` in `ComposeCommands`, expose the fetch through `crates/postio-ffi/src/session.rs`, and draw the row in `macos/Sources/PostioKit/ComposeColumn.swift`, with tests first
- [ ] T049 [US8] Make `storyboards/compose/remote-image.toml` green, and run `/ux-review`

## Phase A7: Polish, and land the now phase

- [ ] T050 [P] Cite this spec beside spec 002 in the module docs of `crates/postio-ui/src/editor/markdown.rs` and `crates/postio-body/src/document.rs`. `docs/archive/` is frozen (`check-archive-frozen.py`), so spec 002 is not edited.
- [ ] T051 [P] Add cross-references to ADR 0039 and ADR 0004:
  - ADR 0039 names Part B as work waiting on it, the R6 edit ops as its `apply`'s first arms, and tables in a native text view as a second thing to spike;
  - ADR 0004's closed-set count is updated, and D5 cited.
- [ ] T052 [P] Record the recipient-client results (`quickstart.md` §7) in `docs/notes/2026-<mm>-<dd>-formatting-in-recipient-clients.md`, and list it in `docs/notes/README.md`. The maintainer runs the send.
- [ ] T053 Run `quickstart.md` end to end, then `scripts/target-sweep.py`. Land with `scripts/issue-land.sh --detach --full-suite --title 'feat(body,ui,widgets): compose every Markdown construct'`.

---

# Part B — Held for ADR 0039 ⛔

**Blocked until** ADR 0039's steps 1–3 have landed: `EditIntent`/`apply` in
`postio-body`, the composer routed through it, and the native surface
passing the composer suites. The paths below name the native surface as
`crates/postio-widgets/src/composer/native/`, which ADR 0039 will settle.
On a later branch, `feature/compose-formatting-native`.

## Phase B1: Foundational (held)

- [ ] T054 ⛔ ADR 0039. Register the **Held** rows of `data-model.md` in `crates/postio-core/src/registry.rs`, with `Context::TablePicker`, cases first. These are `code_language`, `line_break`, `formatting`, `paste_markdown`, the table picker's ten, and `picker_choose_0`, `5`–`9`.
- [ ] T055 ⛔ ADR 0039. Widen the picker frame's keys to `picker_choose_0`–`9` in `crates/postio-widgets/src/widgets/pickers/mod.rs`. A guard shows the four existing pickers ignore the new keys.
- [ ] T056 ⛔ ADR 0039. Write failing cases, then add the edit ops as arms of ADR 0039's `apply` in `crates/postio-body/` (research R6). Each is pinned linear in its block:
  - inline code over a selection;
  - toggle task list, and toggle task;
  - set a list's start, and set a code language;
  - indent and outdent;
  - insert a table, add or delete a row or column, align a column, delete a table.

## Phase B2: User Story 1 — type the inline constructs (P1) ⛔

- [ ] T057 ⛔ ADR 0039. Write failing native-composer cases for US1's scenarios 1–7 in `crates/postio-widgets/tests/widgets_suite/composer_formatting.rs`, and `storyboards/compose/formatting-inline.toml` red on `main`
- [ ] T058 ⛔ ADR 0039. Implement the inline rows of `contracts/sequences.md` on the native surface: wrappings, links, autolinks, escapes and flanking, and "never inside code or a carried quote". Draw strikethrough and inline code. Retire their FR-029 refusals. Make the storyboard green.

## Phase B3: User Story 2 — type the block constructs (P1) ⛔

- [ ] T059 ⛔ ADR 0039. Write failing cases for US2's scenarios 1–8, and `storyboards/compose/formatting-blocks.toml` red
- [ ] T060 ⛔ ADR 0039. Implement the block rows (`LinePrefix`, `LineComplete`), Tab and Shift+Tab in list items, `line_break`, and the drawing of each block. This covers the heading scale, the code ground and its label, nested quote rules, and the task box as a `role="checkbox"` control toggled by click. Retire their refusals.
- [ ] T061 ⛔ ADR 0039. Build the code language picker's words in `crates/postio-ui/src/format_picker.rs`, cases first, and the picker on the frame

## Phase B4: User Story 4 — the formatting picker and toolbar (P2) ⛔

- [ ] T062 ⛔ ADR 0039. Write failing cases, then implement `crates/postio-ui/src/format_picker.rs` (`contracts/formatting-picker.md`, "Model")
- [ ] T063 ⛔ ADR 0039. Write failing `crates/postio-gtk/tests/focus_suite/formatting_picker.rs` cases for US4's scenarios 1–6, and `storyboards/compose/formatting-picker.toml` red
- [ ] T064 ⛔ ADR 0039. Build the picker at the caret on the frame, and the toolbar's new buttons, their reflection and the overflow rule. Then the Mac's SwiftUI picker, and `/ux-review`.

## Phase B5: User Story 5 — tables (P2) ⛔

- [ ] T065 ⛔ ADR 0039. Write failing cases for US5's scenarios 1–5, and `storyboards/compose/formatting-table.toml` red
- [ ] T066 ⛔ ADR 0039. Implement the delimiter-row sequence, `table` opening the table picker inside a table, the table picker, and cell navigation (Tab, Enter, Shift+Enter). Draw the grid, then run the PNG loop and `/ux-review`.

## Phase B6: User Story 6 and 8, the rich parts ⛔

- [ ] T067 ⛔ ADR 0039. Write failing cases for US6's scenarios 1–2, then implement `paste_markdown` on the native surface (FR-019)
- [ ] T068 ⛔ ADR 0039. Draw the remote image chip inline at the image's place, pressing the same `attach_remote_image`, with cases first
- [ ] T069 ⛔ ADR 0039. Assert no FR-029 refusal remains: every formatting command is answered in the rich view. Then land Part B with `--full-suite`.

---

## Dependencies and order

### Part A

- **Setup**: T001 comes before T033. T002 and T003 are done. T004 and T005
  are independent.
- **Foundational**: T006 → T007 → T008 → T009 → T010 → T011; T012 → T013
  → T014; T015 → T016.
- **US3** (T017–T021) needs Foundational only.
- **US6** (T022–T029): T022–T025 need Foundational, and T026 → T027 →
  T028.
- **US7** (T030–T040) needs T025 and T027. Within it, T030 → T031 → T032
  → T033 → T034 → T035 → T036 → T037 → T038. T039 (the Mac) can follow
  T032 on the Mac.
- **US8** (T041–T049) needs T025 (remote images from Markdown). It can run
  beside US7, except T045, which shares `composer/mod.rs` with T035–T038.
  Run T045 after T038.
- **Polish** last. T053 lands Part A.

### Part B

- Everything is blocked on ADR 0039. Then: T054–T056, then US1 → US2 →
  US4 → US5 → T067–T069.

## Parallel examples (Part A)

- After Foundational: T017 (plain text), T022 (autolinks), T026 (moving
  the marker edits) and T041 (the privacy cases) run in parallel. They
  touch four crates.
- Within US8: T044 (the check) and T047 (the terminal row).

## Implementation strategy

- **MVP**: Part A's Setup, Foundational, US3 and US6. Every construct is
  in the model, sent well, and authored in the terminal.
- **Then US7**: GTK and Mac users author every construct through the
  source view. This is the interim surface, and the maintainer's
  sentence holds from here.
- **Then US8**: remote images, on a press.
- **Part B** follows ADR 0039, and replaces each FR-029 refusal with the
  real thing.
