//! Markdown, in both directions, for a frontend that draws text.
//!
//! [`from_html`] is the terminal reader's: the reader sanitiser's output, as
//! CommonMark a terminal can style. Only ever sanitised HTML -- converting
//! *after* the sanitiser is what keeps every guarantee it makes
//! (`docs/archive/specs/005-tui-frontend` research R5, `contracts/markdown.md`).
//!
//! Two things are Postio's own on the way out, because Markdown has no word
//! for them:
//!
//! * an image is `![alt](postio-image:N)`, a placeholder the terminal draws
//!   and never follows -- including an image whose remote `src` the sanitiser
//!   removed, which would otherwise vanish without a trace;
//! * a folded quote (`<details>`, from [`crate::fold_html_quotes`]) is its
//!   content between [`FOLD`] and [`END_FOLD`] lines, which the terminal
//!   turns into a block that expands.

/// The line that opens a folded quote in [`from_html`]'s output.
///
/// Private-use characters, so ordinary text does not look like one. A sender
/// who writes them can make text fold, which is all they can do with it.
pub const FOLD: &str = "\u{E000}fold\u{E000}";

/// The line that closes a folded quote.
pub const END_FOLD: &str = "\u{E000}end\u{E000}";

/// The scheme an image placeholder carries in [`from_html`]'s output.
pub const IMAGE_SCHEME: &str = "postio-image";

/// Sanitised HTML as Markdown for a terminal.
pub fn from_html(sanitized: &str) -> String {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use htmd::element_handler::{Context, Handlers};
    use htmd::{Element, HtmlToMarkdown};

    let images = Arc::new(AtomicUsize::new(0));
    let converter = HtmlToMarkdown::builder()
        .add_handler(vec!["img"], move |_: &dyn Handlers, element: &Element| {
            let alt = element
                .node
                .attr("alt")
                .map(|alt| alt.trim().to_owned())
                .filter(|alt| !alt.is_empty())
                .unwrap_or_else(|| "image".to_owned());
            let number = images.fetch_add(1, Ordering::Relaxed);
            // Brackets would end the alt text early; the placeholder's words
            // are the sender's, and only ever drawn, never followed.
            let alt = alt.replace(['[', ']'], "");
            Some(format!("![{alt}]({IMAGE_SCHEME}:{number})").into())
        })
        .add_handler(
            vec!["details"],
            |handlers: &dyn Handlers, element: &Element| {
                // A fold is a container: its quote begins blocks of its own.
                let inner = handlers.walk_children(element.node, Context::BLOCK);
                Some(format!("\n\n{FOLD}\n\n{}\n\n{END_FOLD}\n\n", inner.content.trim()).into())
            },
        )
        // The terminal draws its own line for a fold; the sanitiser's summary
        // words would only repeat it.
        .add_handler(vec!["summary"], |_: &dyn Handlers, _: &Element| {
            Some("".into())
        })
        .build();
    // A converter that cannot read what the sanitiser wrote has nothing to
    // show; the plain text is what the reader falls back to then.
    converter.convert(sanitized).unwrap_or_default()
}

/// Markdown, as the composer's own document.
///
/// What the terminal composer sends through: the HTML part is
/// [`crate::render`] of this, the generator the desktop composer uses, so
/// the same content makes the same HTML from either (`contracts/markdown.md`,
/// SC-006). Total -- every input is some document, and it never panics.
///
/// CommonMark with GFM's tables, strikethrough, task lists and bare
/// autolinks (spec 014 R1, R2). What the document has no node for stays the
/// words the person typed: raw HTML, a footnote. A link to anything but
/// `http`, `https` or `mailto` is its words and no link. An image is an image
/// when it names a part attached to this draft (`cid:`), and a
/// [`crate::Inline::RemoteImage`] when it names the web: a chip the person
/// may press to download and attach, sent as a link if they never do. Nothing
/// here fetches it (spec 014 D7; ADR 0003's privacy rule holds).
pub fn to_document(markdown: &str) -> crate::Document {
    to_document_mapped(markdown).0
}

/// As [`to_document`], with the [`SourceMap`] between the Markdown and the
/// document it reads into: what the source view places the caret with
/// (spec 014 FR-027).
pub fn to_document_mapped(markdown: &str) -> (crate::Document, SourceMap) {
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};

    // GFM's tables, strikethrough and task lists (spec 014 R1). Its bare
    // autolinks are not in the parser; `autolink` finds them below.
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut stack = vec![Frame::Blocks(Vec::new())];
    // Every visible character read, and where in the source it came from.
    let mut seen = Seen::default();
    let mut lost = Lost::default();
    for (event, range) in Parser::new_ext(markdown, options).into_offset_iter() {
        // GFM drops a row's cells past its header before any event says
        // they were there, so they are counted off the source line.
        if let (Event::Start(Tag::TableRow), Some(Frame::Table(alignments, _))) =
            (&event, stack.last())
        {
            let typed = typed_cells(markdown.get(range.clone()).unwrap_or_default());
            lost.cells += typed.saturating_sub(alignments.len());
        }
        match event {
            Event::Start(tag) => stack.push(match tag {
                Tag::Paragraph => Frame::Inlines(Holder::Paragraph, Vec::new()),
                Tag::Heading { level, .. } => Frame::Inlines(
                    Holder::Heading(
                        crate::HeadingLevel::from_digit(level as u8)
                            .unwrap_or(crate::HeadingLevel::Six),
                    ),
                    Vec::new(),
                ),
                Tag::BlockQuote(_) => Frame::Quote(Vec::new()),
                Tag::CodeBlock(kind) => {
                    // The info string's first word, if the type will hold it;
                    // the rest of the info string is dropped.
                    let language = match kind {
                        CodeBlockKind::Fenced(info) => info
                            .split_whitespace()
                            .next()
                            .and_then(crate::Language::parse),
                        CodeBlockKind::Indented => None,
                    };
                    Frame::Code(language, String::new())
                }
                Tag::List(first) => Frame::List(
                    match first {
                        Some(start) => crate::ListKind::Ordered {
                            start: u32::try_from(start).unwrap_or(u32::MAX),
                        },
                        None => crate::ListKind::Bullet,
                    },
                    Vec::new(),
                ),
                Tag::Item => Frame::Item(None, Vec::new(), Vec::new()),
                Tag::Emphasis => Frame::Inlines(Holder::Emphasis, Vec::new()),
                Tag::Strong => Frame::Inlines(Holder::Strong, Vec::new()),
                Tag::Strikethrough => Frame::Inlines(Holder::Strikethrough, Vec::new()),
                Tag::Link { dest_url, .. } => {
                    Frame::Inlines(Holder::Link(crate::Href::parse(&dest_url)), Vec::new())
                }
                Tag::Image { dest_url, .. } => Frame::Image(dest_url.into_string(), String::new()),
                Tag::Table(alignments) => Frame::Table(
                    alignments
                        .into_iter()
                        .map(|alignment| match alignment {
                            pulldown_cmark::Alignment::None => crate::Alignment::None,
                            pulldown_cmark::Alignment::Left => crate::Alignment::Left,
                            pulldown_cmark::Alignment::Center => crate::Alignment::Center,
                            pulldown_cmark::Alignment::Right => crate::Alignment::Right,
                        })
                        .collect(),
                    Vec::new(),
                ),
                Tag::TableHead | Tag::TableRow => Frame::Row(Vec::new()),
                Tag::TableCell => Frame::Inlines(Holder::Cell, Vec::new()),
                // A block of HTML: what it holds goes to its container.
                Tag::HtmlBlock => Frame::Blocks(Vec::new()),
                // Anything else the parser was not asked to know is not
                // reached; what is, keeps its words below.
                _ => Frame::Inlines(Holder::Transparent, Vec::new()),
            }),
            Event::End(_) => {
                // A mark opened as HTML and never closed ends with what
                // holds it.
                while matches!(stack.last(), Some(Frame::Inlines(Holder::Html(..), _))) {
                    let frame = stack.pop().expect("an open mark");
                    close(&mut stack, frame, &mut lost);
                }
                let Some(frame) = stack.pop() else { break };
                if stack.is_empty() {
                    stack.push(frame);
                    break;
                }
                close(&mut stack, frame, &mut lost);
            }
            Event::Text(text) => match stack.last_mut() {
                Some(Frame::Code(_, code)) => {
                    seen.run(markdown, &range, &text);
                    code.push_str(&text);
                }
                // An image's words are its alternative text, not a place
                // the caret can be.
                Some(Frame::Image(dest, alt)) => {
                    // A web image's words are where a caret can be: the
                    // rich view draws them as its link.
                    if crate::WebHref::parse(dest).is_some() {
                        seen.run(markdown, &range, &text);
                    }
                    alt.push_str(&text);
                }
                // Inside a link the words are its label, never another link.
                Some(Frame::Inlines(Holder::Link(_), _)) => {
                    seen.run(markdown, &range, &text);
                    push_inline(&mut stack, crate::Inline::Text(text.into_string()));
                }
                // A bare address is a link (GFM's extended autolinks) only
                // where it was typed as it reads: a backslash or an entity
                // in its source is someone saying "this is text", which is
                // how `from_document` keeps text text.
                _ if markdown.get(range.clone()) == Some(&*text) => {
                    seen.run(markdown, &range, &text);
                    let before = markdown[..range.start].chars().next_back();
                    for inline in crate::autolink::link_text_after(&text, before) {
                        push_inline(&mut stack, inline);
                    }
                }
                _ => {
                    seen.run(markdown, &range, &text);
                    push_inline(&mut stack, crate::Inline::Text(text.into_string()));
                }
            },
            Event::Code(code) => {
                seen.run(markdown, &range, &code);
                push_inline(&mut stack, crate::Inline::Code(code.into_string()));
            }
            // The few tags `from_document` writes where Markdown has no
            // spelling (spec 014's golden spellings): `<br>`, an empty
            // comment between two lists, and a mark CommonMark would not
            // read. Any other HTML is the words that were typed.
            Event::Html(html) => {
                let html = html.trim();
                if html == "<!-- -->" {
                    continue;
                }
                let breaks = html.matches("<br>").count();
                if breaks > 0 && html.replace("<br>", "").is_empty() {
                    push_block(
                        &mut stack,
                        crate::Block::Paragraph({
                            seen.run(markdown, &range, &"\n".repeat(breaks - 1));
                            vec![crate::Inline::Break; breaks - 1]
                        }),
                    );
                } else {
                    seen.run(markdown, &range, html);
                    push_inline(&mut stack, crate::Inline::Text(html.to_owned()));
                }
            }
            Event::InlineHtml(html) => match &*html {
                "<br>" => {
                    seen.run(markdown, &range, "\n");
                    push_inline(&mut stack, crate::Inline::Break);
                }
                "<strong>" => stack.push(Frame::Inlines(Holder::Html(Mark::Strong), Vec::new())),
                "<em>" => stack.push(Frame::Inlines(Holder::Html(Mark::Emphasis), Vec::new())),
                "<del>" => stack.push(Frame::Inlines(
                    Holder::Html(Mark::Strikethrough),
                    Vec::new(),
                )),
                "<code>" => stack.push(Frame::Inlines(Holder::Html(Mark::Code), Vec::new())),
                "</strong>" | "</em>" | "</del>" | "</code>"
                    if matches!(
                        stack.last(),
                        Some(Frame::Inlines(Holder::Html(mark), _)) if mark.closer() == &*html
                    ) =>
                {
                    let frame = stack.pop().expect("the open mark");
                    close(&mut stack, frame, &mut lost);
                }
                _ => {
                    seen.run(markdown, &range, &html);
                    push_inline(&mut stack, crate::Inline::Text(html.into_string()));
                }
            },
            Event::SoftBreak => {
                seen.run(markdown, &range, " ");
                push_inline(&mut stack, crate::Inline::Text(" ".to_owned()));
            }
            Event::HardBreak => {
                seen.run(markdown, &range, "\n");
                push_inline(&mut stack, crate::Inline::Break);
            }
            Event::Rule => push_block(&mut stack, crate::Block::Rule),
            Event::TaskListMarker(done) => {
                // On the item, which in a loose list is under its paragraph.
                if let Some(Frame::Item(task, ..)) = stack
                    .iter_mut()
                    .rev()
                    .find(|frame| matches!(frame, Frame::Item(..)))
                {
                    *task = Some(if done {
                        crate::Task::Done
                    } else {
                        crate::Task::Open
                    });
                }
            }
            _ => {}
        }
    }
    // Close whatever an unfinished input left open, innermost first.
    while stack.len() > 1 {
        let frame = stack.pop().expect("more than one");
        close(&mut stack, frame, &mut lost);
    }
    let blocks = match stack.pop() {
        Some(Frame::Blocks(blocks)) => blocks,
        _ => Vec::new(),
    };
    let document = crate::Document { blocks };
    let mut map = SourceMap::align(markdown, &document, seen.0);
    map.lost = lost;
    (document, map)
}

