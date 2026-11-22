# ADR 0004 — The composer's document, and where it lives

- **Status:** Accepted (2026-08-24), built as decided
- **Decision:** a domain-rank crate, `postio-body`, owns `Document`, the HTML subset, the parser, the serialiser, quoting and sanitisation in both directions. `postio-model` gains no HTML parser. Builds on [ADR 0003](0003-rich-text-compose.md).

## Rules
- `postio-body` depends on `postio-model` and `ammonia`, has no SQL and no toolkit. `postio-model::outgoing` keeps taking `MessageBody { text, html }`; the composer renders `Document -> MessageBody`.
- `Document` is `Vec<Block>` over the ADR 0003 subset. `Image` holds a `ContentId`: a remote image has no representation in a document Postio sends.
- The canonical HTML string (`drafts.body_html`) is the record; `Document` is the working form. No serialised `Document` is stored.
- Two properties are tested: `parse(to_html(d)) == d`, and `to_html(parse(h)) == h` for `h` in the subset (the serialiser is the normal form).
- `parse` never fails; it narrows. Input on the quoting path is attacker-controlled, so an error would be a denial of service on replying.
- One allowlist, three paths. Reader: `sanitize_body` (remote `src` blocked, or allowed per sender). Quote and send: no representation. `RemoteImages::Allowed` exists only on the reader path, so allowing a sender's images never re-emits them in a reply.
- The final ammonia pass on outgoing HTML is a backstop that must never fire. A test asserts it changes nothing for every serialisable `Document`.
- A quoted reply to a hostile corpus fixture emits no script, remote reference or tracking pixel. The test lives in `postio-body` over `crates/postio-model/tests/corpus/`.
- Two undo stacks never meet. Mail undo (`postio-core::undo`, `u`) is separate from edit undo (`Ctrl+Z`), which lives on the `Document` as `EditStep` deltas in `postio-body`. Widget-native undo is off, with a comment saying why. `Context::Composer` keeps mail verbs unbound.
- The composer surface edits every draft as a `Document`; plain text is a `Document` of paragraphs (the model is not restricted, the editor is).

## Why
- `postio-model` is the crate every other crate waits on; an HTML parser (`html5ever`, `markup5ever`) there taxes every `cargo test -p`. This mirrors the `postio-search` and `postio-index` split.
- Two allowlists in two crates would have to agree forever with no compiler to enforce it. A general HTML document type makes `to_text` unbounded and leaves the sanitiser as the only defence.
- Widget undo would make each editing surface disagree about what one step is.

## Revisit when
- The subset grows a variant that could carry a URL, script or style.
