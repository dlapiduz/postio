//! HTML in: narrows arbitrary markup to a `Document`. Total, because the quoting-path input is
//! attacker-controlled and an error would make a hostile message impossible to reply to.
//! Safety comes from the type, which has no variant for script, remote images or `javascript:`;
//! `parse` is the outgoing path's whole defence and ammonia a backstop (ADR 0004 Q4).
//! `DROPPED` removes tag and contents: with scripting off `<noscript>` parses as markup, which
//! would turn its fallback into a tracking pixel.

use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use html5ever::{LocalName, QualName, ns, parse_fragment};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::document::{Block, ContentId, Document, HeadingLevel, Href, Inline};

/// Elements whose contents go with them; any other unknown element is unwrapped.
pub(crate) const DROPPED: [&str; 10] = [
    "script", "style", "iframe", "object", "embed", "svg", "math", "noscript", "template",
    // `summary` is chrome, not a hazard: the editor wraps a reply's quote in `<details><summary>`
    // and posts that DOM back on every edit, so its text would land in the draft.
    // A sender's own `<details>` inside a quote is untouched: quote markup is carried opaquely
    // (`Block::Quoted`).
    "summary",
];

pub fn parse(html: &str) -> Document {
    narrow(&fragment(html))
}

/// The tree `parse` narrows, for callers that take something out of it first (`own_text`).
pub(crate) fn fragment(html: &str) -> RcDom {
    parse_fragment(
        RcDom::default(),
        ParseOpts::default(),
        QualName::new(None, ns!(html), LocalName::from("body")),
        Vec::new(),
        false,
    )
    .one(html)
}

pub(crate) fn narrow(dom: &RcDom) -> Document {
    let mut blocks = Vec::new();
    let mut loose: Vec<Inline> = Vec::new();
    walk_blocks(&dom.document, &mut blocks, &mut loose);
    flush(&mut blocks, &mut loose);
    Document { blocks }
}

/// Bare text between blocks is real content (`hello<p>world</p>` is two paragraphs), so it cannot
/// be discarded.
fn flush(blocks: &mut Vec<Block>, loose: &mut Vec<Inline>) {
    let trimmed = trim_inlines(std::mem::take(loose));
    if !trimmed.is_empty() {
        blocks.push(Block::Paragraph(trimmed));
    }
}

pub(crate) fn name_of(handle: &Handle) -> Option<String> {
    match &handle.data {
        NodeData::Element { name, .. } => Some(name.local.to_string()),
        _ => None,
    }
}

fn attribute(handle: &Handle, wanted: &str) -> Option<String> {
    let NodeData::Element { attrs, .. } = &handle.data else {
        return None;
    };
    attrs
        .borrow()
        .iter()
        .find(|attr| str::eq_ignore_ascii_case(&attr.name.local, wanted))
        .map(|attr| attr.value.to_string())
}

fn walk_blocks(node: &Handle, blocks: &mut Vec<Block>, loose: &mut Vec<Inline>) {
    for child in node.children.borrow().iter() {
        match &child.data {
            NodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                if !text.trim().is_empty() {
                    loose.push(Inline::Text(collapse(&text)));
                } else if !ends_with_space(loose) {
                    // Whitespace-only is content only between two inlines (`<b>x</b> <i>y</i>`);
                    // elsewhere it is markup indentation (#1486).
                    // `ends_with_space` is true for an empty run, so only a genuine gap between
                    // inlines reaches here.
                    loose.push(Inline::Text(" ".to_owned()));
                }
            }
            NodeData::Element { .. } => {
                let Some(name) = name_of(child) else { continue };
                if DROPPED.contains(&name.as_str()) || name == "head" {
                    continue;
                }
                match block_for(child, &name) {
                    // An empty paragraph is narrowed away: WebKit nests a new list inside its
                    // paragraph and spec recovery leaves an empty `<p>` each side.
                    // A deliberate blank line is `<p><br></p>` and unaffected.
                    Some(Block::Paragraph(inlines)) if inlines.is_empty() => {
                        flush(blocks, loose);
                    }
                    Some(block) => {
                        flush(blocks, loose);
                        blocks.push(block);
                    }
                    None => match inline_for(child, &name) {
                        Some(inline) => loose.push(inline),
                        None => {
                            walk_blocks(child, blocks, loose);
                            separate_table_part(&name, blocks, loose);
                        }
                    },
                }
            }
            _ => walk_blocks(child, blocks, loose),
        }
    }
}

