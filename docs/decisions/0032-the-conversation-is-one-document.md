# ADR 0032 — The conversation is one document

- **Status:** Accepted (2026-09-09). Built: the terminal and macOS draw a conversation as one document (the classic app's reading pane did, until its removal in T256); the desktop app shows one message at a time (below). The screen-reader gate (#1424) is still open
- **Date:** 2026-09-06
- **Raised by:** the maintainer, asking: *"Why do we need a view per message in the conversation view? Isn't there a way to render all messages in the single view? Maybe with html?"*
- **Issue:** [#1216](https://github.com/dlapiduz/postio/issues/1216)
- **Revisits:** ADR 0015 Q4 (the conversation pane stacks every message of a thread)
- **Related:** [ADR 0042](0042-the-reading-renderer-is-disconnected-and-memory-safe.md) (the renderer that draws the document), ADR 0003 (no script in the reader), ADR 0023 (fonts named by URL), [ADR 0043](0043-focus-is-the-one-desktop-app.md) (Focus is the one desktop app), `PRODUCT.md` §20 (accessibility)
- **Decision:** **a conversation that is read as a stack is composed as one document**, with each message's chrome — its header, its verbs, its notices — expressed in that document, not as one rendering surface per message. `postio-render` draws it in Postio's own process (ADR 0042); the macOS frontend hands the same composed document to one web view.

---

## Where it applies

**Focus shows one message at a time.** Focus is the one desktop app (ADR 0043).
Its open message — in the dialog over the list, or in the pane beside it —
shows one message of the conversation, the latest by default, and `[` and `]`
step to the older and newer messages of the thread (`specs/007-postio-focus`
FR-037, decision C2). It opens a message rather than a thread, and draws it
with the shared `Reader` in a single-message mode. What it keeps from this ADR
is the cost argument: one surface serves every open, whatever the thread.

**The stacked conversation is one document** wherever a conversation is read as
a stack: the terminal's conversation view and the macOS conversation pane
(`postio-ffi`'s `conversation.rs`).

## Why one document

A rendering surface per message costs per message: in time to show a thread,
in memory, and — under a web engine — in a process each. One document costs
one surface whatever the thread's length; scrolling is the document's, not a
stack of scrollers; and expansion is state in the document rather than widget
lifecycle. Expansion needs no script: `<details>` and `<summary>` are a
disclosure widget in HTML itself, and the reader runs with no script (ADR 0003).

The alternatives were all smaller and worse: windowing a few per-message
surfaces still tears one down and rebuilds it at the edges; pre-warming a spare
surface hides the latency without removing it.

## What one document requires

**Containment.** Several senders' HTML share one document, so no message's CSS
may reach another's, or Postio's chrome. Each of these has a test, and each is
load-bearing:

* **A `<style>` block's selectors are rewritten** under the message's own
  container before the document is composed (`postio_body::styles`), so a rule
  naming `p` — or Postio's own chrome — matches only inside the message it
  arrived in. `@import` and `@font-face` are refused outright.
* **Inline declarations are contained** by `sanitize::contain_declarations` and
  the refusal tables, which drop what escapes a message's own block
  (`position`, `z-index`, the viewport units).
* **`contain_body`'s non-visible overflow** is what stops a `transform`
  painting over a neighbour (#1346). Removing or flattening it looks cosmetic
  and is not.
* **`style-src` names no source to fetch from**, so a sender's stylesheet
  cannot reach the network.
* Parsing to a tree rather than passing text through answers the unclosed
  element.

**Parts are addressed per message.** `postio-cid:` carries a per-message token,
because in one document "whichever message is open" is ambiguous. A sender's
own `postio-cid:` reference is dropped by the sanitiser — only `cid:` is
rewritten — so a URL in one message can never reach another message's parts.

**Remote images stay a per-sender decision**, carried in how each message's
images are addressed rather than in a document-wide policy.

**The verbs are links.** With no script, Reply, Archive, Unsubscribe and Show
images are links navigated through a Postio scheme and intercepted, so a
mistake is a refused navigation rather than an act.

## The gate still owed

A screen-reader pass over an HTML conversation, against the widget tree it
replaced, is the check this ADR named as deciding it. The maintainer accepted
the decision before it was run; the pass is [#1424], and if it finds the
document worse, that is a defect against this ADR rather than a reopening of
it.

[#1424]: https://github.com/dlapiduz/postio/issues/1424
