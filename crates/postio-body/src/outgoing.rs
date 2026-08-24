//! Rendering a [`Document`] for the wire, and the backstop that must never fire.
//!
//! # Why there is no outgoing allowlist
//!
//! Issue #30 asked that "outgoing HTML is sanitised through the same
//! allowlist the reader uses". Taken literally that is the weaker mechanism,
//! and ADR 0004 Q4 chose the stronger one.
//!
//! Outgoing HTML is **generated**, never passed through. The only route a
//! sender's markup can take into an outgoing body is the quoting path, and
//! quoting runs [`fn@crate::parse`] — into a type that cannot hold a script, a
//! remote image, an `iframe`, a `style` attribute or a `javascript:` href,
//! because those have no variant. [`Document::to_html`] then emits from that
//! type. **The subset is the allowlist**, enforced by the compiler on the way
//! in rather than by a filter on the way out.
//!
//! # So what is [`harden`] for?
//!
//! It is a backstop, and its status is honest: **it must never change
//! anything.** If it ever does, the serialiser has a bug and the right
//! response is to fix the serialiser, not to be glad the filter caught it. A
//! backstop that silently cleans up after a real defect is a backstop that
//! hides one — so `tests/outgoing.rs` asserts it is a no-op over every
//! document the serialiser can produce, including documents parsed from the
//! hostile corpus.
//!
//! # `RemoteImages::Allowed` is not reachable from here
//!
//! Deliberately. A user allowing a sender's images to *display* must not
//! thereby allow those images to be re-emitted into a reply that goes back
//! out to the world. That switch exists on the reader path and nowhere else.

use ammonia::Builder;
use std::collections::HashSet;

use crate::document::Document;

/// The tags [`Document::to_html`] can emit. Nothing else is reachable.
const EMITTED_TAGS: [&str; 16] = [
    "p",
    "h1",
    "h2",
    "h3",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "hr",
    "strong",
    "em",
    "code",
    "a",
    "img",
    "br",
];

/// The last pass before the bytes leave. See the module docs: this exists to
/// be a no-op, and there is a test that says so.
///
/// It is kept rather than deleted because "the type cannot express it" is an
/// argument about today's code, and the cost of being wrong is a tracking
/// pixel Postio sent on the user's behalf. Defence in depth is cheap here;
/// what is not acceptable is pretending it is the *primary* control.
pub fn harden(html: &str) -> String {
    Builder::default()
        .tags(HashSet::from(EMITTED_TAGS))
        .link_rel(None)
        .url_schemes(HashSet::from(["http", "https", "mailto", "cid"]))
        .clean(html)
        .to_string()
}

/// What a [`Document`] becomes on the wire: both alternatives.
///
/// The pair `postio_model::outgoing` already knows how to build a
/// `multipart/alternative` from. It takes a `MessageBody`, not a `Document` —
/// the composer renders down before handing off, and that ordering is what
/// lets this crate sit *above* `postio-model` rather than inside it.
pub fn render(document: &Document) -> (String, String) {
    (document.to_text(), harden(&document.to_html()))
}
