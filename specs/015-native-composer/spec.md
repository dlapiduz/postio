# Feature Specification: The native composer, over the document

**Feature Branch**: `feature/native-composer`

**Created**: 2026-11-22

**Status**: Approved by the maintainer on 2026-11-28, with the
recommendations to Q1-Q3 (Clarifications)

**Input**: Issue #2215, "Build ADR 0039's native composer over the
document". The maintainer decided on 2026-11-22 that spec 014's rich editing
surface waits for this composer.

## Context

The composer is a WebKit `contenteditable` on both frontends. ADR 0039
(accepted 2026-09-18) replaces it with a native text view over
`postio_body::Document`, and moves the editing algebra into `postio-body` as
pure functions. Nothing tracked the work until #2215. It is spec-driven, so
it has no issues of its own (CLAUDE.md).

What exists on `main`, and what this spec changes:

| Piece | Today | After |
|---|---|---|
| Editing surface, GTK | `webkit6::WebView` over `contenteditable` (`postio-widgets/src/composer/editor.rs`, 1023 lines) | `GtkTextView` over a projection of `Document` |
| Editing surface, Mac | `WKWebView` over the same script (`macos/Sources/PostioAppKit/ComposeEditor.swift`) | `NSTextView` over the same projection |
| The bridge | `postio-ui/data/editor.js` (310 lines), shared by both frontends, three script-message handlers (`postioEdited`, `postioConverted`, `postioFormat`) | none |
| What an edit is | whatever the DOM did, then `postio_body::parse` of the result | `apply(Document, Selection, EditIntent)`, a pure function |
| Undo | `EditHistory` of `Document` snapshots (ADR 0004 Q5), amended by a 700 ms timer in the bridge | the same `EditHistory`, fed by `apply`'s result |
| Markdown typing | `postio_ui::editor::markdown`, generated into JavaScript (`markdown_table_js`) | the same table, matched in Rust and applied by `apply` |
| Dialect contract | `editable_dialect.rs`, a WebKit integration test pinning WebKit's markup | a `postio-body` unit test |
| Spell-check | never enabled | `libspelling`, if its spike passes (Q3) |
| Accessibility | `AccessibleRole::TextBox` on a `WebView` | `GtkAccessibleText` (native), `NSTextView` (native) |
| Browser engine in the Linux app | `webkit6` for the composer only (the reader is Blitz, ADR 0042) | none |

Two facts the issue did not know, both found reading the tree:

- **The Mac already composes**, on a `WKWebView` that injects the same
  `editor.js` as GTK (`ComposeEditor.swift`). ADR 0039 says compose is
  deferred on the Mac; it has shipped since. So the shared script cannot be
  deleted until both frontends have left it (Q1).
- **Spec 014 widens `Document`** (headings 1 to 6, `Code`, `Table`, tasks,
  strikethrough, remote images). Its PR #2199 is not on `main`. Step 1 is
  built on `main`'s `Document`; the arms that need 014's types come after
  014's Part A has merged (D6).

### What this spec inherits

- **ADR 0039** in full: the decision, its sequencing and its two
  falsifiers. **ADR 0004**: the document is the record, the DOM or buffer a
  working copy (Q3), and the two undo stacks (Q5). **ADR 0003**'s
  restricted subset and hardening requirements 2, 5 and 6 (requirements 1,
  3 and 4 are met vacuously once no browser is left).
- **ADR 0049**: each layer asserts its own question. **ADR 0044**: a
  keyboard change ships a storyboard, red on the base. **ADR 0045**:
  behaviour lives in the engine, not in Swift or GTK.
- **Spec 014**'s held tasks T054 to T069, which are this spec's acceptance
  for the rich editing surface (US6).
- **Spec 002** (archived) FR-067 to FR-077, the editor's requirements,
  unchanged.
- **The constitution**, in particular I (local-first), II (the keyboard is a
  system), IV (test-first), V (performance as counts), VI (privacy) and VII
  (boundaries).

## Decisions this spec takes

