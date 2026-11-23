//! Rendering a `Document` for the wire, and `harden`, a backstop that must never change anything.
//! Outgoing HTML is generated from the closed type, so the subset is the allowlist (ADR 0004 Q4);
//! if `harden` changes something, fix the serialiser.
//! `RemoteImages::Allowed` is deliberately unreachable here: allowing a sender's images to display
//! must not re-emit them in a reply.

use ammonia::Builder;
use std::collections::HashSet;

use crate::document::Document;

/// The tags `to_html` can emit, including the table family because `Block::Quoted` can emit
/// anything the sanitiser admits (ADR 0033). Leaving them out would silently undo the fidelity the
/// quote exists for.
const EMITTED_TAGS: [&str; 29] = [
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "hr",
    "strong",
    "em",
    "del",
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

/// A backstop kept because "the type cannot express it" is an argument about today's code; a test
/// asserts it is a no-op.
pub fn harden(html: &str) -> String {
    let mut builder = Builder::default();
    builder
        .tags(HashSet::from(EMITTED_TAGS))
        .link_rel(None)
        .url_schemes(HashSet::from(["http", "https", "mailto", "cid"]))
        // The sender's own declarations carry a quote's styling: the sanitiser admits `style`, not
        // `class`.
        .add_generic_attributes(["style"])
        // Tells `parse` this blockquote is a reply quote; it must survive the round trip through
        // the editor.
        .add_tag_attributes("blockquote", [crate::document::QUOTED_MARKER])
        .add_tag_attributes("ol", ["start"])
        .add_tag_attributes("pre", [crate::document::LANGUAGE_MARKER])
        .add_tag_attributes("ul", [crate::document::TASK_MARKER])
        .add_tag_attributes("li", [crate::document::TASK_MARKER])
        .add_tag_attributes("a", [crate::document::REMOTE_IMAGE_MARKER]);
    for tag in [
        "table", "thead", "tbody", "tfoot", "tr", "td", "th", "colgroup", "col",
    ] {
        builder.add_tag_attributes(tag, crate::sanitize::TABLE_LAYOUT.iter().copied());
    }
    builder.clean(html).to_string()
}

/// The text half is `to_flowed_text` (#333): wrapping here, before the string crosses into
/// `postio-model`, lets `assemble` advertise `format=flowed` unconditionally.
pub fn render(document: &Document) -> (String, String) {
    (document.to_flowed_text(), harden(&document.to_html()))
}

/// The HTML part only when the document holds more than text paragraphs: a plain message goes out
/// as `text/plain; format=flowed` alone.
pub fn message_body(document: &Document) -> postio_model::MessageBody {
    let text = document.to_flowed_text();
    postio_model::MessageBody {
        text: (!text.is_empty()).then_some(text),
        html: (!document.is_plain_text()).then(|| harden(&document.to_html())),
    }
}
