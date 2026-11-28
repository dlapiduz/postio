# Research: The native composer

Each entry is a decision, why, and what was rejected. Entries marked
**Spike** are answered by a task in `tasks.md`, and the answer is written
back here, so the gate (FR-008) has a place to land.

## R1 — The position model

**Decision.** `DocPosition` is a block path (`Vec<u32>`: the index in
`Document.blocks`, then, for each list or quote entered, the item or child
index) and a character offset into that block's flattened inline text.
Offsets count Unicode scalar values.

**Why.** Spec 014 R6 already chose it ("the block's index path and the
offset in its text"). It survives a patch of another block. A flat offset
over the whole document would shift on every edit, and an offset in a
`GtkTextBuffer` is what we are mapping *from*, not the model.

**Rejected.** A tree path to the exact `Inline` node: it breaks as soon as
bold splits a text run. UTF-16 or byte offsets: GTK counts characters, AppKit
counts UTF-16 units, so the model uses the neutral unit and each frontend
converts at its edge (`Projection::utf16_of`).

## R2 — What `apply` is made of

**Decision.** One closed `EditIntent` enum, one `apply`, one function per
arm in `postio-body::edit_ops/` (a module per family: `text`, `marks`,
`blocks`, `lists`, `paste`, `convert`). The history stays in `edit.rs`.

**Why.** ADR 0039 Q1: "total functions over a closed set". A module per
family keeps each under a screen, and lets `body_suite/apply.rs` mirror it.

**Rejected.** A trait object per intent: nothing needs open extension, and
exhaustiveness is the property we want from the compiler when spec 014
widens `Document`.

## R3 — Where GTK keeps preedit **Spike: passed (2026-11-29)**

**Verdict: pass, with no change to D3 or D4.** GTK keeps preedit in the
layout, never in the buffer; its commit arrives as the buffer's
`insert-text`, which the surface stops and turns into one intent; a patch
of another block leaves a composition open; Escape commits nothing. The
gate (FR-008) is open for Phase 4.

**Evidence** (`widgets_suite/spike_preedit.rs`, GTK 4.22.5 on the headless
compositor). The view's own input method is reached through its key
controller (`EventControllerKey::im_context`), set to GTK's simple context,
and fed real key events with `IMContext::filter_key`, so the path is the
one a person's input method takes. Ctrl+Shift+U then `65e5` (U+65E5, 日)
into a list item of `project(doc)`:

| Step | View's `preedit-changed` | Buffer | `insert-text` seen |
|---|---|---|---|
| Ctrl+Shift+U | `u` | `before\n• a`, unchanged | none |
| `6`, `5`, `e`, `5` | `u6`, `u65`, `u65E`, `u65E5` | unchanged | none |
| Space | `` (cleared) | unchanged: emission stopped | `日`, then ` ` |
| Ctrl+Shift+U, `65`, a patch of the first line, `e5`, Space | `u65` survives the patch | first line patched only | `日`, then ` ` |
| Ctrl+Shift+U, `65e5`, Escape | `` | unchanged | none |

So the interception is `insert-text` (and `delete-range`) stopped on the
user path, not the input method's `commit`, which the view keeps private.
The space that ends a hex entry is committed after the character: the
surface sees two inserts, one a composition's commit and one typing.
The spike is kept as a guard until T034 asserts the same on the real
surface, and is deleted in that commit.

The expectation it tested:

**Expectation.** `GtkTextView` handles the `preedit-changed` signal of its
`GtkIMContext` by setting a preedit string on the text layout (Pango
attributes drawn at the cursor). The `GtkTextBuffer` itself is not modified
until `commit`. If so, the buffer equals the projection throughout a
composition, and the mapping is trivially stable. This **contradicts** ADR
0039's wording ("the buffer holds uncommitted text") and must be checked on
the real widget, not read from memory.

**What the spike proves** (Phase 2, T015 and T016):

1. A CJK preedit sequence, then a commit, into a list item: the buffer text
   is unchanged during preedit; after commit it holds the text; `Projection`
   maps the caret to the right `DocPosition` at each step.
2. The `commit` can be intercepted before it mutates the buffer
   (`insert-text` stop-emission, or a `commit` handler on the IM context) and
   turned into one `Insert` intent. If it cannot, D4 needs a different
   interception and the gate fails.
3. A patch of **another block** during a preedit does not cancel it.
4. Cancel (Escape) leaves buffer and document unchanged.

**If it fails.** The surface holds state `Document` cannot represent. D3
already gives the projection a `Preedit` hole; if even that cannot hold,
stop and amend ADR 0039 (FR-008). Nothing is deleted before then.

**AppKit differs.** Marked text lives in `NSTextStorage`. The Mac maps it as
the projection's `Preedit` region (`NSTextInputClient.markedRange`), and
Phase 6 has its own case (T055).

## R4 — Intercepting a native text view

**Decision.** Three doors, one rule: nothing mutates the buffer except the
surface's patch.

- **Keys** go to a `GtkEventControllerKey` in the capture phase. Registry
  commands run first; structural keys (Enter, Backspace, Delete, Tab,
  Shift+Tab) become intents; caret motion is left to the widget.
- **Text** arrives by `commit` and becomes `Insert`. The buffer's
  `insert-text` and `delete-range` are blocked on the user path and
  unblocked only inside the patch.
- **Clipboard and drop** are `paste-clipboard`, `cut-clipboard` and a drop
  target, each turned into `Paste` or `Delete` intents.

**Why.** It is the only arrangement where `Document` stays the record
(ADR 0004 Q3). The alternative, letting the buffer edit and re-reading it,
makes the buffer the record and returns us to `parse`-the-DOM with a
different DOM.

**Rejected.** `GtkTextBuffer` undo (ADR 0004 Q5), and a full re-render per
keystroke (D4).

## R5 — What step 2 cannot route **Decision**

`beforeinput` for `insertParagraph`, `deleteContent*`, `formatBold`,
`insertFromPaste` and the list and quote toggles is cancellable. `insertText`
over a composition and every `insertCompositionText` is not, and cancelling
it breaks the input method. So on the WebView surface, step 2 routes
everything except composing text, which stays DOM-native and is absorbed by
`parse` at `compositionend`, as now. Step 3 removes the exception. The old
`postioEdited` handler stays for that one path until step 4 deletes it.

**Why accept it.** The alternative is to change nothing on the WebView and
find out about the algebra only at step 3. Step 2 is worth landing because
it moves the *rules* (Enter, Backspace, lists, Markdown) into Rust and
deletes the JS copies; it is not required to move characters.

## R6 — Undo coalescing

**Decision.** `Edited` carries `Join::Run` (extend the open step, for typing
and deleting inside one block) or `Join::New`. The surface owns the run's
timer (today's 700 ms `COALESCE`), because a clock is a surface's
business; the rule that a mark toggle, a split, a conversion or a paste is
always `New` is `apply`'s. Cases for the timer use `start_paused = true`
(CLAUDE.md: no real waits).

## R7 — The composer suites, and what each becomes

Disposition of today's files in `crates/postio-widgets/tests/widgets_suite/`
(FR-018). "Port" means the same assertion on the new surface.

| File | Asks | Becomes |
|---|---|---|
| `composer_document.rs` | the body round-trips through the neutral document | Port, then also a `postio-body` case |
| `composer_markdown.rs`, `editor_markdown.rs` | typed sequences convert and undo to literals | Move down to `body_suite/apply.rs` plus `postio-ui` matcher cases; one port for composition |
| `editor_format.rs` | format toggles and the caret's `FormatState` | Move down (toggle) and port (state is read from `Selection`) |
| `editor_images.rs`, `composer_inline_image.rs` | inline image paste and drop | Port (child anchor) |
| `editor_bridge.rs`, `editor_profile.rs`, `editor_teardown.rs` | the script bridge, the hardened WebKit profile, teardown | Delete in step 4, each with a verdict; the claims become the structural check |
| `editable_dialect.rs` | WebKit's markup narrows into the subset | Delete in step 4; the contract moves to `body_suite` as a unit test |
| `editor_appearance.rs` | scheme and ground colour reach the editor | Port to widget CSS |
| `composer_autosave.rs`, `composer_resume.rs`, `composer_confirms.rs`, `composer_attachments.rs`, `composer_recipients.rs`, `composer_host.rs`, `composer_focus.rs`, `composer_toolbar.rs`, `composer_size.rs`, `composer_warm.rs` and the rest | the shell around the editor | Unchanged where they do not touch the surface; `composer_warm.rs` is deleted with `warm()`; `composer_focus.rs` and `composer_toolbar.rs` are re-pointed |

The full list is `ls widgets_suite/composer_*.rs widgets_suite/editor_*.rs`;
T045 records a verdict for each.

## R8 — Spell-check **Spike**

ADR 0039's second falsifier: does `SpellingTextBufferAdapter` cope with a
buffer patched by the surface instead of edited in place? Patch operations
are ordinary `insert`/`delete` on the buffer, so the adapter should see them.
The spike (T041) types, patches an earlier block, and checks the underline
survives. A failure costs nothing counted: spell-check is absent today.

## R9 — Performance as counts

**Decision.** `apply` returns the changed block range; a test double
`CloneCounter` is not needed, because `Edited::changed` is the claim and a
property test builds the output document and asserts every block outside the
range is pointer-equal in structure (equal and not rebuilt, checked by
`Rc`-free structural identity: the old and new `Vec` prefixes and suffixes
compare equal and the changed range is the reported one). The surface's
patch counts buffer operations through a small `PatchStats` returned from
`patch`, asserted constant per keystroke in `widgets_suite`.

**Why not time it.** CLAUDE.md gates budgets as counts; a clock is flaky on
shared machines.

## R10 — Reply quote **Open (Q2)**

`Block::Quoted` holds sanitised rich HTML. Options are in the spec's Q2. The
projection treats it as one `Run::Atomic` either way, so step 1 is unaffected.

## R11 — Accessibility

`GtkTextView` implements `GtkAccessibleText` natively since GTK 4.14, and
`postio-widgets` already builds against `v4_20`. The atomic runs (list
prefix, quote, image anchors) need accessible names, set on the tags and
child widgets. Verify with `focus_suite/a11y_sweep.rs`, adding the composer.

## R12 — What does not change

`postio-body`'s `parse`, `to_html`, `render`, `harden`, `quote_of`, the
signature and reply machinery. The draft store (`body_html`,
`body_markdown`), autosave and resume. The registry's `Composer` context
(spec 014 widens it, not this spec).

## R13 — What a valid edited document is (found building T013)

**Finding.** `contracts/edit-intent.md` guarantee 1, `parse(to_html(d)) == d`,
cannot hold for every result: a trailing space while typing, a double space
and an empty paragraph after Enter are editing state, and `parse` collapses
and drops them by design (its whitespace rules keep the round trip of
*written* mail stable). So the property test asserts guarantee 1 where the
result holds no such state (`settled`), and asserts structure everywhere:
no empty list, item or quote, and inlines in the form `parse` writes (no two
neighbours that are one run, no empty mark). Over 4 000 generated cases it
found two real bugs, each fixed in the same commit.
