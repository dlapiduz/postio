//! HTML in: narrowing arbitrary markup down to a `Document`. `parse` is total because its
//! quoting-path input is attacker-controlled.
//! Narrowing is safe because the target type cannot express a script, remote image or `javascript:`
//! href; the outgoing ammonia pass is a backstop (ADR 0004 Q4).
//! `DROPPED` removes tag and contents; `<noscript>` is listed because, parsed with scripting off,
//! its content becomes markup.

use html5ever::driver::ParseOpts;
use html5ever::tendril::TendrilSink;
use html5ever::{LocalName, QualName, ns, parse_fragment};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::document::{
    Alignment, Block, Cell, ContentId, DONE_BOX, Document, HeadingLevel, Href, Inline,
    LANGUAGE_MARKER, Language, ListItem, ListKind, OPEN_BOX, REMOTE_IMAGE_MARKER, TASK_MARKER,
    Table, Task, WebHref,
};

pub(crate) const DROPPED: [&str; 10] = [
    "script", "style", "iframe", "object", "embed", "svg", "math", "noscript", "template",
    // Chrome, not a hazard: the editor wraps a reply's quote in `<details><summary>` and the bridge
    // posts it back on every edit; without this "Quoted message" ended up in the draft.
    "summary",
];

pub fn parse(html: &str) -> Document {
    narrow(&fragment(html))
}

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

