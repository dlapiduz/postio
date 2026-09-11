# Contract: the conversation document

**What it governs**: what `postio-ui` hands a frontend for a whole
conversation, and what the frontend may not do to it. Both frontends are bound
by it (`postio-gtk` today, macOS via #1259/#1285).

Today's equivalent is `postio_ui::reader::document::document_for`, which
composes one message: `contain_body` → `scroll_markers` → `wrap_document`. Its
own doc comment says why it exists — those three steps *"were a `format!` at
the call site, and a second frontend composing its own would be free to forget
one"*, and forgetting `contain_body` loses a security affordance while looking
completely fine. The conversation form inherits that reasoning.

## The document

One document per conversation, handed to one rendering surface.

| Element | Required | Rule |
|---|---|---|
| Per-message container | yes | Each message gets its own `.postio-body` box (`contain_body`, #323) — a visible edge between what Postio wrote and what arrived |
| Per-message scope identity | yes | Stable within the document; every admitted sender rule is scoped to it (FR-020) |
| Per-message chrome | yes | Sender, date and per-message actions expressed in the document, since a rendering surface cannot contain toolkit widgets |
| Scroll markers | yes | `scroll_markers()`, the JS-free scroll primitive — application → document |
| CSP | yes | `content_security_policy()`, extended: `style-src` narrowed to refuse `@import` |
| Sheet | per message | `Sheet::Theme` for correspondence, `Sheet::Senders` for a message shown on the sender's own paper |

## Invariants

1. **A message's styling cannot leave its container.** Enforced at sanitize
   time by scoping and by the refused-property set, not by trusting the engine.
2. **A message cannot name a remote resource that loads.** Enforced twice: the
   sanitizer rewrites, and the CSP refuses. A sanitizer bug must degrade to
   broken markup, never to a live request.
3. **`cid:` references resolve per message.** With every message in one
   document, a handle that resolves against "whichever message is open" is
   ambiguous; URIs carry a per-message token and the handler routes on it.
4. **Postio's own words stay outside the sender's box.** Absent and empty
   states bypass `contain_body`, as they do today.
5. **Sender script never runs**, whatever the JavaScript setting is (R3).

## What a frontend may not do

- Compose its own document. It calls the `postio-ui` entry point.
- Add a rendering surface per message (FR-057).
- Reload the whole document because one message's content arrived (FR-062).
