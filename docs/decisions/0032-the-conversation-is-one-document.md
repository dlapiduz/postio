# ADR 0032 — The conversation is one document

- **Status:** Accepted (2026-09-09). Screen-reader gate open (#1424).
- **Decision:** A conversation read as a stack is composed as one document, with each message's chrome expressed in it, not one rendering surface per message. `postio-render` draws it in-process (ADR 0042); macOS hands the same composed document to one web view.

## Rules
- Stacked conversation: the terminal's conversation view and the macOS pane (`postio-ffi` `conversation.rs`). Focus shows one message at a time (latest by default; `[` and `]` step) with the shared `Reader` in single-message mode (spec 007 FR-037).
- Expansion is `<details>`/`<summary>`. No script (ADR 0003).
- Containment, each tested:
  - A `<style>` block's selectors are rewritten under the message's container (`postio_body::styles`). `@import` and `@font-face` are refused.
  - Inline declarations pass `sanitize::contain_declarations` and the refusal tables (no `position`, `z-index`, viewport units).
  - `contain_body`'s non-visible overflow stops a `transform` painting over a neighbour (#1346). Do not flatten it.
  - `style-src` names no fetch source.
  - Parse to a tree, never pass text through.
- `postio-cid:` carries a per-message token. The sanitiser drops a sender's own `postio-cid:` references; only `cid:` is rewritten.
- Remote images stay a per-sender decision, carried in each message's image addressing.
- Verbs (Reply, Archive, Unsubscribe, Show images) are links through a Postio scheme and intercepted, so a mistake is a refused navigation.
- A screen-reader pass against the replaced widget tree is owed (#1424). If it finds the document worse, that is a defect against this ADR.

## Why
- A surface per message costs time, memory and (under a web engine) a process each. One document costs one surface and one scroller whatever the length.
- Windowing or pre-warming per-message surfaces hides the cost without removing it.

## Revisit when
- The screen-reader pass fails.
