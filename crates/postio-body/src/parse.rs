//! HTML in: narrowing arbitrary markup down to a [`Document`].
//!
//! # This function does not fail
//!
//! Its input on the quoting path is a message somebody else wrote, which is
//! to say attacker-controlled. A parse *error* there would mean a hostile
//! message you cannot reply to — a denial of service delivered by mail. So
//! [`parse`] is total: it never returns an error, it **narrows**. Anything
//! outside the subset is unwrapped to its text, or dropped entirely.
//!
//! # Why narrowing is safe rather than merely tidy
//!
//! Because the target type cannot express the dangerous thing. A `<script>`
//! is not "removed by a rule that might have a hole in it" — [`Inline`] has
//! no variant that could hold one. Same for a remote `<img src>`, an
//! `<iframe>`, a `style` attribute and a `javascript:` href. The narrowing
//! here is what makes the *shape* right; the type is what makes it safe.
//!
//! That is why `parse` is the outgoing path's whole defence and the ammonia
//! pass on the way out is only a backstop. See ADR 0004 Q4.
//!
//! # Dropped tag *and* contents
//!
//! Most unknown elements are unwrapped — a `<div>` or a `<span>` is a
//! container, and its text is the author's. `DROPPED` is the list where
//! that is wrong, and `<noscript>` is the interesting one: with scripting
//! off, the HTML spec has the parser treat its content as *markup* rather
//! than inert text, which turns a sender's "if you can't run our JavaScript,
//! at least load this" fallback into exactly the tracking pixel disabling
//! JavaScript was supposed to stop.

use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use html5ever::{LocalName, QualName, ns, parse_document, parse_fragment};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::document::{Block, ContentId, Document, HeadingLevel, Href, Inline};

/// Elements whose *contents* go with them.
///
/// Everything else unknown is unwrapped, keeping its text.
const DROPPED: [&str; 9] = [
    "script", "style", "iframe", "object", "embed", "svg", "math", "noscript", "template",
];

/// Narrow `html` to the subset Postio can hold.
///
/// Total. See the module docs for why that is a security property and not a
/// convenience.
pub fn parse(html: &str) -> Document {
    let dom = parse_fragment(
        RcDom::default(),
        ParseOpts::default(),
        QualName::new(None, ns!(html), LocalName::from("body")),
        Vec::new(),
        false,
    )
    .one(html);

    let mut blocks = Vec::new();
    let mut loose: Vec<Inline> = Vec::new();
    walk_blocks(&dom.document, &mut blocks, &mut loose);
    flush(&mut blocks, &mut loose);
    Document { blocks }
}

/// Parse a whole document rather than a fragment.
///
/// Only differs for input carrying `<html>`/`<head>`, which a mail body
/// often does. `<head>` content is dropped: a `<title>` is not body text.
pub fn parse_document_html(html: &str) -> Document {
    let dom = parse_document(RcDom::default(), ParseOpts::default()).one(html);
    let mut blocks = Vec::new();
    let mut loose: Vec<Inline> = Vec::new();
    walk_blocks(&dom.document, &mut blocks, &mut loose);
    flush(&mut blocks, &mut loose);
    Document { blocks }
}

/// Turn whatever inline content has accumulated into a paragraph.
///
/// Bare text between blocks is real content — `hello<p>world</p>` means two
/// paragraphs, not one — so it cannot simply be discarded when a block
/// element arrives.
fn flush(blocks: &mut Vec<Block>, loose: &mut Vec<Inline>) {
    let trimmed = trim_inlines(std::mem::take(loose));
    if !trimmed.is_empty() {
        blocks.push(Block::Paragraph(trimmed));
    }
}

fn name_of(handle: &Handle) -> Option<String> {
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
        .find(|attr| attr.name.local.as_ref().eq_ignore_ascii_case(wanted))
        .map(|attr| attr.value.to_string())
}