/// [`from_document`], with the [`SourceMap`] of what it wrote.
///
/// The map of the written Markdown read back, which is the document's own
/// map because the view loses nothing (FR-024).
pub fn from_document_mapped(document: &crate::Document) -> (String, SourceMap) {
    let written = from_document(document);
    let (_, map) = to_document_mapped(&written);
    (written, map)
}

/// A place in a document: the path of indexes down to the block that holds
/// text -- a paragraph, a heading, a code block or a table cell, counting a
/// list's items and a table's cells (header first) on the way -- then a
/// character offset in that block's text, where a break is one character.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocPosition {
    /// Indexes down to the block.
    pub path: Vec<usize>,
    /// Characters into its text.
    pub offset: usize,
}

/// Where each character of a document is in its Markdown, and back.
///
/// Used only to place the caret when the source view opens and closes
/// (spec 014 FR-027). Never stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceMap {
    /// Each visible character: its position, its source byte, and the
    /// length in the source of what spelled it.
    entries: Vec<(DocPosition, usize, usize)>,
    /// The byte offset where each source line starts.
    lines: Vec<usize>,
    /// The source's length.
    len: usize,
    /// What the Markdown said that the document could not hold.
    lost: Lost,
}

/// What typed Markdown said that its document narrowed away (spec 014
/// FR-026): the source view reports it rather than dropping it silently.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lost {
    /// Table cells past their header's width.
    pub cells: usize,
    /// Links to an address mail may not link to (not http, https, mailto).
    pub links: usize,
    /// Images naming neither a part of this draft nor the web.
    pub images: usize,
}

impl Lost {
    /// Whether nothing was narrowed.
    pub fn is_empty(&self) -> bool {
        *self == Lost::default()
    }
}

impl SourceMap {
    fn align(source: &str, document: &crate::Document, seen: Vec<(char, usize, usize)>) -> Self {
        let mut entries = Vec::new();
        let mut from = 0;
        for (position, c) in visible(document) {
            // Greedy: what was read but not kept (a paragraph's last
            // `<br>`) is passed over.
            let Some(at) = seen[from..].iter().position(|(read, ..)| *read == c) else {
                break;
            };
            let (_, byte, len) = seen[from + at];
            entries.push((position, byte, len));
            from += at + 1;
        }
        let lines = std::iter::once(0)
            .chain(source.match_indices('\n').map(|(at, _)| at + 1))
            .collect();
        SourceMap {
            entries,
            lines,
            len: source.len(),
            lost: Lost::default(),
        }
    }

    /// What the Markdown said that the document narrowed away.
    pub fn lost(&self) -> Lost {
        self.lost
    }

    /// The byte offset in the Markdown of `position`.
    pub fn to_source(&self, position: &DocPosition) -> usize {
        if let Some((_, byte, _)) = self.entries.iter().find(|(at, ..)| at == position) {
            return *byte;
        }
        // One past a block's last character: after what spelled it.
        if let Some(before) = position.offset.checked_sub(1) {
            let previous = DocPosition {
                path: position.path.clone(),
                offset: before,
            };
            if let Some((_, byte, len)) = self.entries.iter().find(|(at, ..)| *at == previous) {
                return byte + len;
            }
        }
        // A block with no text: where the next text is, or the end.
        self.entries
            .iter()
            .find(|(at, ..)| at > position)
            .map_or(self.len, |(_, byte, _)| *byte)
    }

    /// The position of the byte `offset` in the Markdown.
    ///
    /// Inside a marker (`**`, `## `, `- [ ] `), the first character it
    /// formats, on its line. Past the last character on its line -- text
    /// that was narrowed away -- the nearest position before it. On a line
    /// with no text of its own (a fence, a delimiter row), the next
    /// character; at the end, the end.
    pub fn to_document(&self, offset: usize) -> DocPosition {
        let offset = offset.min(self.len);
        let line = self
            .lines
            .partition_point(|start| *start <= offset)
            .saturating_sub(1);
        let start = self.lines[line];
        let end = self.lines.get(line + 1).copied().unwrap_or(self.len + 1);
        let on_line = |byte: usize| (start..end).contains(&byte);
        if let Some((at, ..)) = self
            .entries
            .iter()
            .find(|(_, byte, _)| *byte >= offset && on_line(*byte))
        {
            return at.clone();
        }
        if let Some((at, ..)) = self
            .entries
            .iter()
            .rev()
            .find(|(_, byte, _)| *byte < offset && on_line(*byte))
        {
            return DocPosition {
                path: at.path.clone(),
                offset: at.offset + 1,
            };
        }
        if let Some((at, ..)) = self.entries.iter().find(|(_, byte, _)| *byte >= offset) {
            return at.clone();
        }
        self.entries.last().map_or_else(
            || DocPosition {
                path: vec![0],
                offset: 0,
            },
            |(at, ..)| DocPosition {
                path: at.path.clone(),
                offset: at.offset + 1,
            },
        )
    }
}