/// A table has no `Block`, so cells narrowed to `GateInterlock` and rows ran together, which
/// reached `text/plain` replies (#1482).
/// A cell ends with a space and a row ends the run.
fn separate_table_part(name: &str, blocks: &mut Vec<Block>, loose: &mut Vec<Inline>) {
    match name {
        "td" | "th" => {
            // Only where something was collected, and once: an empty cell would pad the row for
            // nothing.
            if !ends_with_space(loose) {
                loose.push(Inline::Text(" ".to_owned()));
            }
        }
        "tr" => flush(blocks, loose),
        _ => {}
    }
}

fn ends_with_space(loose: &[Inline]) -> bool {
    match loose.last() {
        None => true,
        Some(Inline::Text(text)) => text.ends_with(char::is_whitespace),
        Some(_) => false,
    }
}

pub(crate) fn block_for(handle: &Handle, name: &str) -> Option<Block> {
    match name {
        "p" => Some(Block::Paragraph(inlines_of(handle))),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let digit = name[1..].parse::<u8>().ok()?;
            Some(Block::Heading {
                level: HeadingLevel::from_digit(digit)?,
                inlines: inlines_of(handle),
            })
        }
        "ul" | "ol" => Some(Block::List {
            ordered: name == "ol",
            items: items_of(handle),
        }),
        "blockquote" => {
            // A reply quote comes back as one (ADR 0033), rebuilt through `quote_of` rather than
            // trusted: the content has since been through a `contenteditable` DOM.
            if attribute(handle, crate::document::QUOTED_MARKER).is_some() {
                let (html, text) = quoted_halves(handle);
                Some(Block::Quoted(crate::quote_of(
                    Some(&html),
                    &text,
                    QUOTE_SCOPE,
                )))
            } else {
                Some(Block::Quote(blocks_of(handle)))
            }
        }
        "pre" => Some(Block::Pre(raw_text(handle))),
        "hr" => Some(Block::Rule),
        // A forward's carried body is a `div`, since a forward is not a quote (#1483); only a
        // marked one, as an ordinary `div` is layout.
        "div" => attribute(handle, crate::document::QUOTED_MARKER)
            .map(|marker| carried_block(handle, &marker)),
        _ => None,
    }
}

/// Fixed, not carried through the round trip: the scope only has to be unique in the document, and
/// the original's id would put a database id on the wire.
pub const QUOTE_SCOPE: &str = "quote";

fn carried_block(handle: &Handle, marker: &str) -> Block {
    let (html, text) = quoted_halves(handle);
    let quoted = crate::quote_of(Some(&html), &text, QUOTE_SCOPE);
    Block::Quoted(if marker == "carried" {
        quoted.carried()
    } else {
        quoted
    })
}

/// Both halves come from the same nodes, because only the editor's DOM knows what the user did to
/// the quote.
/// The text half goes through ordinary narrowing, since `text/plain` is a reduction; the HTML half
/// must not be narrowed.
fn quoted_halves(handle: &Handle) -> (String, String) {
    let mut html = String::new();
    for child in handle.children.borrow().iter() {
        let mut buffer = Vec::new();
        let node = markup5ever_rcdom::SerializableHandle::from(child.clone());
        // `IncludeNode`: serializing only the children would hand back a table's text with the
        // table gone.
        let options = html5ever::serialize::SerializeOpts {
            traversal_scope: html5ever::serialize::TraversalScope::IncludeNode,
            ..Default::default()
        };
        if html5ever::serialize::serialize(&mut buffer, &node, options).is_ok() {
            html.push_str(&String::from_utf8_lossy(&buffer));
        }
    }
    let text = crate::Document {
        blocks: blocks_of(handle),
    }
    .to_text();
    (html, text)
}

