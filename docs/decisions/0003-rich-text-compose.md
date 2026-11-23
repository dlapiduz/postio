# ADR 0003 — Rich-text (HTML) compose

- **Status:** Accepted (2026-08-24); the editing surface (Q2) is superseded by [ADR 0039](0039-the-composer-is-a-native-surface-over-the-document.md), and the reader is no longer a WebView ([ADR 0042](0042-the-reading-renderer-is-disconnected-and-memory-safe.md)).
- **Decision:** Postio has a real WYSIWYG rich-text composer. A restricted HTML subset is the canonical draft form, `text/plain` is generated from it, and `postio-body` owns the subset and its conversions.

## Rules
- The draft is a restricted subset: `Block = Paragraph | Heading(1..3) | List | Quote | Rule | Pre`; `Inline = Text | Strong | Emphasis | Code | Link | Image{content_id, alt} | Break`. No tables, colours, fonts or arbitrary CSS.
- `to_text()` is a total function over the subset. Never use a generic HTML-to-text crate.
- Outgoing HTML is generated from `Document`, never passed through. A sender's `<script>`, tracking pixel or `style` has no path into a sent message.
- `Image` carries a `ContentId`, never a URL. Pasted or dropped bytes become a local blob and an attachment with that `Content-ID`. Compose never fetches a remote URL: a pasted `<img src=https...>` is inlined or dropped.
- `reply()` takes the quote as an argument (`Option<Document>`), so `postio-model` stays free of an HTML parser. Quoting parses the source through the sanitiser into `Document` ([ADR 0033](0033-a-reply-quotes-what-the-reader-shows.md)).
- Replies and forwards carry no script outward. Both paths are tested against a hostile corpus fixture: Postio never makes a recipient run what its own user was protected from.
- Compose stays keyboard-first. Every formatting action is a registry command with a binding; a toolbar is a second route, never the only one.
- Signatures keep the RFC 3676 rule: `-- ` is last, and `apply()` replaces idempotently over a `Document`.
- Markdown-authored compose is rejected.

## Why
- Every peer client treats WYSIWYG as baseline. Markdown (aerc, muttdown, Fractal) makes the plaintext part authored; the cost of the rejected option is that `to_text()` must be derived well.
- A closed subset makes plaintext generation, sanitising and a second frontend's editor tractable.
- Quoting is the sharpest hazard: a reply re-emits markup, so the composer defends the recipient where the reader defends the user.

## Revisit when
- Users need tables, colours or fonts. Each is a new sanitising, round-trip and `to_text()` problem.