/// The private character a caret is carried in, between the rich editor and
/// the document (spec 014 R14): the editor marks a copy of its body with it,
/// and `parse` keeps it where the caret was, so the caret's [`DocPosition`]
/// is read off the document `parse` builds rather than reasoned out from the
/// editor's own markup. Private use, so it is nobody's text.
pub const CARET: char = '\u{E002}';

/// The document with [`CARET`] taken out of it, and the position it marked.
pub fn take_caret(mut document: crate::Document) -> (crate::Document, Option<DocPosition>) {
    let position = visible(&document)
        .into_iter()
        .find(|(_, c)| *c == CARET)
        .map(|(position, _)| position);
    strip_caret_blocks(&mut document.blocks);
    (document, position)
}

/// `document` with [`CARET`] at `position`: what the rich editor is loaded
/// with to put its caret there.
pub fn put_caret(document: &crate::Document, position: &DocPosition) -> crate::Document {
    let mut marked = document.clone();
    if let Some(block) = block_at(&mut marked.blocks, &position.path) {
        put_caret_in(block, *position.path.last().unwrap_or(&0), position.offset);
    }
    marked
}

/// The block that holds text at `path`, or the cell of a table.
fn block_at<'a>(blocks: &'a mut [crate::Block], path: &[usize]) -> Option<&'a mut crate::Block> {
    use crate::Block;
    let (first, rest) = path.split_first()?;
    let container =
        !rest.is_empty() && matches!(blocks.get(*first)?, Block::List { .. } | Block::Quote(_));
    let block = blocks.get_mut(*first)?;
    if !container {
        // A table's cell is the last index, read by `put_caret_in`.
        return Some(block);
    }
    match block {
        Block::List { items, .. } => {
            let (item, rest) = rest.split_first()?;
            block_at(&mut items.get_mut(*item)?.blocks, rest)
        }
        Block::Quote(inner) => block_at(inner, rest),
        _ => None,
    }
}

fn put_caret_in(block: &mut crate::Block, cell: usize, offset: usize) {
    use crate::Block;
    match block {
        Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
            put_caret_inlines(inlines, offset);
        }
        Block::Code { text, .. } => {
            let at = text
                .char_indices()
                .nth(offset)
                .map_or(text.len(), |(at, _)| at);
            text.insert(at, CARET);
        }
        Block::Table(table) => {
            let width = table.width();
            let mut header = table.header().to_vec();
            let mut rows = table.rows().to_vec();
            let target = if cell < width {
                header.get_mut(cell)
            } else {
                rows.get_mut((cell - width) / width)
                    .and_then(|row| row.get_mut((cell - width) % width))
            };
            if let Some(target) = target {
                put_caret_inlines(target, offset);
            }
            *table = crate::Table::new(table.alignments().to_vec(), header, rows);
        }
        Block::List { .. } | Block::Quote(_) | Block::Quoted(_) | Block::Rule => {}
    }
}

/// Put the caret `offset` visible characters into `inlines`. Returns what
/// is left of the offset when it runs past them.
fn put_caret_inlines(inlines: &mut Vec<crate::Inline>, mut offset: usize) -> Option<usize> {
    use crate::Inline;
    let mut index = 0;
    while index < inlines.len() {
        match &mut inlines[index] {
            Inline::Text(text) | Inline::Code(text) => {
                let len = text.chars().count();
                if offset <= len {
                    let at = text
                        .char_indices()
                        .nth(offset)
                        .map_or(text.len(), |(at, _)| at);
                    text.insert(at, CARET);
                    return None;
                }
                offset -= len;
            }
            Inline::Strong(inner) | Inline::Emphasis(inner) | Inline::Strikethrough(inner) => {
                offset = put_caret_inlines(inner, offset)?;
            }
            Inline::Link { inlines: inner, .. } => offset = put_caret_inlines(inner, offset)?,
            Inline::Break => {
                if offset == 0 {
                    inlines.insert(index, Inline::Text(CARET.to_string()));
                    return None;
                }
                offset -= 1;
            }
            // A remote image is drawn as its words, a link, so the caret
            // can be in them.
            Inline::RemoteImage { alt, .. } => {
                let len = alt.chars().count();
                if offset <= len {
                    let at = alt
                        .char_indices()
                        .nth(offset)
                        .map_or(alt.len(), |(at, _)| at);
                    alt.insert(at, CARET);
                    return None;
                }
                offset -= len;
            }
            Inline::Image { .. } => {}
        }
        index += 1;
    }
    if offset == 0 {
        inlines.push(Inline::Text(CARET.to_string()));
        return None;
    }
    Some(offset)
}

fn strip_caret_blocks(blocks: &mut [crate::Block]) {
    use crate::Block;
    for block in blocks {
        match block {
            Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
                strip_caret_inlines(inlines)
            }
            Block::Code { text, .. } => text.retain(|c| c != CARET),
            Block::List { items, .. } => {
                for item in items {
                    strip_caret_blocks(&mut item.blocks);
                }
            }
            Block::Quote(inner) => strip_caret_blocks(inner),
            Block::Table(table) => {
                let clean = |row: &[crate::Cell]| -> Vec<crate::Cell> {
                    row.iter()
                        .map(|cell| {
                            let mut cell = cell.clone();
                            strip_caret_inlines(&mut cell);
                            cell
                        })
                        .collect()
                };
                *table = crate::Table::new(
                    table.alignments().to_vec(),
                    clean(table.header()),
                    table.rows().iter().map(|row| clean(row)).collect(),
                );
            }
            Block::Quoted(_) | Block::Rule => {}
        }
    }
}

fn strip_caret_inlines(inlines: &mut Vec<crate::Inline>) {
    use crate::Inline;
    for inline in inlines.iter_mut() {
        match inline {
            Inline::Text(text) | Inline::Code(text) => text.retain(|c| c != CARET),
            Inline::Strong(inner) | Inline::Emphasis(inner) | Inline::Strikethrough(inner) => {
                strip_caret_inlines(inner);
            }
            Inline::Link { inlines, .. } => strip_caret_inlines(inlines),
            Inline::RemoteImage { alt, .. } => alt.retain(|c| c != CARET),
            Inline::Break | Inline::Image { .. } => {}
        }
    }
    inlines.retain(|inline| !matches!(inline, Inline::Text(text) if text.is_empty()));
}

/// What the parser read, character by character.
#[derive(Default)]
struct Seen(Vec<(char, usize, usize)>);

impl Seen {
    /// The characters of `text`, read from `range`: one for one where the
    /// source spells them as they are, else all at where the run began (an
    /// escape, an entity).
    fn run(&mut self, source: &str, range: &std::ops::Range<usize>, text: &str) {
        let slice = source.get(range.clone()).unwrap_or_default();
        if let Some(inner) = slice.find(text).filter(|_| !text.is_empty()) {
            for (at, c) in text.char_indices() {
                self.0.push((c, range.start + inner + at, c.len_utf8()));
            }
        } else {
            for c in text.chars() {
                self.0.push((c, range.start, range.len()));
            }
        }
    }
}

/// Every character the caret can be on, in order, with its position.
fn visible(document: &crate::Document) -> Vec<(DocPosition, char)> {
    let mut out = Vec::new();
    for (index, block) in document.blocks.iter().enumerate() {
        visible_block(block, vec![index], &mut out);
    }
    out
}

fn visible_block(block: &crate::Block, path: Vec<usize>, out: &mut Vec<(DocPosition, char)>) {
    use crate::Block;
    let leaf = |path: Vec<usize>, chars: Vec<char>, out: &mut Vec<(DocPosition, char)>| {
        for (offset, c) in chars.into_iter().enumerate() {
            out.push((
                DocPosition {
                    path: path.clone(),
                    offset,
                },
                c,
            ));
        }
    };
    match block {
        Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
            leaf(path, visible_inlines(inlines), out);
        }
        Block::Code { text, .. } => leaf(path, text.chars().collect(), out),
        Block::List { items, .. } => {
            for (item, list_item) in items.iter().enumerate() {
                for (index, block) in list_item.blocks.iter().enumerate() {
                    let mut inner = path.clone();
                    inner.extend([item, index]);
                    visible_block(block, inner, out);
                }
            }
        }
        Block::Quote(blocks) => {
            for (index, block) in blocks.iter().enumerate() {
                let mut inner = path.clone();
                inner.push(index);
                visible_block(block, inner, out);
            }
        }
        Block::Table(table) => {
            for (index, cell) in table.cells().enumerate() {
                let mut inner = path.clone();
                inner.push(index);
                leaf(inner, visible_inlines(cell), out);
            }
        }
        Block::Quoted(_) | Block::Rule => {}
    }
}

