---
description: "Task list for spec 015, the native composer over the document"
---

# Tasks: The native composer, over the document

**Input**: `specs/015-native-composer/`: [spec.md](spec.md), [plan.md](plan.md),
[research.md](research.md), [data-model.md](data-model.md),
[contracts/](contracts), [quickstart.md](quickstart.md)

**This file is the tracker.** These tasks get no issues (CLAUDE.md,
spec-driven work; #2215 is closed by the PR's `Closes` line at the end).
Tick a box in the commit that finishes it.

**Commits** on this branch end with `Refs: specs/015-native-composer` and the
task ids, for example `Refs: specs/015-native-composer T009 T010`. Never
write a closing keyword. Land with
`scripts/issue-land.sh --detach --title '<what the branch does>'`
after the phases marked **Land**.

**Tests are required** (constitution IV). Each task names what is seen
**Red** first, then what makes it **Green**. A case that is green on arrival
is a guard and says so.

## Conventions

- `[P]` means different files and no unfinished dependency. `[USn]` is the
  spec's user story. `⛔ 014` marks a task that needs spec 014's Part A
  merged.
- **The fast tier carries step 1.** `scripts/test-fast.sh` between edits,
  and `cargo nextest run -p postio-body --test body_suite <case>` to confirm.
- **No build runs while another agent builds.** The Linux box takes at most
  two agents and no back-to-back full-suite loops.
- **GTK tasks run on Linux, Swift tasks on a Mac** (`macos/CLAUDE.md`).
  Phases 2 to 5, 7 and 8 are Linux. T025, T026 and Phase 6 need a Mac.
- **Storyboards** (ADR 0044): behaviour-preserving ones are guards, green
  on arrival on the WebView and kept green on the native surface, and say so.
  A storyboard for behaviour that is new (spell underline, accessible text)
  is seen red on `main` first. `/ux-review` runs before landing steps 3 and 6.
- **Fixtures** use `example.com` and invented names only. CJK sample text is
  invented ("こんにちは", "你好").
- **Logs** never carry the text being edited (FR-025).

---

## Phase 1: Setup

- [ ] T001 [P] Write `storyboards/compose/native-list-keys.toml`, `native-undo-runs.toml` and `native-paste.toml` from US3's scenarios. **Guards:** green on the WebView on `main`; say so in their headers, and run with `scripts/storyboards.sh`
- [x] T002 [P] Write `scripts/checks/check-no-browser-engine.py` and its self-test in `scripts/tests/`. It fails if `Cargo.toml`, `Cargo.lock` or `flatpak/*.json` name `webkit6`, `webkit2gtk` or `WebKitGTK`, except the reader allowlist if any. **Red:** the self-test names `webkit6` in `crates/postio-widgets/Cargo.toml` on `main`. **Do not wire it into `scripts/check.sh` yet** (T040 does)
- [x] T003 [P] Comment on #2215 with the spec's path, the order of the phases and Q1 to Q3, so the maintainer can answer there

---

## Phase 2: Step 1 — `EditIntent` and `apply` in `postio-body` (US1, P1) 🎯

**Independent test**: `cargo nextest run -p postio-body --test body_suite apply projection`. No UI, no display. **Phase 2 is independently landable (spec D6, ADR 0039 sequencing).**

- [x] T004 [US1] **Red:** write cases in a new `crates/postio-body/src/position.rs` for `DocPosition` and `Selection` validity (no position in `Quoted`, `Rule` or an image's alt; offset within the block's text; collapsed and range) and their ordering. **Green:** implement them and export from `lib.rs`
- [x] T005 [US1] **Red:** write `crates/postio-body/tests/body_suite/projection.rs` for invariants 1 and 2 of `data-model.md` over lists, nested lists, quotes, headings and an image, and for `utf16_of` on CJK and an emoji. **Green:** implement `project`, `Projection::position_of`, `offset_of`, `utf16_of` in `crates/postio-body/src/projection.rs`
- [x] T006 [US1] **Red:** write text-intent cases in `crates/postio-body/tests/body_suite/apply.rs`, one per row of `contracts/edit-intent.md`: `Insert` over a range, `SplitBlock` (paragraph, heading, list item, empty last item leaves, empty nested outdents, empty quote child), `DeleteBackward`/`DeleteForward` (char, word, line, merge, un-list, un-quote, never across `Quoted`), `InsertBreak`. **Green:** implement `crates/postio-body/src/edit_ops/text.rs`, with `Edited`, `BlockRange` and `apply` in `edit_ops/mod.rs`
- [x] T007 [US1] **Red:** cases for `ToggleMark(Strong | Emphasis | Code)` over one block and across blocks, toggling twice returning the input, `SetLink`, `InsertImage`. This is spec 014 R6's inline-code arm on `main`'s `Document`. **Green:** `edit_ops/marks.rs`
- [x] T008 [US1] **Red:** cases for `ToggleBlock` (heading levels 1 to 3, bullet, numbered, quote, each toggling off), `Indent`/`Outdent` (a first item cannot indent), `InsertRule`. **Green:** `edit_ops/blocks.rs`, `edit_ops/lists.rs`. Spec 014 R6's indent and outdent are these
- [x] T009 [US1] **Red:** cases for `Paste(Document)` splicing (inline into the block, blocks between, over a range) and a hostile-markup case that goes `parse` then `Paste` and arrives narrowed (FR-007). **Green:** `edit_ops/paste.rs`
- [x] T010 [P] [US1] **Red:** in `crates/postio-ui/src/editor/markdown.rs`, cases for `conversion_before(block_text, caret) -> Option<Conversion>` over every row of today's sequence table (FR-068 of spec 002), longest-first. **Green:** implement it beside the table; the JavaScript generator is untouched until T020
- [x] T011 [US1] **Red:** cases for `Convert(Conversion)`: each sequence becomes its block or mark in one call, `Join::New`, and one `EditHistory::undo` restores the literal characters (FR-006). **Green:** `edit_ops/convert.rs`
- [x] T012 [US1] **Red:** extend `crates/postio-body/src/edit.rs`'s cases: `EditStep` carries `selection_before` and `selection_after`; `record` with `Join::Run` amends, `Join::New` starts a step; undo returns both document and selection. **Green:** change `EditHistory` accordingly (`amend` keeps its meaning)
- [x] T013 [US1] **Red:** write the property test in `body_suite/apply.rs`: for generated documents, selections and intents, the result satisfies `contracts/edit-intent.md` guarantees 1 to 3, and a 10 000-block document edited in the middle reports a `changed` range of at most two blocks with the outside equal (FR-003, a count, no clock). **Green:** fix what it finds
- [x] T014 [US1] **Red:** cases for the preedit hole: while a `Preedit` is set, every intent but `CommitPreedit`/`CancelPreedit` returns `unchanged`; a preedit update leaves the document equal; `project` shows the hole as a `Preedit` run that `position_of` never returns (FR-005, invariant 4). **Green:** `Projection` preedit and `apply`'s refusal

### The spike gate (FR-008)

- [x] T015 [US1] **Spike A, pure.** Write `body_suite/projection.rs`'s `cjk_preedit_into_a_list_item`: a list item "a", then updates "に", "にほ", "にほん", commit "日本語" and one `Insert`, one undo. At each step assert the document and the offset to position map. Red until T014 is done; then green. This is a guard that the model holds
- [x] T016 [US1] **Spike B, the widget.** Write `crates/postio-widgets/tests/widgets_suite/spike_preedit.rs`, a throwaway: a bare `GtkTextView` over `project(doc)`, driven by emitting `preedit-changed` and `commit` on its `GtkIMContext`, with `insert-text` blocked on the user path. Record in `research.md` R3, with the evidence (buffer text at each step, whether `insert-text` fired, whether a patch of another block cancels the preedit, Escape). Display tests live in `tests/` (CLAUDE.md)
- [x] T017 [US1] **Gate decision.** Write the verdict at the top of `research.md` R3: pass, pass with a change to D3 or D4 (say which), or fail. A fail stops here: amend ADR 0039 in a PR and wait for the maintainer. A pass deletes `spike_preedit.rs` (its claims are now in T034's real case) or keeps it as a guard, and the choice is recorded
- [x] T018 [US1] **Land step 1.** Run `quickstart.md` §1, then `scripts/issue-land.sh --detach --title 'feat(body,ui): the edit algebra over the document'`. The WebView composer is unchanged

---

## Phase 3: Step 2 — route the WebView composer through `apply` (US2, P1)

**Independent test**: the existing `widgets_suite` composer and editor cases, unchanged in what they assert, plus the new routing cases. **Mac host work is in this phase** because the Mac injects the same `editor.js` (spec Context).

- [ ] T019 [US2] **Red:** in `crates/postio-ui/src/editor/input.rs`, cases for `intent_of(beforeinput: InputEvent, selection) -> Routed`, where `Routed` is `Intent(EditIntent)`, `Native` (composition and plain insertion: leave to the DOM, research R5) or `Ignore`. **Green:** implement it. The payload's JSON shape is in `plan.md`
- [ ] T020 [US2] **Red:** a `widgets_suite` case, `editor_intents.rs`, with a counting hook on `parse`: Enter in a list item leaves the document equal to `apply`'s result and `parse` called zero times for it. **Green:** the `postioIntent` handler in `crates/postio-widgets/src/composer/editor.rs`, and in `crates/postio-ui/data/editor.js` a `beforeinput` forwarder and `applyPatch(blockRange, html, selection)`. Structural keys first: Enter, Backspace, Delete, Tab. **One commit**
- [ ] T021 [US2] **Red:** a case per command that bold, italic, headings, lists, quote, link and image are applied by `apply` and reflected in `FormatState`. **Green:** route them; delete their `execCommand` strings from `compose.rs`'s `mark_script` and friends. **One commit**
- [ ] T022 [US2] **Red:** the Markdown cases of `editor_markdown.rs` stay green, plus one: a conversion is one `Join::New` step. **Green:** route conversion through `Convert`; delete `markdown_table_js` and the `postioConverted` handler. **One commit** (FR-010)
- [ ] T023 [US2] **Red:** a paste case: hostile HTML arrives as `Paste(parse(..))` and not as DOM. **Green:** route `insertFromPaste`. **One commit**
- [ ] T024 [US2] **Red:** a case that undo and redo repaint through `applyPatch`: two undos after a run then a toggle; the run's coalescing timer uses `start_paused = true`. **Green:** `EditHistory` driven by `Edited.join`; the bridge no longer reports edits it already made
- [ ] T025 [P] [US2] **Red:** an `ffi_suite` case in `crates/postio-ffi/tests/` for a new `EditorDoc` object (holds a `Document`; `apply(intent) -> EditedFfi`; `project()`; `html()`): the table of T006 to T011 over the FFI, equal to the Rust result. **Green:** `crates/postio-ffi/src/editor_doc.rs` and the UniFFI records. Mac session
- [ ] T026 [P] [US2] **Red:** Swift cases in `macos/Tests/PostioKitTests/` for the host half: the coordinator answers a `postioIntent` message with `EditedFfi` and the block patch, using a fake `EditorDoc`. **Green:** `macos/Sources/PostioAppKit/ComposeEditor.swift`. The Mac stays on `WKWebView` until Phase 6. Mac session
- [ ] T027 [US2] Run T001's storyboards on both frontends and `/ux-review`: no behaviour change
- [ ] T028 [US2] **Land step 2.** Title `refactor(widgets,ui): composer edits go through apply`. Per-family commits (T020 to T023) mean a revert takes one family

---

## Phase 4: Step 3 — the `GtkTextView` surface, beside the WebView (US3, P2)

**Independent test**: the composer suites parametrised over both surfaces, and `focus_suite` for the composition. The native surface is selected by `POSTIO_NATIVE_COMPOSER=1`, read once at startup, default off. It is a developer switch and is deleted in Phase 5. **No task starts before T017 says pass.**

- [x] T029 [US3] **Red:** a guard that every public method `Composer` uses on the editor is on one `ComposerSurface` trait (`load`, `document`, `connect_changed`, `connect_format_state`, `undo`, `redo`, `format`, `create_link`, `insert_image`, `place_caret_start/end`, `flow_in`). **Green:** extract it from `editor.rs` and make the WebView implement it. No behaviour change. `crates/postio-widgets/src/composer/surface.rs`
- [x] T030 [US3] **Red:** `support_compose.rs` learns to build either surface, and a suite case `composer_native_basic.rs` loads a document and `document()` returns it. **Green:** `crates/postio-widgets/src/composer/native/mod.rs`, a `GtkTextView` holding a `Projection`, the switch
- [x] T031 [US3] **Red:** render cases (asserting tags and text, never geometry — `docs/gotchas.md`): heading tags, nested list margin and prefix text, quote bar, link tag, code tag. **Green:** `native/render.rs`, tokens from `postio_ui::tokens`
- [x] T032 [US3] **Red:** cases that the surface applies the result of `apply` as a `Patch` and counts it (FR-014): one keystroke is a bounded number of buffer operations; a signal-blocked count equals the patch's. **Green:** `native/patch.rs` and `PatchStats`
- [x] T033 [US3] **Red:** input cases, one per row of `contracts/surface.md`: keys, Enter, Backspace, Tab, registry commands, text commit. **Green:** `native/input.rs` with a capture-phase key controller, `insert-text` and `delete-range` blocked on the user path, `enable-undo` false with the reason in a comment (ADR 0004 Q5). A coverage case fails on a key the registry does not bind (FR-023)
- [x] T034 [US3] **Red:** `composer_native_ime.rs`: the CJK-into-a-list-item case of T016, on the real surface, through to one undo step; plus Escape cancel and a patch of another block mid-preedit. **Green:** `native/preedit.rs`. If T016 chose a different interception, this follows it
- [x] T035 [US3] **Red:** caret cases: a click on a bullet lands on the item's text; Home and End skip prefixes; the selection round-trips through `offset_of`/`position_of` across a patch. **Green:** `native/selection.rs`
- [x] T036 [US3] **Red:** clipboard cases: hostile HTML paste narrows; plain text paste splits on newlines into blocks as today; cut then paste round-trips; drop of a file attaches as today. **Green:** `native/clipboard.rs`
- [x] T037 [P] [US3] **Red:** port `editor_images.rs` and `composer_inline_image.rs`: an inline image is a `GtkTextChildAnchor` over a `GtkPicture` of the blob bytes, with no `postio-cid:` resolution, and deleting it is one atomic edit. **Green:** `native/image.rs`
- [ ] T038 [P] [US3] **Red:** the reply quote per Q2 (default A: read-only atomic range with a fold; the spike for B is T038's second case, timeboxed). Backspace on it removes it whole; typing into it is refused with no change. **Green:** `native/quoted.rs`
- [x] T039 [US3] **Red:** port `editor_format.rs`: toolbar reflection (`FormatState`) from the caret's `Selection`, one pure function in `postio_ui` shared with the Mac. **Green:** implement it and connect `connect_format_state`
- [x] T040 [US3] **Red:** undo and redo cases with the run timer on `start_paused = true`: a typing run is one step, a conversion is its own, undo restores the caret from `EditStep.selection_before`. **Green:** `native/history.rs`
- [x] T041 [US3] **Spike, spell-check (R8, Q3).** Attach a `SpellingTextBufferAdapter` in `native/spelling.rs`, behind the dependency decision. **Red:** a case that a misspelling is underlined, and survives a patch of an earlier block. If the adapter fights the patching, record it in R8 and ship without
- [x] T042 [US3] **Red:** accessibility: `focus_suite/a11y_sweep.rs` gains the composer, asserting text, caret and selection through `GtkAccessibleText`, accessible names on atomic units, and each formatting command's name. **Green:** set them in `native/a11y.rs`
- [x] T043 [P] [US3] **Red:** port `editor_appearance.rs`: the scheme and ground reach the surface through widget CSS, and a scheme change keeps the caret and the history (FR-075 of spec 002). **Green:** `native/css.rs`; move the tokens out of `postio_ui::editor::document` (FR-021's tokens half)
- [x] T044 [US3] **Red:** the performance case `composer_native_counts.rs`: 5 000 words, a keystroke clones one block and does a constant number of buffer operations and tag applications (FR-014, SC-003); the open is under 500 ms to a focused caret with no web process (a count of children). **Green:** fix what the counts show
- [ ] T045 [US3] **The suites.** Parametrise `composer_*.rs` and `editor_*.rs` over both surfaces, record each file's verdict in `research.md` R7 (port, moved down, or delete in step 4), and make the native column green. A "moved down" verdict means its case already exists in `body_suite` or `postio-ui`
- [ ] T046 [US3] Run T001's three storyboards on the native surface, and write `storyboards/compose/native-spellcheck.toml` and `native-screen-reader.toml` for the new behaviour, **red on `main`**, then green. Run the PNG loop at 1440 × 900 and 1024 × 768, light and dark, and `/ux-review`
- [ ] T047 [US3] **Land step 3.** Title `feat(widgets): a native composer surface beside the WebView`. Nothing user-visible: the switch is off

---

## Phase 5: Step 4 — delete the GTK WebView composer (US5, P2)

**Blocked on**: T045 (every native case green) and the maintainer's go-ahead after trying `POSTIO_NATIVE_COMPOSER=1`. **One commit** (FR-020).

- [ ] T048 [US5] **Red:** `scripts/checks/check-no-browser-engine.py` from T002 is wired into `scripts/check.sh` in this commit and fails until the rest of the task is done. **Green, in one commit:** delete `editor.rs`'s bridge, the three handlers, every `evaluate_javascript` call, `scheme.rs`, `web_process.rs`, `warm()` and `composer_warm.rs`, `editable_dialect.rs`, `editor_bridge.rs`, `editor_profile.rs`, `editor_teardown.rs`, and `webkit6` from `crates/postio-widgets/Cargo.toml` and the flatpak manifest; make the native surface the default and delete the switch. Each deleted test case carries a `scripts/test-removal-check.sh` verdict in the commit body
- [ ] T049 [US5] **Red:** the dialect contract as a `postio-body` unit test: markup `parse` accepts is what `to_html` writes, in `body_suite/document.rs` (was `editable_dialect.rs`'s job). **Green:** it should already hold; this is a guard, made before T048's deletion lands
- [ ] T050 [US5] Remove the composer's rows from the egress log's allowlist (#151) and add one case: no composer code path creates a socket or loads a URL
- [ ] T051 [US5] **Land step 4.** Title `refactor(widgets): one composer surface, no browser engine`. **Q1 = B:** `editor.js` and `EDITOR_SCRIPT` stay for the Mac until Phase 6

---

## Phase 6: The Mac's `NSTextView` (US4, P2) — Mac session

- [ ] T052 [US4] Write the conformance table as `crates/postio-body/tests/data/composer-conformance.toml`: scripts of inputs (text, Enter, Tab, marked text) and expected `Document` HTML after each. **Red:** a `body_suite` runner case `conformance.rs` fails on an empty table; **Green:** fill it from T006 to T011 and T034's inputs. The GTK `composer_native_conformance.rs` runs it too
- [ ] T053 [US4] **Red:** `ffi_suite` cases that `EditorDoc.project()` and `patch` agree with the Rust ones, with UTF-16 offsets. **Green:** extend `editor_doc.rs`
- [ ] T054 [US4] **Red:** Swift cases in `PostioKit` (no AppKit, #1264) for `ComposeSurfaceModel`: an `NSTextView` change (`shouldChangeText`) becomes an intent, the patch is applied with the delegate suppressed, the selection is set from `offsetOf`. **Green:** `macos/Sources/PostioKit/ComposeSurfaceModel.swift`
- [ ] T055 [US4] **Red:** a marked-text case: `setMarkedText` in a list item does not change the `EditorDoc`; `insertText` on commit is one `CommitPreedit`; cancel leaves it. AppKit puts marked text in `NSTextStorage`, so this is the Mac's own proof (R3). **Green:** `macos/Sources/PostioAppKit/NativeComposeTextView.swift`
- [ ] T056 [US4] **Red:** Swift conformance runner over `composer-conformance.toml` equals the Rust documents (FR-017). **Green:** fix what differs
- [ ] T057 [US4] Draw the blocks, `NSTextAttachment` images and the reply quote per Q2, with the system spell-check and VoiceOver defaults. Tests first for what the model decides; screens with `scripts/macos-shot.sh`
- [ ] T058 [US4] **Red:** `ComposeCommandsTests` for the surface answering every composer registry id it claims. **Green:** wire `ComposeCommands.handled` to the intents
- [ ] T059 [US4] **Delete, one commit:** `WKWebView` and `WebKit` from `ComposeEditor.swift`, `editorScript()` from `crates/postio-ffi/src/session.rs`, `crates/postio-ui/data/editor.js`, `EDITOR_SCRIPT`, `editor_script()`, `editor/document.rs`'s CSP and the old handler, `postioEdited` and `postioIntent` from the host. `check-no-browser-engine.py` extends to `macos/` (`import WebKit` in the composer is a failure). The reader's own `WebKit` use, if any, is out of this check (it is Blitz on Linux and `MessageBodyView` on the Mac; the check names its allowlist)
- [ ] T060 [US4] **Land step 6.** Title `feat(macos): the Mac composes on NSTextView`

---

## Phase 7: Unblock spec 014 (US6, P2)

**Blocked on**: Phase 4, and for tasks marked ⛔ 014, spec 014's Part A merged. The mapping to 014's ids is `contracts/spec-014-unblock.md`. Each commit also edits `specs/014-compose-formatting/tasks.md` to tick the task and drop its marker (FR-024).

- [ ] T061 [US6] ⛔ 014 Registry: **Red:** 014's T054 cases for the Held rows and `Context::TablePicker`. **Green:** register them. (014 T054)
- [ ] T062 [US6] ⛔ 014 Picker frame keys `picker_choose_0` to `9`, with the guard that the four existing pickers ignore them. (014 T055)
- [ ] T063 [US6] ⛔ 014 **Red:** native-composer cases for 014 US1's scenarios 1 to 7 in `composer_formatting.rs` and `storyboards/compose/formatting-inline.toml` red. **Green:** inline typing rows of `contracts/sequences.md`. (014 T057)
- [ ] T064 [US6] ⛔ 014 Draw strikethrough and inline code; retire their FR-029 refusals; storyboard green. (014 T058)
- [ ] T065 [US6] ⛔ 014 **Red:** cases for 014 US2's scenarios 1 to 8 and `formatting-blocks.toml`. (014 T059)
- [ ] T066 [US6] ⛔ 014 Block rows, Tab and Shift+Tab, `line_break`, and the drawing of each block (heading scale, code ground, nested quote rules, the task box as a checkbox control). (014 T060)
- [ ] T067 [US6] ⛔ 014 The code language picker's words and picker. (014 T061)
- [ ] T068 [US6] ⛔ 014 `crates/postio-ui/src/format_picker.rs` model (`contracts/formatting-picker.md`), cases first. (014 T062)
- [ ] T069 [US6] ⛔ 014 `focus_suite/formatting_picker.rs` for US4's scenarios 1 to 6, `formatting-picker.toml` red. (014 T063)
- [ ] T070 [US6] ⛔ 014 The picker at the caret on the frame, the toolbar's new buttons and their overflow; then the Mac's SwiftUI picker (Mac session); `/ux-review`. (014 T064)
- [ ] T071 [US6] ⛔ 014 **Red:** US5's scenarios 1 to 5 and `formatting-table.toml`. (014 T065)
- [ ] T072 [US6] ⛔ 014 Delimiter-row sequence, table picker, cell navigation; draw the grid; PNG loop and `/ux-review`. The table `apply` arms (014 T056) land first, here: each pinned linear in its block, cases first. (014 T056, T066)
- [ ] T073 [US6] ⛔ 014 `paste_markdown` on the native surface. (014 T067)
- [ ] T074 [US6] ⛔ 014 The remote image chip drawn inline, pressing `attach_remote_image`. (014 T068)
- [ ] T075 [US6] ⛔ 014 Assert no FR-029 refusal remains. Land it. (014 T069)

---

## Phase 8: Polish

- [ ] T076 [P] Amend ADR 0039: status "Built", the spike's outcome, the signature deviation (D1), the Mac having shipped compose before it, and Q1 to Q3's answers. No parallel ADR for this spec (CLAUDE.md)
- [ ] T077 [P] Update `docs/ARCHITECTURE.md` (`postio-body`'s `apply` and `project`; `postio-widgets`'s native surface; no `webkit6`) and remove the WebKit composer gotchas from `docs/gotchas.md`; add the native surface's traps found along the way
- [ ] T078 [P] Update `docs/PRODUCT.md` §10 for what is no longer held, and the reply quote's look if Q2 chose A
- [ ] T079 Run `quickstart.md` end to end, then `scripts/target-sweep.py`. Close #2215 with the final PR's `Closes` line

---

## Dependencies and order

- **Phase 1** first; T002 before T048.
- **Phase 2**: T004 → T005 → T006 → T007 → T008 → T009 → T011 → T012 → T013
  → T014 → T015 → T016 → T017 → T018. T010 can run beside T004 to T009. T011
  needs T010.
- **Phase 3** needs Phase 2 landed (T018). T019 → T020 → T021 → T022 → T023
  → T024. T025 → T026 can run beside T020 and must land before the editor.js
  change reaches the Mac (same PR). T027 and T028 last.
- **Phase 4** needs T017 = pass and Phase 3. T029 → T030 → T031 → T032 →
  T033 → T034 → T035 → T036. T037, T038, T043 are parallel after T030.
  T039, T040 need T033. T041 needs T033. T042 after T031. T044 after T036.
  T045 after all. T046, T047 last.
- **Phase 5** needs T045, T047 and the maintainer's go-ahead.
- **Phase 6** needs Phase 5, a Mac, and T025. T052 → T053 → T054 → T055 →
  T056 → T057 → T058 → T059 → T060.
- **Phase 7** needs Phase 4 and, for ⛔ 014, spec 014's Part A. It runs beside
  Phases 5 and 6, except T070 (the Mac picker), which needs Phase 6.
- **Phase 8** last. T076 can start once T017 is written.

### Parallel example (Phase 2)

```text
T004 position.rs        T010 postio-ui conversion_before
T005 projection.rs      (after T004: T006, T007, T008 are sequential in apply.rs)
```

## Implementation strategy

1. Phase 1, then Phase 2 to T018: the independently useful half.
2. **Stop and review** after T017 (the gate) and after Phase 3 (what the
   maintainer sees is unchanged).
3. Phase 4 behind the switch; the maintainer tries it; Phase 5 on their word.
4. The Mac and spec 014's held tasks in parallel on their dependencies.
