# ADR 0039 — The composer is a native surface over the document

- **Status:** Accepted — decided 2026-09-18. **Supersedes [ADR 0003](0003-rich-text-compose.md) Q2**
  (the `contenteditable` WebView), which it answers again on evidence ADR 0003
  said it did not have. Everything else in ADR 0003 — the restricted subset,
  the hardening requirements, the privacy posture — stands unchanged and is
  what makes this possible.
- **Date:** 2026-09-18
- **Issue:** [#1543](https://github.com/dlapiduz/postio/issues/1543) (`needs-architecture`),
  which asked whether the *reader* could be Blitz and required an explicit
  answer about the composer before any recommendation counted
- **Related:** [ADR 0004](0004-composer-document-model.md) (the document, and
  Q5's two undo stacks — the load-bearing prior decision),
  [ADR 0019](0019-macos-frontend.md) (the frontend whose compose slice this
  unblocks), [ADR 0032](0032-the-conversation-is-one-document.md),
  `specs/002-compose-editor/` (the feature's own requirements, unchanged by
  this)
- **Decision:** the composer stops being a `WebView`. Its editing surface
  becomes a **native toolkit text view over `postio_body::Document`** —
  `GtkTextView` in `postio-gtk`, `NSTextView` in the Swift frontend — and the
  **editing algebra moves into `postio-body` as pure functions on `Document`**.
  No `contenteditable`, no bundled script, no script-message bridge, and no
  browser engine in the compose path at all.

---

## Context

#1543 spiked a Blitz reading surface and reached the question it said would
decide the matter: *Blitz has no `contenteditable` and no JavaScript, so what
happens to the composer?* The spike named three options — Blitz reader plus a
native composer, WebKit kept for the composer alone, or something defensible —
and recorded that "ship both engines is worse than either alone".

That framing contains one mistake, and correcting it is most of this decision.
**`GtkTextView` is not a second engine.** "Two engines" is a real cost when it
means two HTML/CSS implementations — two sanitiser targets, two font stacks,
two sets of layout behaviour, two security postures to keep true. A toolkit
text widget is none of those. It is the same class of thing as `GtkLabel`, and
Postio is already full of them. So the choice is not between one engine and
two; it is between one engine plus a widget, and two engines.

The second thing that changed is that ADR 0003's premises have expired. It was
written on 2026-08-24, when nothing existed, and it chose `contenteditable`
because that view "arrives with selection handling, IME, native undo,
spell-check, drag-and-drop and paste already working". Measured against the
tree at `13cf825d`, three of those six are not being collected:

| ADR 0003 said it gives us | What is actually there |
|---|---|
| Native undo | **Rejected and turned off.** ADR 0004 Q5 ruled that a DOM undo step and a `Document` step disagree about what one step is, put the history in `postio_body::EditHistory`, and made widget-native undo the documented silent failure mode |
| Spell-check | **Never enabled.** `set_spell_checking_enabled` is called nowhere; the word "spell" does not appear in `postio-gtk`'s composer at all. The feature ADR 0003 counted has never been switched on |
| The input dialect | **Re-implemented in Rust anyway.** The markdown sequences live in `postio_ui::editor::markdown` and are *generated into* `editor.js` by `markdown_table_js`, because "a hand-written copy in JavaScript is a copy that drifts" |
| Selection, IME, drag-and-drop, paste | Genuinely provided — and also provided by `GtkTextView`, from the same `GtkIMContext` stack WebKitGTK sits on under GTK |

So the surface is being paid for in full and used for less than half of what
it was chosen for. What it charges in exchange is specific and countable:
`editor.js` (260 lines), three registered script-message handlers, a formatting
path that runs `execCommand` strings through `evaluate_javascript`, and
`tests/gtk_suite/gtk_editable_dialect.rs` (224 lines) whose whole job is to pin
*WebKit's* dialect so `postio_body::parse` can absorb it. That test exists
because the markup is a foreign engine's opinion rather than Postio's.

And ADR 0004 Q3 already decided the thing that makes the WebView removable:
**the DOM is a working copy and never the record.** Every edit crosses the
bridge, is `parse`d, and is thrown away; `Document` is what persists. The
engine is not holding the document. It is holding a caret and some pixels.

---

## Q1 — What replaces `contenteditable`?

**A `GtkTextView` whose buffer is a projection of `Document`, and an edit
algebra in `postio-body` that operates on `Document` directly.**

```
   keystroke / gesture  ──> EditIntent          (postio-ui, toolkit-free)
                              │
                              ▼
                            apply(&Document, EditIntent) -> Document
                              │                             (postio-body,
                              │                              pure, closed set)
                              ├──> EditHistory              (already exists)
                              └──> render into GtkTextBuffer  (postio-gtk)
```

The direction of flow is the one ADR 0004 Q3 already draws, with the WebView
lifted out of the middle of it. What is new is `apply`, and it is worth being
precise about its size: `Document` is **six block kinds and seven inline
kinds**, closed by ADR 0004 Q2 and defended by a test. "Toggle strong over this
selection", "split this paragraph", "make this block a list item", "merge with
the previous block on backspace at offset zero" are total functions over that
set. They are ordinary Rust, they run at the `--lib` tier in milliseconds, and
every one of them can be proven red before it is written.

This is the opposite of what ADR 0003 feared. It rejected `GtkTextView` because
"nested lists and nested quotes have to be faked with indentation tags" — true,
and irrelevant, because **the nesting does not live in the buffer**. It lives
in `Document`, where it is structurally exact. The buffer renders a
`List { ordered, items }` as a left margin and a bullet prefix the way a label
renders a string: a display decision that cannot lose information, because the
information is not stored there.

What the buffer does own is a caret offset, and mapping that offset to a
position in `Document` is the one genuinely new mechanism. It is named again
under "What would falsify this".

**Inline images** are `GtkTextChildAnchor` holding a `GtkPicture` over the blob
bytes — strictly simpler than today's path, which resolves a `postio-cid:` URI
through a custom scheme handler registered on a `WebContext`.

**Paste** normalises through `postio_body::parse`, which is what it already
does and must keep doing whatever the surface is: hostile markup narrowing into
the subset is a property of the type, not of the widget.

**Spell-check** becomes real rather than notional. `libspelling` (0.4.10,
packaged by Fedora and installed on the development machine) attaches a
`SpellingTextBufferAdapter` to a `GtkTextBuffer` and brings the underline and
the suggestion menu with it. This is a feature Postio *gains* by leaving the
WebView, not one it gives up.

**Accessibility** likewise. `GtkTextView` implements `GtkAccessibleText`
natively (GTK 4.14+; 4.22 here). The current composer calls
`set_accessible_role(AccessibleRole::TextBox)` on a `WebView` — a role claim
with no interface behind it, leaving a screen reader to reach WebKit's own
out-of-process tree. ADR 0032's screen-reader gate is still open in the index;
this narrows it rather than widening it.

## Q2 — Why not a Blitz composer?

**Because it does not exist, and building it is the months ADR 0003 was right
to refuse.**

Checked against `blitz-dom` 0.3.0-beta.2 rather than assumed: the string
`contenteditable` does not occur anywhere in its source. What is there is
`Document::hit()`, a `TextSelection` of node-and-offset endpoints, IME,
keyboard and pointer event handling, AccessKit, and `parley::PlainEditor` —
and that last one is bound to `<input>` and `<textarea>` nodes, which is
**plain text in a form field**, not rich text over a document tree.

So "the composer is Blitz too" means writing a rich-text editing engine: a
caret model over a styled tree, selection across block boundaries, IME preedit
composed over rich runs, and an accessibility tree. That is strictly worse than
what ADR 0003 turned down, because there would be no toolkit underneath to fall
back on for the IME and accessibility halves.

Rejected. The reader and the composer do not have to share an engine, and the
moment one stops trying to make them, both get easier.

## Q3 — Why not keep WebKit for the composer alone?

The spike called this "worse than either", and it is worth writing down *why*,
because it is the option that looks like the safe one.

- **It keeps the entire cost.** Three processes and ~408 MB of helpers stay
  linked and running for the surface used least often. Nothing about the
  reader's engine changes that; the WebView is the WebView.
- **It keeps the flicker, on exactly the swap that has it.** The black
  composite comes from a second GL surface being swapped in. The composer takes
  over the reading pane (ADR 0034), so reader↔composer *is* that swap. Moving
  the reader to Blitz and leaving the composer on WebKit does not remove the
  second surface — it renames which one is second.
- **It keeps the script posture and the dialect contract**, which are the two
  things in the compose path that need an argument written about them.
- **It keeps macOS compose blocked.** ADR 0019 shipped read-only, with compose
  deferred. A `contenteditable` composer does not port: it is a WebKit dialect,
  a bundled script and three GTK script-message handlers. An algebra over
  `Document` in `postio-body` ports by definition, because the Swift frontend
  already links it.

## Q4 — Does this depend on the Blitz reader?

**No, and that is the strongest argument for taking it first.**

A native composer is right in both worlds:

- If Blitz ships for the reader, WebKitGTK leaves the tree entirely, in two
  independent moves rather than one large one.
- If Blitz does not ship, the reader keeps WebKit and the composer is *still*
  better native — no dialect contract, no bundled script, no bridge, real
  spell-check, real accessibility, a portable editor, and one fewer GL surface.

The reader question turns on evidence that is still being gathered (#1543).
This one turns on evidence already in the tree. They are separable, so
separating them is what stops the composer from being the thing that blocks the
reader decision for another six months.

---

## Alternatives

**Keep the `contenteditable` WebView (status quo).** Defensible only on inertia
now that three of its six stated benefits are provably unclaimed. Its real
remaining advantage is that it works today, which is a reason to sequence
carefully — see below — not a reason to decide differently.

**`GtkTextBuffer` as the record, serialised to HTML.** The flat tag model made
the ADR 0003 case against `GtkTextView`, and it would be a real objection if the
buffer held the document. It does not, and it must not: ADR 0004 Q3 is what
keeps a second frontend possible, and a buffer-shaped record would undo it.

**A third-party GTK rich-text framework.** ADR 0003 assessed `text-engine` and
its own README still says "generally not suitable for use in applications".
Unchanged, and now unnecessary — the framework would be solving the document
problem, which `postio-body` already solved.

**Markdown-authored, HTML generated.** Rejected by the maintainer on
2026-08-24 (ADR 0003) and **not reopened here.** This ADR changes the surface,
not the product decision: Postio still gets a true WYSIWYG editor, and
`postio_ui::editor::markdown` stays what it is — an input convenience over a
rich document, not a source format.

---

## Consequences

- **`postio-body`** gains the edit algebra: an `EditIntent` set and
  `apply(&Document, EditIntent) -> Document`, beside the `EditHistory` that
  already consumes the results. No new dependencies; it stays a pure leaf, and
  `check-crate-boundaries.py` keeps it that way.
- **`postio-gtk`** loses `data/editor.js`, the three script-message handlers,
  `editing_view`/`editing_settings`, every `evaluate_javascript` formatting
  call, and `tests/gtk_suite/gtk_editable_dialect.rs` — the dialect stops being
  a foreign engine's and becomes Postio's, so the contract is a `postio-body`
  unit test instead of a WebKit integration one.
- **`postio-ui`** keeps `editor/markdown.rs` unchanged and stops generating
  JavaScript from it. `editor/document.rs` is the editing *shell* — a
  stylesheet, a ground colour and a CSP for a document that will no longer
  exist; its tokens move to the widget's CSS and the CSP goes, because there is
  nothing left to have a content policy about.
- **ADR 0003's hardening requirements 1, 3 and 4** are satisfied vacuously: no
  composer WebView, no network stack in the compose path, no script at all.
  Requirements 2, 5 and 6 are unchanged and still carry the weight — quoted
  content is sanitised before insertion, what comes out is parsed into
  `Document`, and replies and forwards carry nothing outward. Those were always
  properties of `postio-body`, which is why they survive the surface changing.
- **The egress log (#151)** has less to cover, and the claim it was proving
  ("no network from compose") becomes structural.
- **ADR 0019's deferred compose slice** becomes buildable: the Swift frontend
  implements a view over the same algebra rather than re-deriving an editor.
- **`specs/002-compose-editor/`** is unaffected as a specification. Every FR
  it states is about what the editor must *do*; this changes what it is made
  of. FR-072…FR-077 (the editing shell's appearance) are re-satisfied by widget
  styling rather than by a document stylesheet, and FR-075 — "a scheme change
  must not take the caret and the undo history with it" — gets easier, since
  there is no document to reload.

## Sequencing

The composer works today, and a rewrite that takes it away for a fortnight is
worse than the thing it fixes. So:

1. `EditIntent` and `apply` in `postio-body`, proven against `Document` with no
   UI at all. Independently useful, and it is where the risk actually is.
2. Route the **existing** WebView composer's edits through `apply` instead of
   through `parse`-the-DOM, keeping the surface. Both paths are `Document`-in,
   `Document`-out, so this is a swap with the old behaviour one revert away.
3. Build the `GtkTextView` surface beside it, behind nothing user-visible,
   until `gtk_composer.rs` passes against it.
4. Delete the WebView composer, `editor.js`, the handlers and the dialect test
   in one commit. **One surface, not two behind a flag** — the same rule the
   spike set for the reader.

Steps 1 and 2 are worth doing even if step 3 is never reached, which is the
property to preserve if this has to be paused.

## What would falsify this

**The caret.** `apply` is the easy half; the hard half is mapping a
`GtkTextBuffer` offset to a position in `Document` and back, across a render
that inserts bullets and margins the document does not contain. If that mapping
turns out not to be stable under IME preedit — where the buffer holds
uncommitted text that is not yet an edit — then the surface is holding state
`Document` cannot represent, and the design needs a preedit-shaped hole in it
rather than a workaround. **Spike step 1 to the point of typing a CJK string
into a list item before step 3 is considered locked.**

Second, and smaller: if `libspelling`'s adapter turns out to fight a buffer
that is re-rendered from a `Document` rather than edited in place, spell-check
returns to being a thing Postio does not have — which is where it is today, so
this would cost nothing already counted, but the claim in Q1 should not be left
standing if it is false.

A clean failure on either is a good outcome, and it lands on the sequencing
above before anything has been deleted.
