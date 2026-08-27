# ADR 0003 — Rich-text (HTML) compose

- **Status:** Accepted — **GO** (2026-08-24)
- **Date:** 2026-08-24
- **Issue:** [#3 Rich-text (HTML) compose](https://github.com/dlapiduz/postio/issues/3), under [#17 Epic: Compose](https://github.com/dlapiduz/postio/issues/17)
- **Related:** bead `postio-3o8f`; issues [#12](https://github.com/dlapiduz/postio/issues/12) (rich signatures), [#13](https://github.com/dlapiduz/postio/issues/13) (`$EDITOR`)
- **Decision:** a true WYSIWYG editor, built on a **`contenteditable` WebView
  under a hardened profile**. A restricted HTML subset is the canonical draft
  form; `text/plain` is generated from it; `postio-model` owns the subset and
  its conversions.

> **Product decision, taken by the maintainer 2026-08-24:** Postio gets a real
> rich-text editor. A Markdown-authored alternative was evaluated (it is what
> `aerc`, `muttdown` and Fractal do, and it makes the plaintext part *authored*
> rather than derived) and was **rejected** — issue #3's framing stands, that
> every peer client treats WYSIWYG as baseline. This ADR records the rejected
> option in "Alternatives" because its costs are the costs this design now
> takes on, and they should be visible rather than forgotten.

---

## What is already built

More than it looks, and it changes the shape of the work. Measured at `37c10b8`:

| Piece | State |
|---|---|
| `MessageBody { text, html }` | Built (`model/src/message.rs:18`) |
| `multipart/alternative` on send | **Built** — `outgoing.rs:163-166`, with a passing round-trip test |
| Inline images as `cid:` parts | **Built** — `an_inline_image_round_trips_with_its_content_id_and_is_referenced_from_the_html` passes |
| Draft persistence for HTML | **Built** — `drafts.body_html` exists (`0001_initial_schema.sql:331`) |
| Reading HTML safely | Built — `reader/sanitize.rs` (ammonia), hardened `WebView`, `postio-cid:` scheme |
| Composing HTML | **Absent** — `composer.rs:279` is a `gtk::TextView` |
| Generating `text/plain` | **Absent** — no HTML→text exists anywhere in the tree |
| Quoting an HTML message | **Absent by decision** — `reply.rs:157`: "HTML-only content is not quoted" |

Criterion 2 is half-built at the MIME layer with no generator. Criterion 3 is
currently a documented non-feature.

---

## Q1 — What is the canonical form of a draft?

Not "whatever the editing surface holds" — `ARCHITECTURE.md` §13 rules that
out, and it is what makes a second frontend possible.

**Decision: a restricted HTML subset, with a parsed `Document` in
`postio-model` as its typed form.**

```
    WebView (contenteditable)          ← working copy, never the record
        │  serialize + sanitize
        ▼
    canonical HTML  (restricted subset)   ← stored in drafts.body_html
        │  parse
        ▼
    Document (postio-model)               ← typed form: quoting, plaintext,
        │                                   signatures, tests operate here
        ├── to_html()  ──> text/html
        └── to_text()  ──> text/plain
```

The subset — deliberately small, *what an email body needs*:

```
Block   = Paragraph | Heading(1..3) | List{ordered} | Quote | Rule | Pre
Inline  = Text | Strong | Emphasis | Code | Link{href}
        | Image{content_id, alt} | Break
```

**Restricting the subset is what makes everything else tractable**, and it is
the single most important constraint in this ADR:

- `to_text()` over a closed set of ten node types is a total, testable function.
  Generic HTML→text is the thing that makes most mail's plaintext part
  unreadable; crates like [`html2text`](https://lib.rs/crates/html2text) exist
  because the general problem is hard. **Do not reach for one** — convert from
  `Document`, not from arbitrary markup.
- Outgoing HTML is *generated* from `Document`, never passed through. A
  sender's `<script>`, tracking pixel or `style` attribute has no path into a
  message Postio sends, because there is no pass-through path at all.
- A second frontend implements its own editor against the same subset.

**No tables, no colours, no font control, no arbitrary CSS.** Each is a fresh
sanitisation, round-trip and `to_text()` problem, and none is in issue #3's
scope (bold, italic, lists, links, inline images). `reader/sanitize.rs` already
strips `style` on the way *in* so Postio's CSS always wins; emitting it on the
way out would be inconsistent with that.

## Q2 — What is the editor?

Three candidates were assessed. Two are real.

| Option | Verdict |
|---|---|
| **`WebKitWebView` + `contenteditable`** | **Chosen.** What [Geary](https://wiki.gnome.org/Apps/Geary/FAQ) does — it uses WebKitGTK to edit message bodies as HTML documents, even for plaintext mail. Proven in a GNOME mail client |
| `GtkTextView` + `GtkTextTag` | Viable but a poor trade — see below |
| [`text-engine`](https://github.com/mjakeman/text-engine) | The only GTK4 rich-text framework that exists. Its own README: "under heavy development and generally not suitable for use in applications", one known adopter (Extension Manager). Not a foundation for a mail composer |
| `GtkSourceView` | A code editor. Wrong tool |

### Why `contenteditable` over `GtkTextView`

A `contenteditable` view arrives with selection handling, IME, native undo,
spell-check, drag-and-drop and paste already working, and it produces HTML
directly. Postio **already links WebKitGTK** for the reader, so this adds no
dependency.

`GtkTextView` would mean building all of that. `GtkTextBuffer`'s tag model is
flat, so nested lists and nested quotes have to be faked with indentation
tags; inline images need child anchors; link editing, paste normalisation and
undo granularity are all hand-rolled. It is many months of work to reach an
editor that would still feel worse than every peer. For a solo maintainer that
is the wrong place to spend the budget, and a rich-text editor that feels bad
is worse than not shipping one.

### The cost, stated plainly

A `contenteditable` editor needs JavaScript: `contenteditable` alone gives
typing, but toolbar commands, selection queries and reading the markup back go
through the Selection/`execCommand` APIs, and WebKitGTK will not run
host-injected script into a JS-disabled view either. So compose needs a
**second WebView with JS enabled**, and that view will hold quoted content
derived from attacker-controlled mail.

`ARCHITECTURE.md` §11 was refined for this (2026-08-24, at the maintainer's
direction) and now states the principle rather than the mechanism: **script
that arrived in a message never executes, in either direction**, while Postio's
own bundled editor script is not message content and is therefore permitted.

That is a sharper rule than "the WebView has JS off", not a weaker one — it
closes a gap the old wording missed entirely. The old rule said nothing about
**outbound** script, so a reply or forward could have re-emitted a sender's
markup to a third party while remaining technically compliant. The requirements
below are what make the refined rule true rather than aspirational.

### Hardening requirements — non-negotiable

These are the conditions under which the above is acceptable. They are
acceptance criteria, not advice.

1. **The composer WebView is a separate view with its own settings.** The
   reader's WebView keeps JS off. Nothing here relaxes the reader.
2. **Quoted content is sanitised before it is ever inserted** — through the
   existing `reader/sanitize.rs` path, reduced to the Q1 subset. Hostile markup
   never reaches the DOM, so enabling script in that DOM is not enabling script
   *beside* attacker markup.
3. **No network from the composer view.** No remote loads, ever. Enforced the
   way the reader enforces it — a custom scheme handler that resolves `cid:`
   from the blob store and fails everything else, plus a CSP with no remote
   origins.
4. **The only script is Postio's own**, shipped in the GResource bundle. No
   remote script, no `eval` of message content, no `innerHTML` of untrusted
   strings from the host side.
5. **What comes out of the DOM is sanitised again on the way out**, and parsed
   into `Document`. The DOM is a working copy; it is never trusted as the
   record. This is the property that keeps the canonical subset canonical even
   if WebKit normalises markup unexpectedly.
6. **Replies and forwards carry no script outward**, and this gets its own
   tests. A forward is the sharpest case: `reply.rs:193`'s `forward_body`
   currently takes `source.body.text`, so it is safe by accident today —
   the moment forwarding carries HTML, an unsanitised path would re-emit a
   sender's markup to a third party. Postio must never make a recipient run
   something its own user was protected from. Test it against a hostile corpus
   fixture, in both the reply and forward paths.
7. **The egress log (#151) covers the composer**, so "no network from
   compose" is a proven claim rather than an asserted one.

## Q3 — How does replying to an HTML message quote it?

The criterion that forces a crate-boundary decision, and the sharpest thing here.

Quoting means parsing untrusted HTML into `Document`, which **must** be
sanitised: a reply re-emits quoted markup into the world, so the composer
defends *the recipient* where the reader defends *the user*. The sanitiser
exists — `postio-gtk/src/reader/sanitize.rs`, on `ammonia`.

**But `reply()` lives in `postio-model`, which cannot depend on `postio-gtk`.**
`ammonia` appears in exactly one manifest today, `postio-gtk`'s. So something
must move:

| Option | Cost |
|---|---|
| Put `ammonia` + HTML→`Document` in `postio-model` | The purest crate gains an HTML parser. `ammonia` is pure computation, but it contradicts "pure domain types" in spirit |
| Move `reply`/quoting into a new crate | Splits recipient logic from quoting logic, which are one operation today |
| **Invert the dependency** | `reply()` stops computing the quote and accepts one |

**Decision: invert the dependency.** `reply(source, account, quote:
Option<Document>)` — or a small `QuoteSource` trait — keeps `postio-model`
pure. The HTML→`Document` parser and the sanitiser live in a new crate above it.

That crate is the `postio-ui` extraction proposed in
`architecture-review-2026-08.md` §2: `sanitize.rs`, `quote.rs` and
`allowlist.rs` are on its list of toolkit-free code trapped in `postio-gtk`,
~760 lines with zero or near-zero GTK references. **Issue #3 is the forcing
function for that extraction** — the first feature that genuinely cannot be
built without it. Doing them together is cheaper than working around it.

## Q4 — Inline images

`Image { content_id }` in the subset. Pasted or dropped bytes go to the blob
store and become an attachment with that `Content-ID`. `outgoing.rs` already
emits this correctly with a passing test, and `reader/scheme.rs`'s
`postio-cid:` handler and its `BlobSource` trait are the display path to reuse
in the composer.

**Privacy constraint:** a pasted image is local bytes becoming a local blob.
Nothing in compose may fetch a remote URL to build a body — that is
`ARCHITECTURE.md` §11 broken from the other direction, and it is the most
likely accidental violation in this whole feature (pasting an `<img src=https…>`
from a browser must inline-or-drop, never fetch-on-send).

---

## Alternatives considered

**Markdown-authored, HTML generated** — the user types Markdown, `text/plain`
is what they typed, `text/html` is rendered from it. This is what
[`aerc`](https://man.sr.ht/~rjarry/aerc/configurations/htmlmail.md) does with
`[multipart-converters]`, what [`muttdown`](https://github.com/Roguelazer/muttdown)
does with its `!m` sigil, and what Fractal does for formatted Matrix messages.

**Rejected by the maintainer** in favour of true WYSIWYG. Recorded because its
advantages are precisely the costs this design accepts:

- The plaintext part would be *authored* rather than derived, so it reads
  correctly by construction. Under this ADR it is generated, and its quality
  now depends entirely on `to_text()` and on the subset staying small.
- Issue #13 (`$EDITOR`) would have been lossless and trivial. It is now a
  **lossy round-trip** — rich structure cannot survive a text editor — and #13
  must decide deliberately between warning the user, offering `$EDITOR` only
  for plaintext drafts, or accepting the loss.
- No editor engine, no JS, no second WebView, no privacy-posture change.

## Consequences and interactions

- **`postio-model`** gains `Document`, the subset, `to_html`, `to_text`;
  `reply()` changes signature. No new dependencies.
- **New crate** (`postio-ui`) owns sanitisation and HTML→`Document`, taking
  `ammonia` from `postio-gtk`.
- **`postio-gtk`** gains the composer WebView, its bundled editor script, and
  formatting commands; `composer.rs:554` stops synthesising `MessageBody` from
  a `GtkTextBuffer`.
- **`postio-storage`** needs no migration — `drafts.body_html` already exists.
- **`ARCHITECTURE.md` §11 already covers this** — refined 2026-08-24 to scope
  the no-script rule to message-derived content in both directions. The
  hardening requirements in Q2 are what implement it.
- **Issue #12 (rich signatures)** gains a real HTML variant but inherits
  `signature.rs`'s constraint: RFC 3676 says `-- ` means *everything after this
  is signature*, so it must be last, and `apply()`'s idempotent replace has to
  keep working over a `Document`.
- **Compose stays keyboard-first.** Every formatting action is a registry
  command with a binding (`ARCHITECTURE.md` §2), and the toolbar is a second
  way to reach them — never the only way. A mouse-only bold button would be the
  first command in the app the keyboard cannot reach.

## Suggested sequencing

1. `Document` + subset + `to_html` + `to_text` in `postio-model`, round-trip
   tested. No UI. Correct plaintext generation lands here.
2. Extract `postio-ui`; move sanitise/quote/allowlist; add HTML→`Document`.
3. Invert `reply()`'s quote dependency — HTML replies now quote (criterion 3).
4. Composer WebView behind the hardening requirements, plaintext-only at first,
   with the request log proving no network.
5. Formatting commands and toolbar; inline images last.

Steps 1–3 deliver two of three acceptance criteria with **no editor work**, and
are independently useful. Start there.

## What would falsify this

Nothing here has been prototyped.

The highest-risk assumption is that a `contenteditable` view can be constrained
to emit the Q1 subset reliably. WebKit will normalise markup in ways the editor
script does not fully control, and if serialise→sanitise→parse turns out to be
lossy against ordinary editing gestures, the canonical-subset property fails
and with it the portability argument. **Spike step 4 before steps 1–3 are
considered locked** — the subset's shape should be informed by what the editor
actually produces.

Second risk: if the hardening requirements in Q2 cannot all be met
simultaneously, the honest response is to reopen the editor choice, not to
quietly drop a requirement. Requirement 6 in particular is not negotiable —
outbound script in a forward is a harm to someone who never chose to use
Postio at all.