fn visible_inlines(inlines: &[crate::Inline]) -> Vec<char> {
    use crate::Inline;
    let mut out = Vec::new();
    for inline in inlines {
        match inline {
            Inline::Text(text) | Inline::Code(text) => out.extend(text.chars()),
            Inline::Strong(inner) | Inline::Emphasis(inner) | Inline::Strikethrough(inner) => {
                out.extend(visible_inlines(inner));
            }
            Inline::Link { inlines, .. } => out.extend(visible_inlines(inlines)),
            Inline::Break => out.push('\n'),
            // Drawn as its words in the rich view, a link to its address.
            Inline::RemoteImage { alt, .. } => out.extend(alt.chars()),
            Inline::Image { .. } => {}
        }
    }
    out
}

/// The document in the normal form the Markdown view writes and reads
/// (spec 014 FR-024): adjacent text runs merged, empty ones dropped,
/// adjacent code spans and adjacent marks of one kind merged, and a
/// non-empty code block ending in a line end. Each is a difference no reader can see. Idempotent.
pub fn normalize(document: crate::Document) -> crate::Document {
    crate::Document {
        blocks: document.blocks.into_iter().map(normalize_block).collect(),
    }
}

fn normalize_block(block: crate::Block) -> crate::Block {
    use crate::Block;
    match block {
        Block::Paragraph(inlines) => Block::Paragraph(normalize_inlines(inlines)),
        Block::Heading { level, inlines } => Block::Heading {
            level,
            inlines: normalize_inlines(inlines),
        },
        Block::List { kind, items } => Block::List {
            kind,
            items: items
                .into_iter()
                .map(|item| crate::ListItem {
                    task: item.task,
                    blocks: item.blocks.into_iter().map(normalize_block).collect(),
                })
                .collect(),
        },
        Block::Quote(blocks) => Block::Quote(blocks.into_iter().map(normalize_block).collect()),
        Block::Code { language, mut text } => {
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            Block::Code { language, text }
        }
        Block::Table(table) => {
            let cells = |row: &[crate::Cell]| -> Vec<crate::Cell> {
                row.iter()
                    .map(|cell| normalize_inlines(cell.clone()))
                    .collect()
            };
            Block::Table(crate::Table::new(
                table.alignments().to_vec(),
                cells(table.header()),
                table.rows().iter().map(|row| cells(row)).collect(),
            ))
        }
        other @ (Block::Quoted(_) | Block::Rule) => other,
    }
}

fn normalize_inlines(inlines: Vec<crate::Inline>) -> Vec<crate::Inline> {
    use crate::Inline;
    let mut out: Vec<Inline> = Vec::with_capacity(inlines.len());
    for inline in inlines {
        // A mark inside the same mark reads as the one mark.
        fn flat(
            inner: Vec<Inline>,
            same: fn(Inline) -> Result<Vec<Inline>, Inline>,
        ) -> Vec<Inline> {
            normalize_inlines(
                normalize_inlines(inner)
                    .into_iter()
                    .flat_map(|inline| same(inline).unwrap_or_else(|other| vec![other]))
                    .collect(),
            )
        }
        let inline = match inline {
            Inline::Strong(inner) => Inline::Strong(flat(inner, |i| match i {
                Inline::Strong(nested) => Ok(nested),
                other => Err(other),
            })),
            Inline::Emphasis(inner) => Inline::Emphasis(flat(inner, |i| match i {
                Inline::Emphasis(nested) => Ok(nested),
                other => Err(other),
            })),
            Inline::Strikethrough(inner) => Inline::Strikethrough(flat(inner, |i| match i {
                Inline::Strikethrough(nested) => Ok(nested),
                other => Err(other),
            })),
            Inline::Link { href, inlines } => Inline::Link {
                href,
                inlines: normalize_inlines(inlines),
            },
            Inline::Text(text) if text.is_empty() => continue,
            other => other,
        };
        match (out.last_mut(), inline) {
            (Some(Inline::Text(last)), Inline::Text(more)) => last.push_str(&more),
            (Some(Inline::Code(last)), Inline::Code(more)) => last.push_str(&more),
            // Two marks of one kind side by side read as one.
            (Some(Inline::Strong(last)), Inline::Strong(more))
            | (Some(Inline::Emphasis(last)), Inline::Emphasis(more))
            | (Some(Inline::Strikethrough(last)), Inline::Strikethrough(more)) => {
                last.extend(more);
                let merged = normalize_inlines(std::mem::take(last));
                *last = merged;
            }
            (_, inline) => out.push(inline),
        }
    }
    out
}

/// The composer's document, as Markdown the terminal composer can edit.
///
/// What opens a draft written in the desktop composer: its HTML, parsed to
/// the document, written back out as the Markdown that reads into that same
/// document -- `to_document(&from_document(d)) == d` for any document without
/// a carried quote. A quote carried from the message being answered
/// (`Block::Quoted`) is not Markdown and is not written here; the composer
/// keeps it whole beside the text.
pub fn from_document(document: &crate::Document) -> String {
    let mut out = String::new();
    write_blocks(&mut out, &document.blocks, "");
    out
}

/// `blocks`, each line after the first of each block prefixed with `indent`
/// (a list item's continuation, a quote's `> `), a blank line between them.
///
/// Two lists of one kind side by side would read as one, so an empty
/// comment stands between them (spec 014's golden spellings).
fn write_blocks(out: &mut String, blocks: &[crate::Block], indent: &str) {
    let mut previous: Option<&crate::Block> = None;
    for block in blocks {
        let mut text = String::new();
        if !write_block(&mut text, block) {
            continue;
        }
        if let Some(previous) = previous {
            let blank = |out: &mut String| {
                out.push_str(indent.trim_end());
                out.push('\n');
                out.push_str(indent);
            };
            blank(out);
            if same_list_kind(previous, block) {
                out.push_str("<!-- -->\n");
                blank(out);
            }
        }
        previous = Some(block);
        out.push_str(&text.replace('\n', &format!("\n{indent}")));
        out.push('\n');
    }
}

/// Whether two lists would merge into one if nothing stood between them.
fn same_list_kind(a: &crate::Block, b: &crate::Block) -> bool {
    match (a, b) {
        (crate::Block::List { kind: a, .. }, crate::Block::List { kind: b, .. }) => {
            a.is_ordered() == b.is_ordered()
        }
        _ => false,
    }
}

/// Where a run of inlines sits, which decides how its edges are spelled.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// A paragraph: a break is `\` at the end of a line, except where
    /// Markdown cannot put one.
    Paragraph,
    /// A heading or a table cell: one line, so every break is `<br>`.
    Line,
    /// Inside a mark or a link: the container spells the edges.
    Inner,
}

/// One block, without its trailing newline; `false` for one Markdown does
/// not carry (a quote carried from the message being answered).
fn write_block(out: &mut String, block: &crate::Block) -> bool {
    use crate::Block;
    match block {
        Block::Paragraph(inlines) => {
            // A paragraph of nothing but breaks has no words to hang them
            // on, so it is that many `<br>`s and one more (spec 014: an
            // empty paragraph is a line holding only `<br>`).
            if inlines
                .iter()
                .all(|inline| matches!(inline, crate::Inline::Break))
            {
                out.push_str(&"<br>".repeat(inlines.len() + 1));
            } else {
                write_inlines(out, inlines, Place::Paragraph);
            }
        }
        Block::Heading { level, inlines } => {
            out.push_str(&"#".repeat(usize::from(level.digit())));
            out.push(' ');
            write_inlines(out, inlines, Place::Line);
        }
        Block::List { kind, items } => {
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                let marker = match kind {
                    crate::ListKind::Ordered { start } => {
                        format!("{}. ", u64::from(*start) + index as u64)
                    }
                    crate::ListKind::Bullet => "- ".to_owned(),
                };
                out.push_str(&marker);
                let indent = " ".repeat(marker.len());
                match item.task {
                    None => {}
                    Some(crate::Task::Open) => out.push_str("[ ] "),
                    Some(crate::Task::Done) => out.push_str("[x] "),
                }
                let mut inner = String::new();
                write_item(&mut inner, &item.blocks, &indent);
                out.push_str(inner.trim_end_matches('\n'));
            }
        }
        Block::Quote(blocks) => {
            let mut inner = String::new();
            write_blocks(&mut inner, blocks, "");
            let quoted: Vec<String> = inner
                .trim_end_matches('\n')
                .split('\n')
                .map(|line| {
                    if line.is_empty() {
                        ">".to_owned()
                    } else {
                        format!("> {line}")
                    }
                })
                .collect();
            out.push_str(&quoted.join("\n"));
        }
        Block::Code {
            text: code,
            language,
        } => {
            // A fence longer than any backtick run inside, so the code
            // cannot close it.
            let longest = code.split(|c| c != '`').map(str::len).max().unwrap_or(0);
            let fence = "`".repeat(longest.max(2) + 1);
            out.push_str(&fence);
            if let Some(language) = language {
                out.push_str(language.as_str());
            }
            out.push('\n');
            out.push_str(code);
            if !code.is_empty() && !code.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&fence);
        }
        Block::Rule => out.push_str("---"),
        Block::Table(table) => write_table(out, table),
        Block::Quoted(_) => return false,
    }
    true
}

