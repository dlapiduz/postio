# Phase 0 Research: The Compose Editor

**Plan**: [plan.md](./plan.md) | **Spec**: [spec.md](./spec.md) | **Date**: 2026-09-10

Four questions the plan could not answer without reading the tree. Each was
checked against the code rather than assumed, because most of this feature
already exists and the expensive mistake here is specifying a rebuild.

---

## 1. Where does the editor's stylesheet belong?

**Decision**: a new `postio-ui/src/editor/document.rs`, mirroring the existing
`postio-ui/src/reader/document.rs`.

**Rationale**: the editor's document today is assembled in `postio-gtk`:

```rust
// crates/postio-gtk/src/editor.rs
let shell = format!(
    "<!doctype html><html><head>\
     <meta http-equiv=\"Content-Security-Policy\" content=\"{EDITOR_CSP}\">\
     </head><body contenteditable=\"true\">{inner_html}</body></html>"
);
```

No stylesheet at all — so it renders in WebKit's defaults while everything
around it uses the app's tokens, which is why FR-067 to FR-072 exist and why
dark mode shows a white page.

The reader already solved this and solved it in the right place. `postio-ui`
owns `Sheet`, `sheet_for`, `reader_ground`, `wrap_document`,
`content_security_policy` and `body_html`, re-exported into `postio-gtk` with
the comment *"one implementation for every frontend… what remains in this file
is webkit6 glue."* Putting the editor's equivalent anywhere else would make
`postio-gtk` and the macOS frontend each answer the question, and Principle VII
forbids exactly that.

**Alternatives considered**:
- *CSS in `postio-gtk/data/`* — fastest, and invisible to the macOS frontend,
  which would then need its own copy. Two answers to one question.
- *Style the WebView from GTK* — a `WebView` is not a themed widget; its
  document does not inherit the GTK theme. This is the same mistake that made
  the reader flash black before `paint_ground`.
- *Reuse `reader/document.rs` directly* — rejected but narrowly: the two differ
  in CSP (the editor needs `contenteditable` and its own script; the reader
  permits neither) and in what a quote must look like. Mirrored module,
  shared tokens, separate assembly.

---

## 2. What does "sanitised" mean for a quote that is re-emitted?

**Decision**: exactly what the reader would render, produced by the existing
`postio-body::sanitize` plus `styles::Scoped`, with no second policy.

**Rationale**: FR-045 already says a reply may re-emit only what Postio would
render when reading. That makes the reader's sanitiser the definition rather
than a new one, which matters because a second policy is a second thing to keep
correct — and this one has a security consequence.

The style-scoping half already exists and was built for a harder case. ADR 0032
put several senders' messages in one document, so `styles.rs` rewrites every
rule under `sanitize::message_selector` and runs its declarations through the
same `REFUSED` table an inline attribute goes through. Its own module doc states
the risk it removes: *"Admitting one unscoped would let message A restyle
message B."* A quote inside a draft is the same shape — the sender's CSS must
not reach the user's own text or Postio's chrome — so FR-076 is satisfied by
machinery that is already written and tested.

**What changes** is only the *input* to quote construction:

```rust
// crates/postio-body/src/replying.rs — today
pub fn quoted_reply(source: &Document, attribution: &str) -> Document
```

`Document` is the closed authoring type, so anything outside it "has no
representation rather than being stripped on the way out". FR-042 requires the
sanitised rendering instead.

**Alternatives considered**:
- *Keep building from `Document`* — what exists, and safer. Rejected by the
  maintainer on 2026-09-10: the quote does not then look like the message being
  answered.
- *A separate, stricter outbound sanitiser* — appealing, because re-emission is
  riskier than rendering. Rejected for now as a second policy that would drift
  from the first; if measurement later shows the reader's policy is too
  permissive for outbound use, that is an ADR, not a quiet divergence.

**Risk carried forward, stated plainly**: re-emitting sender markup is strictly
more dangerous than rendering it locally. The containment is FR-045 and FR-076,
and both must be tested as security properties over the corpus, not as
rendering niceties.

---

## 3. Where does the send-size limit come from?

**Decision**: the account's configuration, not the SMTP `SIZE` capability.

**Rationale**: there is no client-side size check today. SMTP `SIZE` is
advertised and readable — `Session::supports("SIZE")` exists — but
`postio-smtp/src/session.rs` is explicit that relying on it is the thing to
avoid:

> Postio's compliance argument for `SIZE` and `8BITMIME` is that it announces
> nothing and relies on nothing, and a capability list is exactly the thing that
> erodes that one `if server_supports` at a time.

Making the composer's refusal conditional on a capability would be that erosion.
A configured limit also matches the spec's own assumption and Principle VII:
providers are data, not code.

**Alternatives considered**:
- *Read `SIZE` from the server* — accurate, and exactly what the crate warns
  against. It also answers nothing when the server does not advertise it.
- *A constant* — one provider's number compiled into the code, which Principle
  VII forbids.
- *No check; let the server reject* — the rejection then arrives after the
  composer has closed, which FR-055 exists to prevent.

**Open, and deliberately left to the task**: what the composer does when no
limit is configured. The spec records it as an edge case; the safe reading is to
check nothing rather than invent a number.

---

## 4. Where does markdown input live?

**Decision**: `crates/postio-gtk/data/editor.js`, as an input transformation,
with the supported set bounded by the existing formatting commands.

**Rationale**: the editor already runs its own script under a CSP that permits
it (`EDITOR_SCRIPT`, `include_str!("../data/editor.js")`), and already reports
caret state back to Rust — `editor.rs` parses `bold`, `italic`, `bullet_list`,
`numbered_list`, `quote_block` out of that report. Markdown input is the same
kind of thing: a typed sequence becomes one of those commands.

FR-063 bounds it to formatting the editor already offers, which is what keeps
this from becoming a markdown dialect: no new document structure, no second
plaintext path, no mode. Tables and footnotes are out by construction.

**Alternatives considered**:
- *Convert in Rust on each keystroke* — a round trip per character across the
  WebView boundary, against a 16 ms budget.
- *Parse markdown on send* — changes what is sent away from what was shown,
  which FR-034 forbids.
- *A markdown-authored draft* — ADR 0003's rejected alternative, and rejected
  again on 2026-09-10 when this spec was clarified.

**Consequence for the macOS frontend**: this one is genuinely GTK-local, because
the script belongs to the WebView. The rule it implements (which sequences map
to which commands) should be stated in `postio-ui` so both frontends implement
the same set, even though the mechanism differs.

---

## Summary of unknowns resolved

| Unknown | Resolution |
|---|---|
| Editor stylesheet location | `postio-ui/src/editor/document.rs`, mirroring the reader |
| Meaning of "sanitised" for re-emission | The reader's existing sanitiser plus `styles::Scoped`; no second policy |
| Source of the size limit | Account configuration; never the `SIZE` capability |
| Markdown mechanism | `editor.js`, bounded by existing formatting commands; rule stated in `postio-ui` |

No `NEEDS CLARIFICATION` markers remain in the Technical Context.
