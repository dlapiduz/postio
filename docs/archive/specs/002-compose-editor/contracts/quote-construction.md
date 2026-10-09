# Contract: Quote Construction

**Producer**: `postio-body` — `replying.rs`, over `sanitize.rs` and `styles.rs`
**Consumers**: the composer (what it shows), `outgoing.rs` (what it sends)

**This contract changes.** It is the subject of the governance gate in
[plan.md](../plan.md): FR-044 supersedes the reply-construction property of ADR
0003 and ADR 0004, and the ADR lands before the code.

## Before

```rust
pub fn quoted_reply(source: &Document, attribution: &str) -> Document
```

Built from the closed authoring type, so anything outside the supported subset
had no representation. `replying.rs` states why that was chosen:

> a reply re-emits quoted content into the world, and building it from the
> closed type means a script or a tracking pixel has no representation rather
> than being stripped on the way out.

## After

The quote is the original **as the reader would render it**: sanitised HTML,
with the sender's stylesheet scoped.

| Guarantee | Rule |
|---|---|
| Fidelity | Structure and styling are carried, not rebuilt from a reduction (FR-044) |
| Provenance | Built from the same sanitised rendering the reader shows (FR-045) |
| Fallback | Where there is no renderable HTML, the text alternative is used — never an empty quote (FR-047) |
| Honesty | What is in the editor is what goes out (FR-046) |
| **No trackers travel** | The quote is sanitised with remote images **blocked**, whatever the reader was allowed to show — a per-sender allowance is the reader's own privacy decision (FR-047, ADR 0033 Q2) |
| **Safety** | Only what the reader would render may be re-emitted: no script, no remote-loading content, no tracking pixels (FR-047) |
| **Containment** | The sender's CSS is scoped to the quote (FR-078), by the mechanism `styles.rs` already implements for ADR 0032 |

## The safety rule is the contract, not a nicety

Rendering happens on one machine; re-emission puts markup in front of everyone
who receives the reply. The permitted set does not widen — `sanitize::REFUSED`
and `styles::Scoped` are the same gate for both — but the consequence of a hole
does. Tests for FR-047 are security tests over the `.eml` corpus, and should be
written to fail loudly: *zero* scripts, *zero* remote-loading references, *zero*
tracking pixels re-emitted, across every HTML message in it.

## Nesting

A quote may contain an earlier quote with its own styling. Scoping applies at
every level; `styles.rs`'s own module doc gives the reason in the reading case —
*"Admitting one unscoped would let message A restyle message B"* — and a draft
containing two generations of quoted mail is the same shape.

## How it is verified

- `postio-body` unit tests, no display needed: given an HTML original, the quote
  carries its structure; given one with a script or a remote image, neither
  appears in the output; given a plain-text-only original, the fallback is used.
- Corpus-wide assertions for FR-047, as above.
- An `app_suite` or `gtk_suite` case that a reply opened in the composer shows
  the quote and that the sent message contains what the editor showed (FR-046).
