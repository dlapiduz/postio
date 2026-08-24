Work `postio-3o8f` — decide where the composer's document lives, before
Postio grows a rich text editor on top of the wrong answer.

## Read before anything else

- `bd show postio-3o8f` — design sketch and acceptance criteria.
- `docs/ARCHITECTURE.md` §13 (the composer's document is not the toolkit's
  buffer) and §11 (privacy), plus `docs/architecture-review-2026-08.md` §6.
- `crates/postio-gtk/src/composer.rs` — today the body is a `gtk::TextView`
  (line ~279) and plain text.
- `crates/postio-gtk/src/reader/{sanitize,quote,allowlist}.rs` — the incoming
  half of the same problem, already solved. Read it before inventing anything.
- `crates/postio-model/src/message.rs` — `MessageBody { text, html }`.

## What this bead is and is not

**It is a decision and a model.** Building the actual rich editing UI is
post-v1 (E10 / E12) and is *not* in scope. Shipping a plain-text composer on
top of a neutral document model is a perfectly good v1, and is the outcome to
aim for.

It is P0 despite rich text not being MVP because the cost is in *deferring the
decision* — every feature the plain composer grows against `GtkTextBuffer` is a
feature a second frontend has to reverse-engineer.

## The argument to hold on to

`GtkTextBuffer`, `NSTextStorage` and a `contenteditable` DOM genuinely disagree
about attribute runs versus nested spans, about what constitutes one undo step,
and about list and blockquote nesting. If composer state is "whatever is in the
buffer", a second frontend's composer is a rewrite rather than a port, and the
two produce different HTML from identical user gestures.

So: model the document outside the toolkit; each platform's editor is a *view*
over it; serialise to HTML from the neutral model.

## Constraints

- **The document type must not depend on a toolkit.** `postio-model` is the
  obvious home. Whatever you pick, `scripts/check-crate-boundaries.py` must
  stay green and no GTK type may leak into it.
- **Sanitisation is bidirectional, and the outgoing half is not the same
  problem.** The reader defends *you* from hostile mail. The composer defends
  *the world* from mail you quote: a reply to a hostile message re-emits that
  message's markup. Share the reader's allowlist rather than writing a second
  one — if that means the allowlist has to move somewhere both can reach, say
  so in the bead rather than duplicating it.
- **Undo granularity belongs to the document model, not the widget**, and must
  not fight `postio-core`'s `UndoStack` semantics (`ARCHITECTURE.md` §5).
- **Privacy** (`ARCHITECTURE.md` §11): nothing the composer emits may reference
  a remote resource the user did not put there.
- **TDD.** The round-trip and the hostile-fixture tests in the acceptance
  criteria are the design pressure — write them first and let them shape the
  type.

## A caution

Do not design a general-purpose rich text framework. The scope is *what an
email body needs*: paragraphs, emphasis, links, lists, blockquotes (quoting is
load-bearing — see `reader/quote.rs`), and inline images resolving from the
blob store. Anything past that is speculative and will be wrong.

## Done when

The bead's acceptance criteria pass, and the GTK composer reads and writes the
neutral document rather than owning body state in a `GtkTextBuffer`.