**D1. `apply` carries a selection.** ADR 0039 writes
`apply(&Document, EditIntent) -> Document`. An edit also needs to know where
the caret is and where it goes next, so the signature is
`apply(&Document, &Selection, EditIntent) -> Edited`, where `Edited` holds
the new document, the new selection and the range of top-level blocks that
changed. The ADR's shape is the special case with a selection of nothing. The
position model is a block path plus a character offset in that block's text
(spec 014 R6's "caret by block path"). See `data-model.md`.

**D2. The buffer is a projection, and the projection is pure.** The mapping
from a buffer offset to a `DocPosition` and back is a pure function in
`postio-body` (`project`), not code in the GTK widget. It is what ADR 0039
calls "the one genuinely new mechanism", so it is tested at the `--lib` tier
and shared by the Mac through the FFI. The surface paints; it does not decide
what an offset means.

**D3. Preedit is a hole, not an edit.** Text the input method has not
committed is not in `Document`. The projection carries an optional
`Preedit` region that maps to no document position. A commit is one
`Insert` intent. This is ADR 0039's "preedit-shaped hole". GTK may never
put preedit in the buffer at all (research R3, to be confirmed by the
spike); AppKit does (marked text lives in `NSTextStorage`), so the model
must allow for both.

**D4. The surface intercepts, then patches.** A native text view mutates its
own buffer by default. The surface stops that: it turns the key, commit,
paste or drop into an `EditIntent`, runs `apply`, and patches **only the
changed blocks** of the buffer. It never re-renders the whole buffer on a
keystroke, which would lose the caret, the preedit and the spell-check
underline, and break the 16 ms budget on a long draft. Widget-native undo is
turned off, as ADR 0004 Q5 requires.

**D5. Step 2 routes everything but the text the engine owns.** On the
WebView surface, `beforeinput` for structural and formatting input types is
cancelled and sent through `apply`. Plain text insertion and IME composition
stay with the DOM and are absorbed as today, because a composition event
cannot be cancelled. Text that arrives this way is still parsed to a
`Document`, as now. This is a stated limit of step 2 (research R5); step 3
removes it.

**D6. Spec 014's arms come in two batches.** Arms over types on `main`
(toggle inline code, indent and outdent, today's block toggles) are step 1.
Arms over types spec 014 adds (tasks, list start, code language, tables) are
built when 014's Part A has merged, in Phase 7, as ADR 0039's step 1 was
already proven by then.

**D7. One deletion commit per surface, one surface per platform.** ADR 0039
says "one surface, not two behind a flag". That holds per platform: GTK's
WebView composer is deleted in one commit when the GTK surface passes its
suites, and the Mac's in one commit when the Mac's does. The shared
`editor.js` goes with the second of the two (Q1).

## Clarifications

Taken without asking, on the evidence:

| # | Question | Answer |
|---|---|---|
| C1 | Does ADR 0003's rejection of `GtkTextView` still bite? | No. Its objection was nested lists faked with indentation tags. Nesting lives in `Document`; the buffer only draws it (ADR 0039 Q1). |
| C2 | Is caret motion rebindable? | No, as today. Arrows, Home, End and word jumps are the toolkit's. Every *command* (formatting, send, links) is a registry id, bound in `Context::Composer`. |
| C3 | Does the reply quote (`Block::Quoted`) become editable? | No. It is opaque to the caret: one atomic unit, deleted whole, as spec 014 FR-025 already draws it. How it is drawn is Q2. |
| C4 | Do autosave, resume and send change? | No. They read `Document`, which was always the record. |
| C5 | A new crate? | No. `postio-body` gains modules, `postio-ui` gains the key-to-intent rules, `postio-widgets` gains the surface. |

The maintainer's answers, 2026-11-28:

| # | Question | Answer |
|---|---|---|
| Q1 | When does the Mac move to `NSTextView`? | B: after step 3, with the deletion split per platform (D7). The Linux app drops `webkit6` first; `editor.js` stays until the Mac moves. |
| Q2 | How is the reply quote drawn? | A for step 3: a read-only plain-text projection in an atomic, tinted range with a fold. B, an embedded reader view, is tried as a spike inside Phase 4 for the maintainer to see. |
| Q3 | Is `libspelling` in step 3? | A, gated on its spike (FR-019) and on the Flatpak runtime shipping it; otherwise B, its own task after step 4. |

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Every edit is a rule, proven without a window (Priority: P1)

A contributor changes how Enter behaves in a list item. They write a case in
`postio-body`, see it red, make it green, and run it in a second, with no
display and no browser.

**Why this priority**: this is ADR 0039's step 1, where its risk is, and it
is worth landing even if everything after it stops.

**Independent Test**: `cargo nextest run -p postio-body --test body_suite apply`
and the `apply` unit cases. No UI.

**Acceptance Scenarios**:

1. **Given** a document and a selection, **When** any `EditIntent` is
   applied, **Then** the result is a valid `Document` (it equals
   `parse(to_html(result))`) and a valid selection, for every intent over a
   generated corpus.
2. **Given** a list item holding "ab" with the caret after "a", **When**
   Enter is applied, **Then** the item splits in two with the caret at the
   start of the second; and Enter in an empty last item leaves the list.
3. **Given** a document of 10 000 blocks, **When** an edit is applied in the
   middle one, **Then** `Edited` reports exactly the blocks changed (one
   block, or the two a split makes), and no other block is cloned (a count,
   not a clock).
4. **Given** a typed CJK string arriving as a sequence of preedit updates and
   one commit inside a list item, **When** the projection and `apply` are
   driven as a surface would, **Then** the document is unchanged until the
   commit, the commit is one intent and one undo step, and every buffer
   offset maps to the same document position before and after.
5. **Given** the typed sequences of `postio_ui::editor::markdown`, **When**
   a trigger is typed, **Then** the conversion is one `apply` call and one
   undo step that restores the literal characters (FR-070 of spec 002).

---

### User Story 2 - The composer on its existing surface edits through `apply` (Priority: P1)

A person writes mail as they do today. They see no difference. Underneath,
Enter, Backspace, list and quote toggles, bold, italic, headings, links,
inline images, Markdown typing and paste now run through `apply`.

**Why this priority**: it is ADR 0039's step 2: the old behaviour is one
revert away, and it kills the JavaScript copy of the rules before the new
surface is built.

**Independent Test**: the existing `widgets_suite` composer cases
(`composer_*`, `editor_*`), unchanged in what they assert, on both
frontends.

**Acceptance Scenarios**:

1. **Given** the composer open on the WebView, **When** the person presses
   Enter in a list item, **Then** the DOM shows what `apply` returned, the
   caret is where `Edited` put it, and `document()` equals the applied
   document without a `parse` of the DOM.
2. **Given** a typing run followed by a pause and a bold toggle, **When**
   Ctrl+Z is pressed twice, **Then** the toggle goes, then the run, as today.
3. **Given** a Japanese phrase composed through the IME on the WebView,
   **When** composition ends, **Then** the text is in the document and
   undoes in one step (D5: the DOM keeps composition).
4. **Given** `editor.js` after this step, **Then** it holds no Markdown
   table, no `execCommand` formatting path and no structure rules: it
   forwards `beforeinput` and renders patches.

---

### User Story 3 - Compose on a native text view on Linux (Priority: P2)

A person writes in a `GtkTextView` that shows the document: paragraphs,
headings, nested lists with their bullets and numbers, quotes, links, inline
images. It has a spell-check underline, a screen-reader text interface, and
an input method that works in every script.

**Why this priority**: it is the point of ADR 0039, and the gate on the
Mac's.

**Independent Test**: the composer suites, run against the new surface
(FR-018), plus `crates/postio-gtk/tests/focus_suite` for the composition and
`storyboards/compose/native-*.toml`.

**Acceptance Scenarios**:

1. **Given** a nested list, **When** the person presses Tab and Shift+Tab,
   **Then** the item nests and un-nests, the bullet prefix and margin follow,
   and the buffer's bullet text is never in the saved document.
2. **Given** a CJK input method preediting inside a list item, **When** the
   person commits, **Then** the text is in that item, the caret is after it,
   and one Ctrl+Z removes it.
3. **Given** a misspelled word, **When** the surface is idle, **Then** it is
   underlined and the suggestion menu offers corrections (if Q3 passes), and
   the draft sent holds the word as typed.
4. **Given** hostile HTML pasted, **Then** what lands is what
   `postio_body::parse` narrows it to, and no script, remote image or
   `file:` reference is reachable (ADR 0003 requirement 2).
5. **Given** a screen reader, **Then** the composer exposes the text, the
   caret and the selection through `GtkAccessibleText`, and each formatting
   command has an accessible name.
6. **Given** a draft of 5 000 words, **When** a key is typed, **Then** the
   typing path is under 16 ms and touches one block (FR-014).

---

### User Story 4 - The Mac composes on `NSTextView` (Priority: P2)

The Mac composer uses an `NSTextView` over the same projection and the same
`apply`, called through the FFI. It has the system's spell-check, marked
text and VoiceOver support.

**Why this priority**: the issue's "Done when" asks for it, and it unblocks
deleting the shared script.

**Independent Test**: Swift tests (`scripts/macos-test.sh`) over the FFI
cases, and the Mac storyboards.

**Acceptance Scenarios**:

1. **Given** marked text in a list item, **When** the person commits it,
   **Then** the result equals the GTK case's document for the same input
   (one conformance table, FR-017).
2. **Given** the same keystroke script as US3's, **When** it runs on the Mac,
   **Then** the saved `Document` is equal.
3. **Given** the Mac frontend after this story, **Then** `ComposeEditor`
   imports no `WebKit` and `PostioFFI` exposes no editor script.

---

### User Story 5 - No browser engine is left in the compose path (Priority: P2)

A person composes, and no web process starts. A reviewer greps the Linux
app's dependency graph and finds no browser engine.

**Why this priority**: ADR 0039 Q4. It is also the privacy claim becoming
structural: "no network from compose" no longer needs an egress test to
hold.

**Independent Test**: `scripts/checks/check-no-browser-engine.py` (reads
`Cargo.toml`, `Cargo.lock` and the flatpak manifest, never runs cargo), and
the deletion commit's diff.

**Acceptance Scenarios**:

1. **Given** the deletion commit, **Then** `editor.js`, `editor.rs`'s bridge,
   the three script-message handlers, `scheme.rs`, `web_process.rs`,
   `editable_dialect.rs` and the `postio-cid:` scheme handler for the
   composer are gone, and the dialect contract is a `postio-body` unit test.
2. **Given** the Linux app's `Cargo.lock` after both steps, **Then** `webkit6`
   is absent (the reader is Blitz).
3. **Given** the composer opens cold, **Then** no web process is spawned and
   `warm()` no longer exists.

---

### User Story 6 - Rich editing for spec 014 (Priority: P2)

The held half of spec 014 lands on the native surface: live formatting, the
formatted display, the toolbar, the pickers and Paste as Markdown.

**Why this priority**: the maintainer decided on 2026-11-22 that these wait
for this composer, and the Markdown source view is the interim way to author
the new constructs until then.

**Independent Test**: spec 014's own acceptance for each held task, run
against the native surface. The mapping from its tasks to this spec's is in
`tasks.md`, Phase 7, and in `contracts/spec-014-unblock.md`.

**Acceptance Scenarios**: each of spec 014 T054 to T069 is done when its own
statement in `specs/014-compose-formatting/tasks.md` is, on the native
surface, and its `⛔ ADR 0039` marker is removed from that file in the
commit that finishes it (FR-024).

---

### Edge Cases

- **A command during preedit**: commits the preedit first, then runs
  (as WebKit's `execCommand` does today; spec 014 R6).
- **Selection across blocks**: bold over a selection spanning a list item
  and a paragraph applies to each block's covered range; backspace over it
  deletes the range and merges the ends.
- **Selection touching a `Quoted` block**: Delete removes it whole or not at
  all; typing inside it is refused with no change.
- **Caret motion in rendered prefixes**: bullets, numbers and margins have no
  document position; the caret skips them, and Home on a list item lands on
  the item's text, not before its bullet.
- **Undo during preedit**: refused until the commit.
- **A very long single block** (a pasted log): the patch is linear in that
  block, which is the stated bound (FR-014).
- **Reduced motion**: the surface animates nothing, so there is nothing to
  honour; the caret blink follows the system setting.
- **Right-to-left text**: offsets are characters, not bytes or UTF-16 units,
  in `Selection`; each frontend converts at its edge (`data-model.md`).

## Requirements *(mandatory)*

### Functional Requirements

**The algebra (step 1)**

- **FR-001**: `postio-body` MUST export `EditIntent`, `Selection`,
  `DocPosition`, `Edited` and `apply`. `apply` MUST be total: it never
  panics, and an intent that does not apply returns the input unchanged with
  `Edited::unchanged`.
- **FR-002**: The result of `apply` MUST be a valid `Document` and a valid
  selection (one inside a block's text, never inside a `Quoted` block).
- **FR-003**: `apply` MUST report the range of top-level blocks it changed,
  and MUST NOT clone or rewrite the others. A case with a 10 000-block
  document asserts the count.
- **FR-004**: `postio-body` MUST export `project` and `Projection`: the
  buffer text, the runs (what each is: text, a list prefix, a margin, an
  atomic unit), and the two-way offset map. The map MUST be total over
  committed text and MUST send a rendered prefix's offsets to the nearest
  document position.
- **FR-005**: The projection MUST carry an optional `Preedit` region. While
  one is set, `apply` MUST refuse every intent except `CommitPreedit` and
  `CancelPreedit`, and the document MUST be unchanged by preedit updates.
- **FR-006**: Every Markdown sequence in `postio_ui::editor::markdown` MUST
  be an `EditIntent::Convert` over the same table, and MUST undo to the
  literal characters in one step.
- **FR-007**: Paste MUST pass through `postio_body::parse` and arrive as
  `EditIntent::Paste(Document)`. Hostile markup narrowing into the subset is
  a property of the type, not of the widget.
- **FR-008**: The spike gate (Phase 2) MUST be run and its result written to
  `research.md` before any task in Phase 4 begins. A failed gate stops this
  spec and amends ADR 0039; it is not worked around.

**The existing surface (step 2)**

- **FR-009**: The WebView composer MUST apply Enter, Backspace, Delete,
  Tab, Shift+Tab, block toggles, mark toggles, links, images, Markdown
  conversions and paste through `apply`, and MUST NOT `parse` the DOM for
  them (D5 states what remains).
- **FR-010**: `markdown_table_js` and every JavaScript implementation of a
  structure rule MUST be deleted in this step.
- **FR-011**: Each intent family MUST land as its own commit so a revert
  takes one family back.

**The native surface (steps 3 and 5)**

- **FR-012**: The GTK surface MUST be a `GtkTextView` in `postio-widgets`
  that holds a projection and never the record. Widget-native undo MUST be
  off, with a comment saying why.
- **FR-013**: The surface MUST turn text commits, key presses, paste, drop,
  cut and IME events into intents, and MUST apply the result as a block-range
  patch (D4).
- **FR-014**: Typing MUST take under 16 ms in a 5 000-word draft, gated as
  counts: blocks cloned, buffer insertions and tag applications per
  keystroke are each bounded by constants, asserted in a test with no clock.
- **FR-015**: The surface MUST draw every block and inline kind the document
  has now: paragraph, headings, nested lists, quotes, links, inline images
  (`GtkTextChildAnchor` over the blob bytes, no scheme handler), code, and
  the reply quote (Q2).
- **FR-016**: Accessibility: the GTK surface MUST implement
  `GtkAccessibleText` (native on `GtkTextView`), and an accessibility sweep
  case MUST cover it. The Mac uses `NSTextView`'s own.
- **FR-017**: The Mac `NSTextView` MUST call the same `apply` and `project`
  through `postio-ffi`. One conformance table of input scripts and expected
  documents MUST run on both platforms.
- **FR-018**: "The composer suites" are every file `widgets_suite/composer_*.rs`
  and `widgets_suite/editor_*.rs`, plus `editable_dialect.rs`. Each is
  ported (same assertion, new surface), moved down to a rule crate, or
  deleted with the surface it tested; `research.md` R7 lists the disposition
  of each, and a deletion carries a `scripts/test-removal-check.sh` verdict.
- **FR-019**: Spell-check: attach `libspelling`'s adapter if its spike
  passes (Q3). If it fails, ship without and amend ADR 0039 Q1's claim.

**One surface (step 4)**

- **FR-020**: The GTK deletion MUST be one commit and MUST remove `editor.rs`'s
  bridge, the handlers, every `evaluate_javascript` call, `webkit6` from the
  composer's graph, and `editable_dialect.rs`. `check-no-browser-engine.py`
  MUST be added in that commit and fail on any `webkit6` dependency.
- **FR-021**: `postio-ui/data/editor.js`, `EDITOR_SCRIPT` and
  `editor/document.rs`'s CSP MUST be deleted with the Mac's WebView
  composer. Their design tokens move to the widget CSS.

**Privacy, keyboard, performance**

- **FR-022**: The composer MUST have no network path: no scheme handler, no
  fetch, no script. The egress log (#151) loses its composer rows because
  the claim is structural.
- **FR-023**: Every command stays a registry id in `Context::Composer`, with
  no key hard-coded in the surface. A coverage case MUST fail if the surface
  handles a key the registry does not bind. The keyboard change ships its
  storyboards (ADR 0044): `storyboards/compose/native-list-keys.toml`,
  `native-undo-runs.toml`, `native-paste.toml`: guards, green on `main`, since they preserve behaviour; storyboards for new behaviour (spell underline, screen reader) are red on `main` first.
- **FR-024**: Spec 014's held tasks T054 to T069 MUST be unblocked: each done
  on the native surface, and its marker removed from spec 014's `tasks.md`
  in the same commit (`contracts/spec-014-unblock.md`).
- **FR-025**: Logs carry ids, counts and outcomes. They never carry the text
  being edited, the preedit, or a spell-check word.

### Key Entities

See `data-model.md`: `DocPosition`, `Selection`, `EditIntent`, `Edited`,
`Projection`, `Run`, `Preedit`, `Conversion`, `Mark`, `BlockKind`.

## Success Criteria *(mandatory)*

- **SC-001**: Every `EditIntent` arm has a `postio-body` case that was seen
  red, and the generated-document property (FR-002) holds for all of them.
- **SC-002**: The IME spike (Phase 2) passes, with a CJK string typed into a
  list item, on GTK; the Mac marked-text case passes in Phase 6.
- **SC-003**: Typing in a 5 000-word draft is block-local: one block cloned,
  constant buffer operations, asserted as counts. Composer open on a warm
  process is under 500 ms to a focused caret with no web process spawned.
- **SC-004**: After step 4, `Cargo.lock` of the Linux app contains no
  `webkit6`, and the reader and composer together have no browser engine.
- **SC-005**: Every composer suite is ported, moved down or deleted with a
  verdict (FR-018), and CI's `Frontends` check is green.
- **SC-006**: Spec 014's T054 to T069 are ticked, and no `⛔ ADR 0039`
  marker remains in its `tasks.md`.
- **SC-007**: A screen reader reads the composer's text and caret on GTK
  and on the Mac (a case each).

## Assumptions

- `GtkTextView` keeps preedit out of the buffer. This is what GTK's source
  does as far as research R3 shows, but the spike must prove it before step
  3 is locked; ADR 0039 assumed the opposite and D3 is built for both.
- `libspelling` is packaged on the targets (ADR 0039 says Fedora and the dev
  machine). The flatpak runtime and CI's apt cache are unchecked (Q3).
- Spec 014's Part A merges before Phase 7 starts. If it has not, Phase 7
  waits and Phases 2 to 6 are unaffected (D6).
- The Mac phases need a Mac. This branch is written on Linux, so Phase 6
  runs from a Mac session (`macos/CLAUDE.md`).
