# ADR 0033 — A reply quotes what the reader shows

- **Status:** Accepted (2026-09-10). Amends ADR 0003 Q3 and ADR 0004.
- **Decision:** A reply carries the sender's HTML as the reader sanitises it, not a reduction rebuilt from the closed `Document`. The permitted set does not widen. The quote is always sanitised with remote images blocked.

## Rules
- `quoted_reply` takes the sanitised rendering, not a `&Document`. `postio-body` keeps the parse and the sanitiser. `postio-model` receives a quote and does not compute one (ADR 0003 Q3 inversion stands).
- One policy: the reader's `sanitize.rs` and `styles.rs`. No second, stricter outbound policy. If measurement shows the reader too permissive to send onward, write a new ADR.
- A quote always uses `RemoteImages::Blocked`, which drops remote references from the markup, even when the user allowed images for the sender while reading. Otherwise the recipient receives a tracker on someone else's decision.
- The sender's CSS is scoped per message (ADR 0032) at every nesting level, so it cannot reach the user's text, an earlier quote or Postio's chrome.
- The corpus security test (FR-047) asserts zero scripts, zero remote-loading references and zero tracking pixels re-emitted. Never relax it into a rendering check.
- The authoring document is unchanged: what the user writes is still ADR 0003's restricted subset.
- A reply is larger on the wire and counts toward the send-size total (FR-054).

## Why
- A quote that does not look like the original is unrecognisable to the recipient. Reduced quotes lose newsletters and tables. The maintainer chose fidelity.
- The cost: the closed type was a structural guarantee that cannot regress. The sanitiser is a behavioural one that can, and it now defends the recipient.
- Rejected: keep the `Document` quote, plain-text quote for HTML originals.

## Revisit when
- The corpus test finds something the reader admits and a reply should not (a stricter outbound sanitiser).