/// A GFM pipe table: the header, the delimiter row, the body rows.
fn write_table(out: &mut String, table: &crate::Table) {
    use crate::Alignment;
    let row = |out: &mut String, cells: &[crate::Cell]| {
        out.push('|');
        for cell in cells {
            out.push(' ');
            write_inlines(out, cell, Place::Line);
            out.push_str(" |");
        }
    };
    row(out, table.header());
    out.push_str("\n|");
    for alignment in table.alignments() {
        out.push_str(match alignment {
            Alignment::None => "---|",
            Alignment::Left => ":--|",
            Alignment::Center => ":-:|",
            Alignment::Right => "--:|",
        });
    }
    for cells in table.rows() {
        out.push('\n');
        row(out, cells);
    }
}

/// A list item: its first block after the marker, the rest indented under
/// it, a blank line between them so a second paragraph is not read as the
/// first one's next line.
fn write_item(out: &mut String, blocks: &[crate::Block], indent: &str) {
    let mut first = true;
    for block in blocks {
        let mut text = String::new();
        if !write_block(&mut text, block) {
            continue;
        }
        if !first {
            out.push_str("\n\n");
            out.push_str(indent);
        }
        first = false;
        out.push_str(&text.replace('\n', &format!("\n{indent}")));
    }
}

fn write_inlines(out: &mut String, inlines: &[crate::Inline], place: Place) {
    use crate::Inline;
    let last = inlines.len().saturating_sub(1);
    for (index, inline) in inlines.iter().enumerate() {
        let before = index.checked_sub(1).map(|at| &inlines[at]);
        let after = inlines.get(index + 1);
        match inline {
            Inline::Text(text) => {
                // A space Markdown would trim: at a paragraph's or a line's
                // edge, or beside a break.
                let edge = place != Place::Inner;
                let opens = edge && (index == 0 || matches!(before, Some(Inline::Break)));
                let closes = edge && (index == last || matches!(after, Some(Inline::Break)));
                let line_start = place == Place::Paragraph && opens;
                let mut escaped = escape(text, line_start);
                if opens && escaped.starts_with(' ') {
                    escaped.replace_range(..1, "&#32;");
                }
                if closes && escaped.ends_with(' ') {
                    escaped.truncate(escaped.len() - 1);
                    escaped.push_str("&#32;");
                }
                out.push_str(&escaped);
            }
            Inline::Strong(inner) => write_mark(out, inner, "**", "strong"),
            Inline::Emphasis(inner) => write_mark(out, inner, "*", "em"),
            Inline::Strikethrough(inner) => write_mark(out, inner, "~~", "del"),
            Inline::Code(code) => {
                if code.is_empty() {
                    out.push_str("<code></code>");
                    continue;
                }
                // A fence longer than any run of backticks inside, padded
                // where CommonMark would otherwise strip or merge.
                let longest = code.split(|c| c != '`').map(str::len).max().unwrap_or(0);
                let fence = "`".repeat(longest + 1);
                let spaced = code.starts_with(' ') && code.ends_with(' ') && code.trim() != "";
                let pad = if code.starts_with('`') || code.ends_with('`') || spaced {
                    " "
                } else {
                    ""
                };
                out.push_str(&format!("{fence}{pad}{code}{pad}{fence}"));
            }
            Inline::Link { href, inlines } => {
                out.push('[');
                write_inlines(out, inlines, Place::Inner);
                out.push_str(&format!("]({})", destination(href.as_str())));
            }
            Inline::Image { content_id, alt } => {
                out.push_str(&format!(
                    "![{}](cid:{})",
                    escape(alt, false),
                    content_id.as_str()
                ));
            }
            Inline::RemoteImage { href, alt } => {
                out.push_str(&format!(
                    "![{}]({})",
                    escape(alt, false),
                    destination(href.as_str())
                ));
            }
            Inline::Break => {
                // `\` at a line's end, where there is a line on both sides
                // and words before; `<br>` everywhere else.
                let backslash = place == Place::Paragraph
                    && index > 0
                    && index < last
                    && !matches!(before, Some(Inline::Break))
                    && !matches!(after, Some(Inline::Break));
                out.push_str(if backslash { "\\\n" } else { "<br>" });
            }
        }
    }
}

/// A mark as its Markdown delimiters where CommonMark will read them back
/// -- its words start and end with a letter or a digit -- and as HTML
/// where it would not: `**x!**y` is not bold, `<strong>x!</strong>y` is.
fn write_mark(out: &mut String, inner: &[crate::Inline], delimiter: &str, tag: &str) {
    let mut text = String::new();
    write_inlines(&mut text, inner, Place::Inner);
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    if word(text.chars().next()) && word(text.chars().next_back()) {
        out.push_str(delimiter);
        out.push_str(&text);
        out.push_str(delimiter);
    } else {
        out.push_str(&format!("<{tag}>{text}</{tag}>"));
    }
}

/// A link destination, in angle brackets where a space would end it.
fn destination(href: &str) -> String {
    if href.contains([' ', '<', '>']) {
        format!("<{}>", href.replace('<', "%3C").replace('>', "%3E"))
    } else {
        href.to_owned()
    }
}

/// Text, with everything Markdown would read as markup escaped.
///
/// Including what GFM would read as a bare autolink: a backslash before its
/// first `.`, `:` or `@` keeps it the text it was, because `to_document`
/// links only an address typed as it reads. `line_start` is whether the text
/// begins a line, where a `#`, `-`, `+`, `=` or a number would start a block.
fn escape(text: &str, line_start: bool) -> String {
    let breaks: Vec<usize> = crate::autolink::find(text)
        .iter()
        .filter_map(|link| {
            text[link.range.clone()]
                .find(['.', ':', '@'])
                .map(|at| link.range.start + at)
        })
        .collect();
    let digits = text.bytes().take_while(u8::is_ascii_digit).count();
    let mut out = String::with_capacity(text.len());
    for (index, c) in text.char_indices() {
        let special = breaks.contains(&index)
            || matches!(
                c,
                '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '!' | '|' | '~' | '&' | '#'
            )
            || (line_start && index == 0 && matches!(c, '-' | '+' | '='))
            || (line_start && digits > 0 && index == digits && matches!(c, '.' | ')'));
        if special {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// What holds a run of inlines.
enum Holder {
    Paragraph,
    Heading(crate::HeadingLevel),
    Emphasis,
    Strong,
    Strikethrough,
    /// One table cell.
    Cell,
    /// A mark written as HTML, because its delimiters would not read back.
    Html(Mark),
    /// A link, when its target is one the document can carry.
    Link(Option<crate::Href>),
    /// Something the document has no node for: its words pass through.
    Transparent,
}

/// The marks `from_document` may write as HTML.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    Strong,
    Emphasis,
    Strikethrough,
    Code,
}

impl Mark {
    fn closer(self) -> &'static str {
        match self {
            Mark::Strong => "</strong>",
            Mark::Emphasis => "</em>",
            Mark::Strikethrough => "</del>",
            Mark::Code => "</code>",
        }
    }
}

/// One open container while the Markdown is read.
enum Frame {
    Blocks(Vec<crate::Block>),
    Quote(Vec<crate::Block>),
    List(crate::ListKind, Vec<crate::ListItem>),
    /// A list item: its box, its blocks, and the words of a tight item not
    /// yet wrapped in a paragraph.
    Item(Option<crate::Task>, Vec<crate::Block>, Vec<crate::Inline>),
    Inlines(Holder, Vec<crate::Inline>),
    Code(Option<crate::Language>, String),
    Image(String, String),
    /// A table: its alignments, and its rows so far, the header first.
    Table(Vec<crate::Alignment>, Vec<Vec<crate::Cell>>),
    /// A table row's cells so far.
    Row(Vec<crate::Cell>),
}

fn push_inline(stack: &mut [Frame], inline: crate::Inline) {
    match stack.last_mut() {
        Some(Frame::Inlines(_, inlines) | Frame::Item(_, _, inlines)) => {
            // One run of words is one text, however the parser split it -- at
            // an escape, a soft break -- so a document reads back the same.
            if let (crate::Inline::Text(more), Some(crate::Inline::Text(last))) =
                (&inline, inlines.last_mut())
            {
                last.push_str(more);
            } else {
                inlines.push(inline);
            }
        }
        Some(Frame::Code(_, code)) => {
            if let crate::Inline::Text(text) = inline {
                code.push_str(&text);
            }
        }
        Some(Frame::Image(_, alt)) => {
            if let crate::Inline::Text(text) = inline {
                alt.push_str(&text);
            }
        }
        _ => push_block(stack, crate::Block::Paragraph(vec![inline])),
    }
}

fn push_block(stack: &mut [Frame], block: crate::Block) {
    match stack.last_mut() {
        Some(Frame::Blocks(blocks) | Frame::Quote(blocks)) => blocks.push(block),
        Some(Frame::Item(_, blocks, loose)) => {
            blocks.extend(paragraph_of(std::mem::take(loose)));
            blocks.push(block);
        }
        Some(Frame::List(_, items)) => items.push(crate::ListItem::new(vec![block])),
        // A block inside an inline container: close it into the words.
        Some(
            Frame::Inlines(..)
            | Frame::Code(..)
            | Frame::Image(..)
            | Frame::Table(..)
            | Frame::Row(_),
        )
        | None => {
            if let Some(Frame::Blocks(blocks)) = stack.first_mut() {
                blocks.push(block);
            }
        }
    }
}

/// How many cells a pipe-table row was typed with: its `|`s, less the
/// escaped ones and the optional outer pair.
fn typed_cells(row: &str) -> usize {
    let row = row.trim();
    let inner = row.strip_prefix('|').unwrap_or(row);
    let inner = inner
        .strip_suffix('|')
        .filter(|rest| !rest.ends_with('\\'))
        .unwrap_or(inner);
    let mut cells = 1;
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '|' => cells += 1,
            _ => {}
        }
    }
    cells
}

