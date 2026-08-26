# ADR 0004 — The composer's document, and where it lives

- **Status:** Accepted — **GO** (2026-08-24)
- **Date:** 2026-08-24
- **Issue:** [#30 Model the composer document before building the rich text editor](https://github.com/dlapiduz/postio/issues/30) (P0)
- **Builds on:** [ADR 0003](0003-rich-text-compose.md), which chose a restricted
  HTML subset with a typed `Document` as its parsed form. 0003 answered *what
  the document is*. This ADR answers **where it lives, who may parse HTML, and
  what "sanitised on the way out" actually means.**
- **Related:** `docs/ARCHITECTURE.md` §11 and §13,
  `docs/architecture-review-2026-08.md` §6
- **Decision:** a new domain-rank crate, **`postio-body`**, owns the document,
  the HTML subset, the parser, the serialiser, quoting and sanitisation — in
  both directions. `postio-model` does **not** gain an HTML parser.
  `postio-gtk` keeps the `WebView` and loses `reader/sanitize.rs` and
  `reader/quote.rs`.

---

## What is already built

Measured at `0e0ec08`.

| Piece | State | Where |
|---|---|---|
| `MessageBody { text, html }` | Built | `model/src/message.rs:18` |
| `Draft.body: MessageBody` | Built | `model/src/draft.rs:122` |
| `drafts.body_html` column | Built | `0001_initial_schema.sql:320` |
| `multipart/alternative` on send | Built, round-trip tested | `model/src/outgoing.rs` |
| Incoming sanitisation (ammonia) | Built | `gtk/src/reader/sanitize.rs` |
| Quote folding | Built | `gtk/src/reader/quote.rs` |
| `Document`, HTML→text, HTML parse | **Absent** | — |
| Composer body | A `gtk::TextView` | `gtk/src/composer.rs:279` |

So the reader half is finished and in the wrong crate, and the composer half
does not exist. That is the shape of the work.

---

## Q1 — Which crate owns the document?

ADR 0003 said "`postio-model` owns the subset and its conversions", and the
architecture review said "`postio-model` (or `postio-ui`)". Both were written
before anyone counted the dependency cost. Counting it changes the answer.

**`postio-model` is the crate every other crate waits on.** It is depended on
by `postio-core`, `postio-search`, `postio-storage`, `postio-index`,
`postio-imap`, `postio-smtp`, `postio-sync`, `postio-runtime`, `postio-gtk` and
`postio-app` — the whole workspace, directly or through one hop. Its
dependency list today is four crates (`chrono`, `mail-builder`, `mail-parser`,
`serde`).

An HTML parser is not a small addition. `ammonia` pulls `html5ever`,
`markup5ever`, `tendril` and a generated tag table; putting it in
`postio-model` puts it in front of every `cargo test -p <crate>` in the
repository, including the nine that have no opinion about HTML whatsoever.
CLAUDE.md's build discipline exists because build time on this machine is
already the binding constraint; spending it on the domain leaf is the most
expensive possible place to spend it.

**Decision: a new crate, `postio-body`, at domain rank, beside
`postio-search`.**

```
  domain    postio-model     pure domain types + JWZ threading
            postio-search    query language                    ─┐ shared leaves,
            postio-body      message bodies, both directions   ─┘ not owned by
                                                                 one parent
```

`postio-body` depends on `postio-model` (for `Attachment` and content ids) and
on `ammonia`. Nothing else in the workspace acquires `ammonia` transitively
except the crates that actually render or compose a body: `postio-gtk` and
`postio-app`.

This is the same argument that already split `postio-search` from
`postio-index`, and the same shape: a pure leaf with no SQL and no toolkit,
shared by whoever needs it, rather than a capability buried in the frontend.

**What it means for `postio-model::outgoing`.** Nothing. `outgoing.rs` keeps
taking a `MessageBody { text, html }` and keeps building the
`multipart/alternative` — it does not learn about `Document`. The composer
renders `Document → MessageBody` and hands that down. That ordering is what
lets `postio-body` sit *above* `postio-model` instead of inside it, and it is
why the layering works out at all.

**What moves.** `reader/sanitize.rs` and `reader/quote.rs` move to
`postio-body` verbatim, tests included. `reader/view.rs`, `reader/scheme.rs`
and `reader/banner.rs` stay: they are WebKit, and WebKit is the frontend's
business. `CID_SCHEME` moves with the sanitiser, because the rewrite that
produces it does.

---

## Q2 — The document type

As ADR 0003 defined it, now pinned as the concrete type:

```rust
pub struct Document { pub blocks: Vec<Block> }

pub enum Block {
    Paragraph(Vec<Inline>),
    Heading { level: HeadingLevel, inlines: Vec<Inline> },   // 1..=3
    List { ordered: bool, items: Vec<Vec<Block>> },
    Quote(Vec<Block>),
    Pre(String),
    Rule,
}

pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Code(String),
    Link { href: Url, inlines: Vec<Inline> },
    Image { content_id: ContentId, alt: String },
    Break,
}
```

Four invariants, each of which is a test rather than a comment:

1. **`Image` carries a `ContentId`, never a URL.** There is no variant that can
   hold `https://tracker.example.com/pixel.gif`. A remote image in a quoted
   message therefore has no representation in a document Postio will send —
   not "is stripped", *has no representation*. This is the single most
   load-bearing line in the type.
2. **`Link.href` is parsed, and only `http`, `https` and `mailto` construct.**
   `javascript:` and `data:` fail to parse rather than being filtered later.
3. **The set is closed.** Ten inline kinds and six block kinds is the whole
   language. `to_text()` over a closed set is a total function you can read in
   one screen; over arbitrary HTML it is the reason most mail's plaintext part
   is unreadable. **Do not reach for `html2text`** — convert from `Document`.
4. **No styling.** No colours, no fonts, no `style`, no `class`. A body Postio
   sends renders in the recipient's client's own typography. This is not
   asceticism: every styling attribute is an attribute the sanitiser then has
   to reason about in both directions.

`Document` is `serde`-serialisable and `PartialEq`, which is what makes the
round-trip property testable.

---

## Q3 — Which form is the record?

**The canonical HTML string is the record; `Document` is the working form.**

```
   editor surface (GtkTextView v1, contenteditable later)
        │  edits
        ▼
   Document                        ← typed, in memory, in postio-body
        │  to_html()      to_text()
        ▼                    ▼
   drafts.body_html     drafts.body_text     ← the record, on disk and on the wire
```

Why the string and not the typed form: `drafts.body_html` already exists, a
draft has to survive a Postio upgrade that changed the `Document` enum, and the
bytes that get sent are HTML regardless. Storing a serialised `Document` would
add a second schema to migrate for no gain.

Two total functions, both tested as properties:

- `parse(to_html(d)) == d` for every `Document` — structure survives the round
  trip, which is issue #30's third acceptance criterion.
- `to_html(parse(h)) == h` for every `h` already in the subset — the serialiser
  is the *normal form*, so re-saving a draft is a no-op rather than a slow
  rewrite of the user's markup.

`parse` is total in the other direction too: it never fails, it *narrows*.
Anything outside the subset is dropped or downgraded, because the input to
`parse` on the quoting path is attacker-controlled and a parse error there
would be a denial of service on replying to a hostile message.

---

## Q4 — One allowlist, and why outgoing sanitisation is a backstop

The issue asks that "outgoing HTML is sanitised through the same allowlist the
reader uses". Taken literally that is the wrong mechanism, and the right one is
stronger.

**Outgoing HTML is *generated*, never passed through.** The only way a
sender's markup can reach an outgoing body is the quoting path, and quoting
runs `parse` — into a type that cannot hold a script, a remote image, an
`iframe`, a `style` attribute or a `javascript:` href, because those have no
variant. `to_html` then emits from that type. **The subset is the allowlist**,
enforced by the type system on the way in rather than by a filter on the way
out.

So there is one allowlist definition in `postio-body`, and it is used three
ways:

| Path | What runs | Remote `src` |
|---|---|---|
| Reader | `sanitize_body`, ammonia, `cid:` → `postio-cid:` | blocked, or allowed per-sender by explicit user action |
| Quote-into-reply | `parse` → `Document` | no representation |
| Send | `to_html` from `Document` | no representation |

**`RemoteImages::Allowed` exists only on the reader path.** It is not a
parameter of the outgoing path, not defaulted, not reachable — a user allowing
a sender's images to display must not thereby allow those images to be
re-emitted into a reply that goes back out to the world.

The final ammonia pass on the outgoing HTML stays, and is worth keeping, but
its status is honest: it is a **backstop that must never fire**. It gets a test
that asserts it changes nothing for every document the serialiser can produce.
A backstop that silently cleans up after a real bug is a backstop that hides
one.

Issue #30's fifth criterion — a quoted reply to a hostile corpus fixture emits
no script, no remote reference and no tracking pixel — becomes a corpus test in
`postio-body` over
`crates/postio-model/tests/corpus/`, and it fails at `parse` time if the type
ever grows a variant that could carry one.

---

## Q5 — Undo: two stacks, and they must not meet

`postio-core::undo` is the *mail* undo. An entry carries its inverse as
`Command`s, coalesces a burst into one unit, expires after
`UndoStack::EXPIRY`, and is bound to `u`.

Text editing undo is none of those things. It is per-keystroke-run, unbounded
within a draft, has no remote half, and dies with the composer.

**Decision: they are separate stacks, and the registry already keeps them
apart.** `Context::Composer` exists (`core/src/context.rs:32`) and the mail
verbs are not bound in it — `lookup_binding(Context::Composer, "a")` is
asserted `None` at `registry.rs:800`. `u` is the same case. Editing undo is
`Ctrl+Z`, handled by the composer, and it never touches `UndoStack`.

**Where the edit history lives: on the `Document`, in `postio-body`, not in the
widget.** This is the whole point of the ADR restated at the level where it is
easiest to get wrong. `GtkTextBuffer` has its own undo (`enable-undo`), and it
is free, and taking it would mean the GTK composer and a future
`contenteditable` composer disagree about what one undo step is — a
`GtkTextBuffer` step is a typing run in a flat buffer; a DOM step is whatever
the browser's editing command coalesced. `postio-body` defines an `EditStep` as
a `Document` delta, and both surfaces produce them. Widget-native undo is
**turned off explicitly**, with a comment saying why, because leaving it on is
the silent failure mode.

---

## Q6 — What ships in v1

> **Amended 2026-08-25, by the maintainer (#347).** The surface changed; the
> cut did not. Once ADR 0003's editing WebView existed as a tested component
> — the dialect contract, the hardened profile, the bridge — keeping the
> `GtkTextView` beside it meant every composer feature maintaining two
> surfaces forever. The decision: **the Editor is the composer surface for
> every draft**, visually plain until the formatting commands land, which
> this section's own argument already licenses: a `Document` of paragraphs
> *is* a plain-text document, so what was restricted was always the editor,
> not the document. `to_text()` still carries the `text/plain` part;
> `to_html()` now ships beside it. The paragraphs below record the original
> v1 shape this amendment supersedes.

**A plain-text composer over the neutral document.** A `Document` whose blocks
are all `Paragraph` and whose inlines are all `Text` *is* a plain-text
document — the model does not need restricting, the editor does. `to_text()` on
such a document is the text the user typed, and `to_html()` is a run of
`<p>` elements, which is what every other client sends anyway.

That gets the P0 decision paid for now, at MVP scope, and leaves the actual
rich editing surface where ADR 0003 and epic E10 already put it.

Order of work, each step green on its own:

1. Create `postio-body`; move `sanitize.rs` and `quote.rs` into it with their
   tests; `postio-gtk` depends on it and re-exports nothing.
2. `Document`, `to_html`, `to_text`, `parse`, with the two round-trip
   properties and the corpus test.
3. Composer reads and writes `Document`; `gtk::TextView` becomes a view over
   it; widget undo off; `Ctrl+Z` drives `EditStep`.
4. Quoting a reply goes through `parse` rather than through
   `reader/quote.rs`'s string handling. `reply.rs:157`'s "HTML-only content is
   not quoted" stops being true, which is the user-visible win.

---

## Alternatives

**`Document` in `postio-model`.** What ADR 0003 and the architecture review
both said. Rejected on build cost alone (Q1): it puts an HTML parser in front
of every crate in the workspace, and `postio-model` currently has four
dependencies for a reason.

**Keep sanitisation in `postio-gtk`, duplicate the outgoing half.** Two
allowlists that must agree forever and no compiler to make them. This is the
failure the issue was filed to prevent.

**A general HTML document type rather than a restricted subset.** Then
`to_text` is unbounded, the sanitiser is the only defence rather than the
backstop, and `Image` can hold a URL. Every property in Q2 and Q4 depends on
the subset being small.

**Store a serialised `Document` in `drafts`.** Adds a schema that has to be
migrated whenever the enum changes, to hold something that is derivable from
the HTML already stored. Rejected in Q3.

---

## Consequences

- `scripts/checks/check-crate-boundaries.py` gains nothing to guard here —
  `postio-body` has no SQL and no toolkit, and the existing `postio-gtk` rule
  is unaffected. Worth adding `postio-body` to the pure-leaf set if that check
  ever grows from two crates to a graph rule (`ARCHITECTURE.md`, known gaps).
- `docs/ARCHITECTURE.md` §13 stops being "decided, not yet built" for the
  *placement* half and gains the crate to the diagram.
- ~700 lines come out of `postio-gtk`, against the ~3,400 the review counted.
- A macOS composer becomes a view to write rather than a document model to
  re-derive, which is the entire reason this is P0 for a decision that ships no
  feature.