/// Bare text between blocks is content: `hello<p>world</p>` is two paragraphs.
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
                    // `flush` empties the run at block boundaries, so `ends_with_space` tells them
                    // apart (#1486).
                    loose.push(Inline::Text(" ".to_owned()));
                }
            }
            NodeData::Element { .. } => {
                let Some(name) = name_of(child) else { continue };
                if DROPPED.contains(&name.as_str()) || name == "head" {
                    continue;
                }
                match block_for(child, &name) {
                    // An empty paragraph is narrowed away: editors make them (WebKit nests a new
                    // list inside the paragraph). A deliberate blank line is `<p><br></p>`, a
                    // Break.
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

/// A cell ends with a space and a row ends the run, so `<td>Gate</td><td>Interlock</td>`
/// does not become `GateInterlock` in the text half (#1482).
fn separate_table_part(name: &str, blocks: &mut Vec<Block>, loose: &mut Vec<Inline>) {
    match name {
        "td" | "th" => {
            // A cell that already ends in whitespace needs no help; an empty one would pad the row.
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
        "ul" => Some(Block::List {
            kind: ListKind::Bullet,
            items: items_of(handle),
        }),
        "ol" => Some(Block::List {
            kind: ListKind::Ordered {
                start: attribute(handle, "start")
                    .and_then(|start| start.trim().parse().ok())
                    .unwrap_or(1),
            },
            items: items_of(handle),
        }),
        "table" => table_of(handle).map(|(table, _)| Block::Table(table)),
        "blockquote" => {
            // A reply quote comes back as one (ADR 0033): narrowing would lose its fidelity once
            // the user types.
            // Rebuilt through `quote_of`, not trusted, since it has been through a
            // `contenteditable` DOM.
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
        // Whitespace is the content of a `<pre>`: taken raw.
        "pre" => Some(Block::Code {
            language: attribute(handle, LANGUAGE_MARKER).and_then(|word| Language::parse(&word)),
            text: raw_text(handle),
        }),
        "hr" => Some(Block::Rule),
        // A forward's carried body is a `div` because a forward is not a quote (#1483); an ordinary
        // `div` is layout.
        "div" => attribute(handle, crate::document::QUOTED_MARKER)
            .map(|marker| carried_block(handle, &marker)),
        _ => None,
    }
}

/// Fixed: the scope only has to be unique within the document, and the original message's id would
/// put a database id on the wire.
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

/// Both halves come from the same nodes: only the editor's DOM knows what the user deleted from the
/// quote.
/// The text half is narrowed (a reduction anyway); the HTML half must not be.
fn quoted_halves(handle: &Handle) -> (String, String) {
    let mut html = String::new();
    for child in handle.children.borrow().iter() {
        let mut buffer = Vec::new();
        let node = markup5ever_rcdom::SerializableHandle::from(child.clone());
        // `IncludeNode`: the child's own tag carries the structure; its children alone lose the
        // table.
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
        "del" | "s" | "strike" => Some(Inline::Strikethrough(inlines_of(handle))),
        "code" | "tt" | "kbd" | "samp" => Some(Inline::Code(raw_text(handle))),
        "br" => Some(Inline::Break),
        "a" => {
            let raw = attribute(handle, "href")?;
            // Only a link carries the marker, so a remote `<img>` still has no shape.
            if attribute(handle, REMOTE_IMAGE_MARKER).is_some()
                && let Some(href) = WebHref::parse(&raw)
            {
                let mut alt = crate::Document {
                    blocks: vec![Block::Paragraph(inlines_of(handle))],
                }
                .to_search_text();
                if alt == href.as_str() {
                    alt.clear();
                }
                return Some(Inline::RemoteImage { href, alt });
            }
            let href = Href::parse(&raw)?;
            Some(Inline::Link {
                href,
                inlines: inlines_of(handle),
            })
        }
        // Only a `cid:` source has anywhere to go: `Inline::Image` holds a `ContentId`.
        "img" => {
            let src = attribute(handle, "src")?;
            // Percent-decode first, then `ContentId::parse`, so a crafted src cannot decode into
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

fn items_of(handle: &Handle) -> Vec<ListItem> {
    handle
        .children
        .borrow()
        .iter()
        .filter(|child| name_of(child).as_deref() == Some("li"))
        .map(|item| {
            let task = task_of(item);
            let mut blocks = blocks_of(item);
            if task.is_some() && attribute(item, TASK_MARKER).is_some() {
                strip_box(&mut blocks);
            }
            if blocks.is_empty() {
                blocks = vec![Block::Paragraph(Vec::new())];
            }
            ListItem { task, blocks }
        })
        .collect()
}

/// The marker this crate writes, or a pasted GitHub `<input type="checkbox">` as the first element.
fn task_of(item: &Handle) -> Option<Task> {
    match attribute(item, TASK_MARKER).as_deref() {
        Some("open") => return Some(Task::Open),
        Some("done") => return Some(Task::Done),
        _ => {}
    }
    let children = item.children.borrow();
    let first = children.iter().find(|child| match &child.data {
        NodeData::Text { contents } => !contents.borrow().trim().is_empty(),
        NodeData::Element { .. } => true,
        _ => false,
    })?;
    (name_of(first).as_deref() == Some("input")
        && attribute(first, "type").is_some_and(|kind| kind.eq_ignore_ascii_case("checkbox")))
    .then(|| {
        if attribute(first, "checked").is_some() {
            Task::Done
        } else {
            Task::Open
        }
    })
}

fn strip_box(blocks: &mut [Block]) {
    let Some(Block::Paragraph(inlines)) = blocks.first_mut() else {
        return;
    };
    if let Some(Inline::Text(first)) = inlines.first_mut() {
        let glyphs = [OPEN_BOX, DONE_BOX, OPEN_BOX.trim_end(), DONE_BOX.trim_end()];
        if let Some(rest) = glyphs.iter().find_map(|glyph| first.strip_prefix(glyph)) {
            *first = rest.to_owned();
        }
        if first.is_empty() {
            inlines.remove(0);
        }
    }
}

/// `None` for a table with a span or a block in a cell. The header is `<thead>`'s row or the first
/// row;
/// a headerless table one row or column wide is layout, not a grid.
pub(crate) fn table_of(handle: &Handle) -> Option<(Table, usize)> {
    let mut head = Vec::new();
    let mut body = Vec::new();
    for section in handle.children.borrow().iter() {
        match name_of(section).as_deref() {
            Some("thead") => head.extend(rows_of(section)),
            Some("tbody" | "tfoot") => body.extend(rows_of(section)),
            Some("tr") => body.push(section.clone()),
            Some("caption" | "colgroup") | None => {}
            Some(_) => return None,
        }
    }
    let declared = !head.is_empty();
    let count = head.len() + body.len();
    let mut rows = head.into_iter().chain(body);
    let header = rows.next()?;
    // A headerless table one row or column wide is layout (most commercial mail wraps itself in
    // one) (#1482).
    let named = declared
        || header
            .children
            .borrow()
            .iter()
            .any(|cell| name_of(cell).as_deref() == Some("th"));
    let width = cells_of(&header)?.len();
    if !named && (count < 2 || width < 2) {
        return None;
    }
    let alignments = cells_of(&header)?
        .iter()
        .map(|cell| {
            match attribute(cell, "align")
                .as_deref()
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("left") => Alignment::Left,
                Some("center") => Alignment::Center,
                Some("right") => Alignment::Right,
                _ => Alignment::None,
            }
        })
        .collect();
    let read =
        |row: &Handle| -> Option<Vec<Cell>> { cells_of(row)?.iter().map(cell_inlines).collect() };
    let header = read(&header)?;
    let rows = rows.map(|row| read(&row)).collect::<Option<Vec<_>>>()?;
    Some(Table::counting(alignments, header, rows))
}

fn rows_of(section: &Handle) -> Vec<Handle> {
    section
        .children
        .borrow()
        .iter()
        .filter(|child| name_of(child).as_deref() == Some("tr"))
        .cloned()
        .collect()
}

fn cells_of(row: &Handle) -> Option<Vec<Handle>> {
    let cells: Vec<Handle> = row
        .children
        .borrow()
        .iter()
        .filter(|child| matches!(name_of(child).as_deref(), Some("td" | "th")))
        .cloned()
        .collect();
    let spans = |cell: &Handle| {
        ["colspan", "rowspan"].iter().any(|name| {
            attribute(cell, name)
                .is_some_and(|value| value.trim().parse::<u32>().map_or(true, |n| n > 1))
        })
    };
    (!cells.iter().any(spans)).then_some(cells)
}

fn cell_inlines(cell: &Handle) -> Option<Cell> {
    fn holds_block(node: &Handle) -> bool {
        node.children.borrow().iter().any(|child| {
            name_of(child).is_some_and(|name| {
                !DROPPED.contains(&name.as_str())
                    && (matches!(
                        name.as_str(),
                        "p" | "div" | "ul" | "ol" | "li" | "table" | "pre" | "blockquote" | "hr"
                    ) || name.starts_with('h') && name.len() == 2 && name != "hr"
                        || holds_block(child))
            })
        })
    }
    (!holds_block(cell)).then(|| inlines_of(cell))
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

/// Edge whitespace is content: eating the space in `<strong>bold</strong> and <em>italic</em>`
/// welds two words.
/// Paragraph edges are trimmed later by `trim_inlines`.
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