/// A paragraph's inlines as the paragraph they spell, or none for none.
///
/// `<br><br>` alone is a paragraph of one break: a paragraph of nothing but
/// breaks is written with one more, since an empty one is `<br>` (spec 014).
fn paragraph_of(mut inlines: Vec<crate::Inline>) -> Option<crate::Block> {
    let breaks = !inlines.is_empty()
        && inlines
            .iter()
            .all(|inline| matches!(inline, crate::Inline::Break));
    if breaks {
        inlines.pop();
    }
    (breaks || !inlines.is_empty()).then_some(crate::Block::Paragraph(inlines))
}

fn close(stack: &mut [Frame], frame: Frame, lost: &mut Lost) {
    use crate::{Block, Inline};
    match frame {
        Frame::Inlines(Holder::Paragraph, inlines) => {
            if let Some(paragraph) = paragraph_of(inlines) {
                push_block(stack, paragraph);
            }
        }
        Frame::Inlines(Holder::Html(mark), inlines) => push_inline(
            stack,
            match mark {
                Mark::Strong => Inline::Strong(inlines),
                Mark::Emphasis => Inline::Emphasis(inlines),
                Mark::Strikethrough => Inline::Strikethrough(inlines),
                Mark::Code => Inline::Code(
                    inlines
                        .into_iter()
                        .map(|inline| match inline {
                            Inline::Text(text) => text,
                            _ => String::new(),
                        })
                        .collect(),
                ),
            },
        ),
        Frame::Inlines(Holder::Heading(level), inlines) => {
            push_block(stack, Block::Heading { level, inlines });
        }
        Frame::Inlines(Holder::Emphasis, inlines) => push_inline(stack, Inline::Emphasis(inlines)),
        Frame::Inlines(Holder::Strong, inlines) => push_inline(stack, Inline::Strong(inlines)),
        Frame::Inlines(Holder::Strikethrough, inlines) => {
            push_inline(stack, Inline::Strikethrough(inlines));
        }
        Frame::Inlines(Holder::Cell, inlines) => {
            if let Some(Frame::Row(cells)) = stack.last_mut() {
                cells.push(inlines);
            }
        }
        Frame::Row(cells) => {
            if let Some(Frame::Table(_, rows)) = stack.last_mut() {
                rows.push(cells);
            }
        }
        Frame::Table(alignments, mut rows) => {
            let header = if rows.is_empty() {
                Vec::new()
            } else {
                rows.remove(0)
            };
            let (table, dropped) = crate::Table::counting(alignments, header, rows);
            lost.cells += dropped;
            push_block(stack, Block::Table(table));
        }
        Frame::Inlines(Holder::Link(Some(href)), inlines) => {
            push_inline(stack, Inline::Link { href, inlines });
        }
        Frame::Inlines(holder @ (Holder::Link(None) | Holder::Transparent), inlines) => {
            if matches!(holder, Holder::Link(None)) {
                lost.links += 1;
            }
            for inline in inlines {
                push_inline(stack, inline);
            }
        }
        Frame::Image(dest, alt) => {
            let attached = dest.strip_prefix("cid:").and_then(crate::ContentId::parse);
            match (attached, crate::WebHref::parse(&dest)) {
                (Some(content_id), _) => push_inline(stack, Inline::Image { content_id, alt }),
                // Never fetched here: a chip the person may press to
                // download and attach it, and a link if they never do
                // (spec 014 D7).
                (None, Some(href)) => push_inline(stack, Inline::RemoteImage { href, alt }),
                (None, None) => {
                    lost.images += 1;
                    push_inline(stack, Inline::Text(format!("![{alt}]({dest})")));
                }
            }
        }
        Frame::Code(language, text) => push_block(stack, Block::Code { language, text }),
        Frame::Quote(blocks) => push_block(stack, Block::Quote(blocks)),
        Frame::List(kind, items) => push_block(stack, Block::List { kind, items }),
        Frame::Item(task, mut blocks, loose) => {
            blocks.extend(paragraph_of(loose));
            if let Some(Frame::List(_, items)) = stack.last_mut() {
                items.push(crate::ListItem { task, blocks });
            } else {
                for block in blocks {
                    push_block(stack, block);
                }
            }
        }
        Frame::Blocks(blocks) => {
            for block in blocks {
                push_block(stack, block);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RemoteImages, fold_html_quotes, sanitize_body};

    fn read(html: &str) -> String {
        from_html(&fold_html_quotes(
            &sanitize_body(html, RemoteImages::Blocked).html,
        ))
    }

    #[test]
    fn structure_survives() {
        let markdown = read(
            "<h2>Plan</h2><p>Some <b>bold</b> and <a href=\"https://example.com/x\">a link</a>.</p><ul><li>one</li><li>two</li></ul>",
        );
        assert!(markdown.contains("Plan"), "{markdown}");
        assert!(markdown.contains("**bold**"), "{markdown}");
        assert!(
            markdown.contains("[a link](https://example.com/x)"),
            "{markdown}"
        );
        assert!(
            markdown.contains("one") && markdown.contains("two"),
            "{markdown}"
        );
    }

    #[test]
    fn a_blocked_remote_image_leaves_a_placeholder_not_nothing() {
        let markdown = read(
            "<p>Before</p><img src=\"https://tracker.example.org/p.gif\" alt=\"Logo\"><p>After</p>",
        );
        assert!(markdown.contains("![Logo](postio-image:"), "{markdown}");
        assert!(!markdown.contains("tracker.example.org"), "{markdown}");
    }

    #[test]
    fn an_image_with_no_alt_is_still_named() {
        let markdown = read("<img src=\"https://example.org/a.png\">");
        assert!(markdown.contains("![image](postio-image:"), "{markdown}");
    }

    #[test]
    fn a_folded_quote_is_marked_so_it_can_expand() {
        let markdown = read("<p>Thanks!</p><blockquote><p>Earlier words</p></blockquote>");
        let fold = markdown
            .find(FOLD)
            .unwrap_or_else(|| panic!("no fold: {markdown:?}"));
        let end = markdown
            .find(END_FOLD)
            .unwrap_or_else(|| panic!("no end: {markdown:?}"));
        let inside = &markdown[fold..end];
        assert!(inside.contains("Earlier words"), "{markdown:?}");
        assert!(markdown[..fold].contains("Thanks!"), "{markdown:?}");
    }

    #[test]
    fn a_table_is_still_a_table() {
        let markdown = read(
            "<table><tr><th>Item</th><th>Qty</th></tr><tr><td>Widget</td><td>3</td></tr></table>",
        );
        assert!(
            markdown.contains("| Item") && markdown.contains("| Widget"),
            "{markdown}"
        );
    }

    mod writing {
        use super::super::to_document;
        use crate::{Block, HeadingLevel, Inline, render};

        fn text(s: &str) -> Inline {
            Inline::Text(s.to_owned())
        }

        #[test]
        fn words_alone_are_plain_text() {
            let document = to_document("Hello Ada,\n\nThanks for the notes.");
            assert!(document.is_plain_text(), "{document:?}");
            assert_eq!(document.blocks.len(), 2);
        }

        #[test]
        fn emphasis_code_and_a_hard_break() {
            let document = to_document("Some **bold**, *italic* and `code`  \nnext line");
            let Block::Paragraph(inlines) = &document.blocks[0] else {
                panic!("{document:?}")
            };
            assert!(
                inlines.contains(&Inline::Strong(vec![text("bold")])),
                "{inlines:?}"
            );
            assert!(
                inlines.contains(&Inline::Emphasis(vec![text("italic")])),
                "{inlines:?}"
            );
            assert!(
                inlines.contains(&Inline::Code("code".into())),
                "{inlines:?}"
            );
            assert!(inlines.contains(&Inline::Break), "{inlines:?}");
            assert!(!document.is_plain_text());
        }

        #[test]
        fn six_heading_levels_and_none_narrows() {
            let document = to_document("# One\n\n#### Four");
            assert!(matches!(
                &document.blocks[0],
                Block::Heading {
                    level: HeadingLevel::One,
                    ..
                }
            ));
            assert!(matches!(
                &document.blocks[1],
                Block::Heading {
                    level: HeadingLevel::Four,
                    ..
                }
            ));
        }

        #[test]
        fn lists_nest_and_quotes_and_code_and_rules_map() {
            let document = to_document(
                "- one\n  - inner\n- two\n\n> quoted\n\n```rust\nfn x() {}\n```\n\n---",
            );
            let Block::List {
                kind: crate::ListKind::Bullet,
                items,
            } = &document.blocks[0]
            else {
                panic!("{document:?}")
            };
            assert_eq!(items.len(), 2);
            assert!(
                items[0]
                    .blocks
                    .iter()
                    .any(|block| matches!(block, Block::List { .. })),
                "{items:?}"
            );
            assert!(
                matches!(&document.blocks[1], Block::Quote(_)),
                "{document:?}"
            );
            assert_eq!(
                document.blocks[2],
                Block::Code {
                    language: crate::Language::parse("rust"),
                    text: "fn x() {}\n".into()
                }
            );
            assert_eq!(document.blocks[3], Block::Rule);
        }

        #[test]
        fn an_ordered_list_is_ordered() {
            let document = to_document("1. first\n2. second");
            assert!(
                matches!(&document.blocks[0], Block::List { kind: crate::ListKind::Ordered { start: 1 }, items } if items.len() == 2)
            );
        }

        #[test]
        fn a_web_link_is_a_link_and_a_script_link_is_only_words() {
            let document =
                to_document("[site](https://example.com) and [trap](javascript:alert(1))");
            let Block::Paragraph(inlines) = &document.blocks[0] else {
                panic!("{document:?}")
            };
            assert!(inlines.iter().any(|inline| matches!(inline, Inline::Link { href, .. } if href.as_str() == "https://example.com")), "{inlines:?}");
            let html = render(&document).1;
            assert!(!html.contains("javascript:"), "{html}");
            assert!(html.contains("trap"), "{html}");
        }

        #[test]
        fn an_attached_image_is_an_image_and_a_remote_one_is_never_fetched() {
            let document = to_document(
                "![chart](cid:chart.1@postio.invalid) ![pixel](https://tracker.example.org/p.gif)",
            );
            let Block::Paragraph(inlines) = &document.blocks[0] else {
                panic!("{document:?}")
            };
            assert!(inlines.iter().any(|inline| matches!(inline, Inline::Image { content_id, .. } if content_id.as_str() == "chart.1@postio.invalid")), "{inlines:?}");
            let html = render(&document).1;
            assert!(!html.contains("<img src=\"https"), "{html}");
            assert!(
                html.contains("tracker.example.org/p.gif"),
                "kept as the words written: {html}"
            );
        }

        #[test]
        fn a_table_is_a_table() {
            let document = to_document("| a | b |\n|:--|--:|\n| 1 | 2 |\n| 3 |");
            assert_eq!(
                document.blocks,
                vec![Block::Table(crate::Table::new(
                    vec![crate::Alignment::Left, crate::Alignment::Right],
                    vec![vec![text("a")], vec![text("b")]],
                    vec![
                        vec![vec![text("1")], vec![text("2")]],
                        vec![vec![text("3")], vec![]],
                    ],
                ))]
            );
        }

        #[test]
        fn strikethrough_tasks_and_a_list_start() {
            let document = to_document("~~gone~~\n\n- [ ] open\n- [x] done\n\n7. seventh");
            assert_eq!(
                document.blocks[0],
                Block::Paragraph(vec![Inline::Strikethrough(vec![text("gone")])])
            );
            let Block::List { kind, items } = &document.blocks[1] else {
                panic!("{document:?}")
            };
            assert_eq!(*kind, crate::ListKind::Bullet);
            assert_eq!(items[0].task, Some(crate::Task::Open));
            assert_eq!(items[1].task, Some(crate::Task::Done));
            assert_eq!(items[0].blocks, vec![Block::Paragraph(vec![text("open")])]);
            assert!(matches!(
                &document.blocks[2],
                Block::List {
                    kind: crate::ListKind::Ordered { start: 7 },
                    ..
                }
            ));
        }

        #[test]
        fn a_fence_keeps_its_first_word_as_the_language() {
            let code = |markdown: &str| match &to_document(markdown).blocks[0] {
                Block::Code { language, .. } => language.as_ref().map(|l| l.as_str().to_owned()),
                other => panic!("{other:?}"),
            };
            assert_eq!(code("```rust\nx\n```").as_deref(), Some("rust"));
            assert_eq!(code("~~~c++ extra words\nx\n~~~").as_deref(), Some("c++"));
            assert_eq!(code("```a\"b\nx\n```"), None);
            assert_eq!(code("```\nx\n```"), None);
            assert_eq!(code("    indented\n"), None);
        }

        #[test]
        fn a_web_image_is_a_remote_image_and_an_attached_one_an_image() {
            let document = to_document("![x](https://example.com/x.png) ![y](cid:y@example.com)");
            let Block::Paragraph(inlines) = &document.blocks[0] else {
                panic!("{document:?}")
            };
            assert!(matches!(&inlines[0], Inline::RemoteImage { href, alt }
                if href.as_str() == "https://example.com/x.png" && alt == "x"));
            assert!(matches!(&inlines[2], Inline::Image { .. }));
            // An address the web does not serve stays the words written.
            let other = to_document("![z](ftp://example.com/z.png)");
            assert_eq!(
                other.blocks[0],
                Block::Paragraph(vec![text("![z](ftp://example.com/z.png)")])
            );
        }

        #[test]
        fn a_bare_address_is_a_link_but_not_inside_code() {
            let document = to_document("see www.example.com and `www.example.com`");
            let Block::Paragraph(inlines) = &document.blocks[0] else {
                panic!("{document:?}")
            };
            assert!(
                matches!(&inlines[1], Inline::Link { href, .. }
                if href.as_str() == "http://www.example.com"),
                "{inlines:?}"
            );
            assert_eq!(
                inlines.last(),
                Some(&Inline::Code("www.example.com".into()))
            );
        }

        #[test]
        fn reference_links_and_entities_read_as_what_they_mean() {
            let document = to_document("[the plan][p] &amp; more\n\n[p]: https://example.com/plan");
            assert_eq!(document.blocks.len(), 1, "{document:?}");
            let Block::Paragraph(inlines) = &document.blocks[0] else {
                panic!("{document:?}")
            };
            assert!(matches!(&inlines[0], Inline::Link { href, .. }
                if href.as_str() == "https://example.com/plan"));
            assert_eq!(inlines[1], text(" & more"));
        }

        #[test]
        fn any_input_is_some_document() {
            for input in [
                "",
                "*",
                "**unclosed",
                "[",
                "![",
                "> > >",
                "- \n-",
                "```",
                "<script>x</script>",
                "\u{0}",
            ] {
                let _ = to_document(input);
            }
        }
    }

    mod reopening {
        use super::super::{from_document, to_document};

        const SAMPLES: &[&str] = &[
            "Hello Ada,\n\nThanks for the notes.",
            "Some **bold**, *italic* and `code`\\\nnext line",
            "# One\n\n## Two\n\n### Three",
            "- one\n  - inner\n- two",
            "1. first\n2. second",
            "> quoted\n>\n> - with a list",
            "```\nfn x() {}\n```",
            "---",
            "[site](https://example.com) and ![chart](cid:chart.1@postio.invalid)",
            "Stars * and _underscores_ and a # and 1. not a list",
            "a `tick`` pair and \\*escaped\\*",
            "**bold *and italic* inside**",
            "| a | b |\n|---|---|",
            "~~struck~~ and ~a tilde",
            "- [ ] open\n- [x] done",
            "7. seventh\n8. eighth",
            "```rust\nfn x() {}\n```",
            "| a | b |\n|:--|--:|\n| 1 | 2 |",
            "![x](https://example.com/x.png)",
            "see www.example.com and ada@example.com",
            "#### four\n\n###### six",
        ];

        #[test]
        fn a_document_written_as_markdown_reads_back_the_same() {
            for sample in SAMPLES {
                let document = to_document(sample);
                let written = from_document(&document);
                assert_eq!(
                    to_document(&written),
                    document,
                    "{sample:?} wrote {written:?}"
                );
            }
        }

        #[test]
        fn the_gfm_forms_are_what_it_writes() {
            for (markdown, written) in [
                ("~~x~~", "~~x~~"),
                ("- [ ] a\n- [x] b", "- [ ] a\n- [x] b"),
                ("7. a\n8. b", "7. a\n8. b"),
                ("```rust\nx\n```", "```rust\nx\n```"),
                (
                    "| a | b |\n|:--|--:|\n| 1 | 2 |",
                    "| a | b |\n|:--|--:|\n| 1 | 2 |",
                ),
                (
                    "![x](https://example.com/x.png)",
                    "![x](https://example.com/x.png)",
                ),
            ] {
                assert_eq!(from_document(&to_document(markdown)).trim_end(), written);
            }
        }

        #[test]
        fn text_that_looks_like_an_address_is_written_so_it_stays_text() {
            let document = crate::Document {
                blocks: vec![crate::Block::Paragraph(vec![crate::Inline::Text(
                    "www.example.com and ada@example.com".into(),
                )])],
            };
            assert_eq!(to_document(&from_document(&document)), document);
        }

        #[test]
        fn markdown_it_writes_is_what_a_person_would_type() {
            let written = from_document(&to_document(
                "Some **bold** and a [link](https://example.com).",
            ));
            assert_eq!(
                written.trim_end(),
                "Some **bold** and a [link](https://example.com)."
            );
        }
    }

    mod source_map {
        //! spec 014 `contracts/source-view.md`, "The caret".
        use super::super::{DocPosition, from_document_mapped, to_document_mapped};

        fn at(path: &[usize], offset: usize) -> DocPosition {
            DocPosition {
                path: path.to_vec(),
                offset,
            }
        }

        /// Every visible character of `markdown`'s document, by position.
        fn positions(markdown: &str) -> Vec<DocPosition> {
            let (document, _) = to_document_mapped(markdown);
            super::super::visible(&document)
                .into_iter()
                .map(|(position, _)| position)
                .collect()
        }

        const EVERY_CONSTRUCT: &str = "# Plan\n\nSome **bold**, ~~struck~~ and `code` and [a link](https://example.com).\n\n- [ ] open\n- two\n  1. nested\n\n> quoted\n\n```rust\nfn x() {}\n```\n\n| a | b |\n|---|--:|\n| c | d |\n\nline one\\\nline two\n";

        #[test]
        fn every_character_maps_to_the_source_and_back_to_itself() {
            let (_, map) = to_document_mapped(EVERY_CONSTRUCT);
            let all = positions(EVERY_CONSTRUCT);
            assert!(all.len() > 40, "{all:?}");
            for position in all {
                let source = map.to_source(&position);
                assert_eq!(map.to_document(source), position, "via {source}");
            }
        }

        #[test]
        fn a_character_maps_to_where_it_was_typed() {
            let (_, map) = to_document_mapped(EVERY_CONSTRUCT);
            let source = map.to_source(&at(&[1], 5));
            assert_eq!(&EVERY_CONSTRUCT[source..source + 4], "bold");
            let struck = map.to_source(&at(&[1], 11));
            assert_eq!(&EVERY_CONSTRUCT[struck..struck + 6], "struck");
        }

        #[test]
        fn an_offset_inside_a_marker_is_the_first_character_it_formats() {
            let (_, map) = to_document_mapped(EVERY_CONSTRUCT);
            let stars = EVERY_CONSTRUCT.find("**bold").unwrap();
            assert_eq!(map.to_document(stars), at(&[1], 5));
            assert_eq!(map.to_document(stars + 1), at(&[1], 5));
            assert_eq!(map.to_document(0), at(&[0], 0), "`# `");
            let task = EVERY_CONSTRUCT.find("- [ ]").unwrap();
            assert_eq!(map.to_document(task + 3), at(&[2, 0, 0], 0), "`- [ ] `");
            let fence = EVERY_CONSTRUCT.find("```rust").unwrap();
            assert_eq!(map.to_document(fence + 2), at(&[4], 0), "a fence line");
            let delimiter = EVERY_CONSTRUCT.find("|---").unwrap();
            assert_eq!(
                map.to_document(delimiter + 1),
                at(&[5, 2], 0),
                "a delimiter row"
            );
        }

        #[test]
        fn narrowed_text_maps_to_the_nearest_position_before_it() {
            // A third cell in a two-column table is cut, so its words have
            // no position: the caret goes to the end of what survived.
            let ragged = "| a | b |\n|---|---|\n| c | d | cut |\n";
            let (_, map) = to_document_mapped(ragged);
            let cut = ragged.find("cut").unwrap();
            assert_eq!(map.to_document(cut + 1), at(&[0, 3], 1));
        }

        #[test]
        fn the_end_of_the_document_is_a_position() {
            let (_, map) = to_document_mapped("one\n\ntwo");
            assert_eq!(map.to_document(9), at(&[1], 3));
            assert_eq!(map.to_source(&at(&[1], 3)), 8);
        }

        #[test]
        fn the_written_markdown_maps_as_the_read_markdown_does() {
            let document = super::super::to_document(EVERY_CONSTRUCT);
            let (written, map) = from_document_mapped(&document);
            let bold = map.to_source(&at(&[1], 5));
            assert_eq!(&written[bold..bold + 4], "bold");
        }
    }

    mod caret {
        use super::super::{CARET, DocPosition, put_caret, take_caret, to_document};

        #[test]
        fn a_caret_put_anywhere_is_taken_back_from_the_same_place() {
            let document = to_document(
                "# Plan\n\nSome **bold** and `code`\\\nnext\n\n- [ ] task\n\n| a | b |\n|---|---|\n| c | d |\n\n```\nfn x() {}\n```",
            );
            for (position, _) in super::super::visible(&document) {
                let marked = put_caret(&document, &position);
                assert_ne!(marked, document, "{position:?} was not marked");
                let (clean, found) = take_caret(marked);
                assert_eq!(found, Some(position.clone()));
                assert_eq!(clean, document, "the caret left a trace at {position:?}");
            }
        }

        #[test]
        fn the_end_of_a_block_is_a_place_too() {
            let document = to_document("one\n\ntwo");
            let end = DocPosition {
                path: vec![1],
                offset: 3,
            };
            let (clean, found) = take_caret(put_caret(&document, &end));
            assert_eq!(found, Some(end));
            assert_eq!(clean, document);
        }

        #[test]
        fn a_document_without_a_caret_has_none() {
            let document = to_document("words");
            assert_eq!(take_caret(document.clone()), (document, None));
            assert!(!"words".contains(CARET));
        }
    }

    #[test]
    fn what_typed_markdown_narrowed_is_counted() {
        let (_, map) = super::to_document_mapped(
            "| a |\n|---|\n| b | c | d |\n\n[x](javascript:alert(1)) ![y](ftp://example.com/y.png)",
        );
        assert_eq!(
            map.lost(),
            super::Lost {
                cells: 2,
                links: 1,
                images: 1
            }
        );
        assert!(super::to_document_mapped("plain").1.lost().is_empty());
    }

    #[test]
    fn a_caret_after_a_remote_images_words_stays_after_them() {
        let source = "The chart: ![weekly chart](https://example.com/c.png)";
        let (document, map) = super::to_document_mapped(source);
        let end = super::DocPosition {
            path: vec![0],
            offset: "The chart: weekly chart".chars().count(),
        };
        assert_eq!(map.to_document(source.len()), end);
        let (clean, found) = super::take_caret(super::put_caret(&document, &end));
        assert_eq!(found, Some(end));
        assert_eq!(clean, document);
    }
}
