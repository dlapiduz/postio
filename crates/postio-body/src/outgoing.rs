//! Rendering a `Document` for the wire, plus `harden`, a backstop that must never change anything.
//! There is no outgoing allowlist: HTML is generated from the closed `Document` type, so the subset
//! is the allowlist (ADR 0004 Q4).
//! `tests/outgoing.rs` asserts `harden` is a no-op over every serialiser output; a change means a
//! serialiser bug. `RemoteImages::Allowed` is unreachable here, so allowing display never re-emits
//! a remote image.

use ammonia::Builder;
use std::collections::HashSet;

use crate::document::Document;

/// The tags `Document::to_html` can emit. The table family is here because `Block::Quoted` can emit
/// anything `sanitize` admits (ADR 0033); leaving it out would silently undo the quote's fidelity.
/// Widening admits nothing dangerous: `ammonia` refuses `<script>`, `<iframe>` and `on*`
/// regardless.
const EMITTED_TAGS: [&str; 25] = [
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
    "table",
    "thead",
    "tbody",
    "tfoot",
    "tr",
    "td",
    "th",
    "colgroup",
    "col",
];

/// Defence in depth that must be a no-op (a test says so): "the type cannot express it" holds only
/// for today's code, and a tracking pixel sent on the user's behalf is costly.
pub fn harden(html: &str) -> String {
    let mut builder = Builder::default();
    builder
        .tags(HashSet::from(EMITTED_TAGS))
        .link_rel(None)
        .url_schemes(HashSet::from(["http", "https", "mailto", "cid"]))
        // A quote's styling travels as inline `style`: the sanitiser admits `style` and not
        // `class`.
        .add_generic_attributes(["style"])
        // Survives so the editor round trip keeps a reply quote distinct from one the user made;
        // carries no content.
        .add_tag_attributes("blockquote", [crate::document::QUOTED_MARKER]);
    for tag in [
        "table", "thead", "tbody", "tfoot", "tr", "td", "th", "colgroup", "col",
    ] {
        builder.add_tag_attributes(tag, crate::sanitize::TABLE_LAYOUT.iter().copied());
    }
    builder.clean(html).to_string()
}

/// Both alternatives for `postio_model::outgoing`, rendered down by the composer so this crate sits
/// above `postio-model`.
/// The text half is flowed (#333), wrapped here because `postio-model` has no HTML parser and
/// `assemble` advertises `format=flowed` unconditionally.
pub fn render(document: &Document) -> (String, String) {
    (document.to_flowed_text(), harden(&document.to_html()))
}