/// Walk `node`'s children, appending blocks and accumulating loose inlines.
fn walk_blocks(node: &Handle, blocks: &mut Vec<Block>, loose: &mut Vec<Inline>) {
    for child in node.children.borrow().iter() {
        match &child.data {
            NodeData::Text { contents } => {
                let text = contents.borrow().to_string();
                if !text.trim().is_empty() {
                    loose.push(Inline::Text(collapse(&text)));
                }
            }
            NodeData::Element { .. } => {
                let Some(name) = name_of(child) else { continue };
                if DROPPED.contains(&name.as_str()) || name == "head" {
                    continue;
                }
                match block_for(child, &name) {
                    // An empty paragraph records nothing, so it is narrowed
                    // away like any other content the subset cannot say.
                    // Editing machinery produces them: WebKit nests a new
                    // list inside the paragraph it formats, and spec
                    // recovery splits that into an empty `<p>` each side. A
                    // deliberate blank line is `<p><br></p>` — a Break —
                    // and is unaffected.
                    Some(Block::Paragraph(inlines)) if inlines.is_empty() => {
                        flush(blocks, loose);
                    }
                    Some(block) => {
                        flush(blocks, loose);
                        blocks.push(block);
                    }
                    // Not a block. Either an inline this subset knows, or a
                    // container whose children are the author's content.
                    None => match inline_for(child, &name) {
                        Some(inline) => loose.push(inline),
                        None => walk_blocks(child, blocks, loose),
                    },
                }
            }
            _ => walk_blocks(child, blocks, loose),
        }
    }
}

/// The block this element is, if it is one.
fn block_for(handle: &Handle, name: &str) -> Option<Block> {
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
        "blockquote" => Some(Block::Quote(blocks_of(handle))),
        // Whitespace is the content of a `<pre>`, so it is taken raw rather
        // than collapsed the way flowing text is.
        "pre" => Some(Block::Pre(raw_text(handle))),
        "hr" => Some(Block::Rule),
        _ => None,
    }
}

/// The inline this element is, if it is one.
fn inline_for(handle: &Handle, name: &str) -> Option<Inline> {
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
        // The one place a remote reference could try to get in, and it
        // cannot: only a `cid:` source has anywhere to go, because
        // `Inline::Image` holds a `ContentId` and nothing else.
        "img" => {
            let src = attribute(handle, "src")?;
            let cid = src.strip_prefix("cid:").or_else(|| {
                src.strip_prefix("CID:")
                    .or_else(|| src.strip_prefix("Cid:"))
            })?;
            Some(Inline::Image {
                content_id: ContentId::parse(cid)?,
                alt: attribute(handle, "alt").unwrap_or_default(),
            })
        }
        _ => None,
    }
}

/// The blocks inside a container element.
fn blocks_of(handle: &Handle) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut loose = Vec::new();
    walk_blocks(handle, &mut blocks, &mut loose);
    flush(&mut blocks, &mut loose);
    blocks
}

/// One run of blocks per `<li>`; anything not in an `<li>` is not an item.
fn items_of(handle: &Handle) -> Vec<Vec<Block>> {
    handle
        .children
        .borrow()
        .iter()
        .filter(|child| name_of(child).as_deref() == Some("li"))
        .map(|item| {
            let blocks = blocks_of(item);
            // A one-paragraph item is the common case and reads better
            // unwrapped, but the type wants blocks either way.
            if blocks.is_empty() {
                vec![Block::Paragraph(Vec::new())]
            } else {
                blocks
            }
        })
        .collect()
}

/// The inline content of an element, with block children flattened into it.
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
                    // A block inside inline context — a `<div>` in a `<p>`,
                    // which real mail does constantly. Its text is content.
                    None => gather_inlines(child, out),
                }
            }
            _ => {}
        }
    }
}

/// All descendant text, uncollapsed. For `<pre>` and `<code>`.
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

/// Collapse runs of HTML whitespace to single spaces, as a browser would.
///
/// A node's *edge* whitespace is content, and has to survive: the space in
/// `<strong>bold</strong> and <em>italic</em>` lives at the start of a text
/// node all by itself, and eating it silently welds two words together. That
/// is a bug the round-trip property found, which is what the property is for.
/// Paragraph edges are trimmed later, by [`trim_inlines`], where there is
/// enough context to know it is an edge.
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
        // Whitespace-only: one space, or nothing if there was nothing.
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

/// Drop leading and trailing whitespace-only text, so `<p> hi </p>` and
/// `<p>hi</p>` parse the same. Without this the round trip cannot hold.
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
