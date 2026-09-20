# ADR 0033 — A reply quotes what the reader shows

- **Status:** Accepted (2026-09-10)
- **Date:** 2026-09-10
- **Decision by:** the maintainer, clarifying `specs/002-compose-editor/spec.md` on 2026-09-10. Offered four options for what survives a reply to rich HTML and chose fidelity over the reduced form, with the conflict stated in the option text.
- **Feature:** `specs/002-compose-editor/` — FR-044 to FR-047. Spec-driven work carries no issue (constitution 1.1.0).
- **Amends:** [ADR 0003](0003-rich-text-compose.md) Q3, whose quote is a `Document`; and [ADR 0004](0004-composer-document-model.md), which put that parse in `postio-body`.
- **Related:** [ADR 0032](0032-the-conversation-is-one-document.md) (the style scoping this relies on), `ARCHITECTURE.md` §11 (nothing leaves unasked), `PRODUCT.md` §21 (privacy)
- **Decision:** **a reply carries the sender's HTML as the reader sanitises it, not a reduction rebuilt from the closed `Document`.** The permitted set does not widen — it is the same sanitiser, the same refused declarations, the same style scoping. And the quote is always sanitised with remote images **blocked**, whatever the reader was allowed to show.

---

## What changes, precisely

ADR 0003 Q3 decided *how* a reply quotes an HTML message:

> Quoting means parsing untrusted HTML into `Document`, which **must** be
> sanitised: a reply re-emits quoted markup into the world, so the composer
> defends *the recipient* where the reader defends *the user*.

Two things are bundled in that sentence, and only one of them moves.

**The safety argument stays, and is the part that matters.** A reply re-emits
markup into the world; the composer defends the recipient. Nothing here
weakens that — the rest of this ADR is mostly about making it sharper.

**The representation changes.** `postio_body::replying::quoted_reply` takes a
`&Document` — the closed authoring type of the restricted subset — so anything
outside that subset has, in the crate's own words, *"no representation rather
than being stripped on the way out"*. That is a real guarantee and it is being
given up deliberately.

## Why

A quote that does not look like the message being answered is a quote the
recipient cannot recognise. Reply to a newsletter, a templated corporate
message, or anything with a table in it, and the reduced form loses the shape
that made it readable. Every mail client the user has come from quotes the
original as it appeared; Postio quoting something visibly poorer reads as a
defect, not as a security posture — and nobody can see the security posture.

The maintainer was given the reduced form as an option, with its advantage
stated, and chose fidelity.

## What we give up, said plainly

The closed type was a *structural* guarantee: a script had no representation,
so no amount of sanitiser error could emit one. What replaces it is a
*behavioural* guarantee: the sanitiser refuses it. Those are not equal. A
structural guarantee cannot regress; a behavioural one can, and now the
sanitiser is load-bearing in a direction it was not before — it defends
somebody who is not the user.

This is why FR-047's test is written as a security test over the whole
corpus, with numbers rather than adjectives: zero scripts, zero
remote-loading references, zero tracking pixels re-emitted. It is not a
rendering nicety and should never be relaxed into one.

## Q1 — What "sanitised" means here

**The reader's existing sanitiser, and no second policy.**
`postio-body`'s `sanitize.rs` plus `styles.rs` already decide what may be
rendered; a reply may re-emit exactly that and nothing more.

A stricter outbound policy was considered and rejected *for now*. It is
appealing — re-emission is riskier than local rendering — but two policies
drift, and the second one is always the one nobody remembers to update. If
measurement later shows the reader's policy is too permissive to send onward,
that is a new ADR, not a quiet divergence.

## Q2 — Remote images do not travel, whatever the reader was allowed

**The sharpest rule here, and the one that is easy to get wrong.**

`sanitize::RemoteImages` has two values, and with `Blocked` the sanitiser
*drops the remote reference from the markup* rather than merely declining to
load it. So a quote built with `Blocked` cannot carry a tracking pixel at all.

The trap is the other value. A user may allow remote images for a sender while
reading — that is a decision about **their own** privacy, taken with the
sender in front of them. Carrying that allowance into a reply would hand the
recipient a tracker they never agreed to, on the strength of somebody else's
decision, and it would do it invisibly.

**Therefore: a quote is always sanitised with `RemoteImages::Blocked`, even
when the message being replied to is displayed with them allowed.** The reply
and the reading of the same message are permitted different things, and this
is the one place where "what the reader would render" is not the whole answer.

## Q3 — Containment is not new, and that is the point

A quote carries the sender's stylesheet. ADR 0032 already had to solve this
for a harder case — several senders' messages in one document — so
`styles.rs` rewrites every rule under a per-message selector and runs its
declarations through the same refused table an inline attribute goes through.
Its module doc states the risk in the reading case: *"Admitting one unscoped
would let message A restyle message B."*

A draft holding a quote is the same shape: the sender's CSS must not reach the
user's own text, an earlier quote nested inside it, or Postio's chrome. No new
mechanism — the existing one, applied at every nesting level.

## Consequences

- `quoted_reply` takes the sanitised rendering rather than a `&Document`.
  `postio-body` keeps the parse and the sanitiser (ADR 0004 is otherwise
  untouched), and `postio-model` still receives a quote rather than computing
  one, so ADR 0003 Q3's dependency inversion stands.
- The corpus gains a security assertion that must stay loud.
- The authoring document is unchanged. This ADR is about the quote only —
  what the user *writes* is still the restricted subset of ADR 0003, and the
  markdown decision of 2026-09-10 did not change that either.
- A reply is now larger on the wire than it was, because it carries markup it
  previously reduced away. That interacts with the send-size total in FR-054.

## Alternatives considered

**Keep the `Document` quote.** What exists, and structurally safer. Rejected
by the maintainer: the quote does not look like the message being answered.

**Fidelity with a stricter outbound sanitiser.** The safest version of the
decision taken. Deferred rather than rejected — see Q1. It becomes attractive
the moment the corpus test finds something the reader admits and a reply
should not.

**Quote as plain text for HTML originals.** Unambiguous and safe, and worse
than what exists today for every rich message. Not seriously considered once
fidelity was the goal.