pub(crate) fn inline_for(handle: &Handle, name: &str) -> Option<Inline> {
    match name {
        "strong" | "b" => Some(Inline::Strong(inlines_of(handle))),
        "em" | "i" => Some(Inline::Emphasis(inlines_of(handle))),
        "code" | "tt" | "kbd" | "samp" => Some(Inline::Code(raw_text(handle))),
        "br" => Some(Inline::Break),
        "a" => {
            let href = Href::parse(&attribute(handle, "href")?)?;
            Some(Inline::Link {
                href,
                inlines: inlines_of(handle),
            })
        }
        // Only a `cid:` source has anywhere to go: `Inline::Image` holds a `ContentId` and nothing
        // else.
        "img" => {
            let src = attribute(handle, "src")?;
            // Percent-decode before `ContentId::parse`, so a crafted src cannot decode into
            // something URL-shaped.
            let cid = if let Some(encoded) = src.strip_prefix("postio-cid:") {
                crate::sanitize::percent_decode(encoded)
            } else {
                src.strip_prefix("cid:")
                    .or_else(|| {
                        src.strip_prefix("CID:")
                            .or_else(|| src.strip_prefix("Cid:"))
                    })?
                    .to_owned()
            };
            Some(Inline::Image {
                content_id: ContentId::parse(&cid)?,
                alt: attribute(handle, "alt").unwrap_or_default(),
            })
        }
        _ => None,
    }
}

fn blocks_of(handle: &Handle) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut loose = Vec::new();
    walk_blocks(handle, &mut blocks, &mut loose);
    flush(&mut blocks, &mut loose);
    blocks
}

fn items_of(handle: &Handle) -> Vec<Vec<Block>> {
    handle
        .children
        .borrow()
        .iter()
        .filter(|child| name_of(child).as_deref() == Some("li"))
        .map(|item| {
            let blocks = blocks_of(item);
            if blocks.is_empty() {
                vec![Block::Paragraph(Vec::new())]
            } else {
                blocks
            }
        })
        .collect()
}

fn inlines_of(handle: &Handle) -> Vec<Inline> {
    let mut inlines = Vec::new();
    gather_inlines(handle, &mut inlines);
    trim_inlines(inlines)
}

fn gather_inlines(node: &Handle, out: &mut Vec<Inline>) {
    for child in node.children.borrow().iter() {
        match &child.data {
            NodeData::Text { contents } => {
                let text = collapse(&contents.borrow().to_string());
                if !text.is_empty() {
                    out.push(Inline::Text(text));
                }
            }
            NodeData::Element { .. } => {
                let Some(name) = name_of(child) else { continue };
                if DROPPED.contains(&name.as_str()) {
                    continue;
                }
                match inline_for(child, &name) {
                    Some(inline) => out.push(inline),
                    None => gather_inlines(child, out),
                }
            }
            _ => {}
        }
    }
}

fn raw_text(handle: &Handle) -> String {
    let mut out = String::new();
    fn walk(node: &Handle, out: &mut String) {
        for child in node.children.borrow().iter() {
            match &child.data {
                NodeData::Text { contents } => out.push_str(&contents.borrow().to_string()),
                NodeData::Element { .. } => {
                    if name_of(child).is_some_and(|n| DROPPED.contains(&n.as_str())) {
                        continue;
                    }
                    walk(child, out);
                }
                _ => {}
            }
        }
    }
    walk(handle, &mut out);
    out
}

/// Edge whitespace is content and survives (`<strong>bold</strong> and <em>italic</em>` would
/// otherwise weld two words); `trim_inlines` trims paragraph edges later.
fn collapse(text: &str) -> String {
    let leading = text.starts_with(char::is_whitespace);
    let trailing = text.ends_with(char::is_whitespace);
    let mut words = String::with_capacity(text.len());
    for (index, word) in text.split_whitespace().enumerate() {
        if index > 0 {
            words.push(' ');
        }
        words.push_str(word);
    }
    if words.is_empty() {
        return if leading || trailing {
            " ".to_owned()
        } else {
            String::new()
        };
    }
    let mut out = String::with_capacity(words.len() + 2);
    if leading {
        out.push(' ');
    }
    out.push_str(&words);
    if trailing {
        out.push(' ');
    }
    out
}

/// So `<p> hi </p>` and `<p>hi</p>` parse the same; the round trip needs it.
fn trim_inlines(mut inlines: Vec<Inline>) -> Vec<Inline> {
    while matches!(inlines.first(), Some(Inline::Text(t)) if t.trim().is_empty()) {
        inlines.remove(0);
    }
    while matches!(inlines.last(), Some(Inline::Text(t)) if t.trim().is_empty()) {
        inlines.pop();
    }
    if let Some(Inline::Text(first)) = inlines.first_mut() {
        *first = first.trim_start().to_owned();
    }
    if let Some(Inline::Text(last)) = inlines.last_mut() {
        *last = last.trim_end().to_owned();
    }
    inlines.retain(|inline| !matches!(inline, Inline::Text(t) if t.is_empty()));
    inlines
}
