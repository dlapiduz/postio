//! The composer's document: a closed type, and the HTML subset it maps to (ADR 0004).
//! Outgoing HTML is generated from this type, never passed through, so no remote image, script URL
//! or style has a representation.
//! `parse` is total and narrows; `parse(to_html(d)) == d`, and `to_html(parse(h)) == h` for `h`
//! already in the subset.

use std::fmt::Write as _;

use crate::flowed;

/// Only [`Href::parse`] builds one, so no serialiser has to check for `javascript:`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Href(String);

impl Href {
    const SCHEMES: [&'static str; 3] = ["http", "https", "mailto"];

    /// Refuses any ASCII control character anywhere: `java&#9;script:` entity-decodes to
    /// `javascript:`, so a scheme check made before the decode would miss it.
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.chars().any(|c| c.is_ascii_control()) {
            return None;
        }
        let (scheme, rest) = trimmed.split_once(':')?;
        if rest.is_empty() {
            return None;
        }
        // No base document to resolve a relative reference against once sent.
        Self::SCHEMES
            .iter()
            .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
            .then(|| Href(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Never a URL, so "Postio cannot send a tracking pixel" is a fact about the type.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentId(String);

impl ContentId {
    /// Refuses anything readable as a URL (colon, whitespace, control character):
    /// a `cid:` body of `//evil.example` must not become an image source.
    pub fn parse(raw: &str) -> Option<Self> {
        let id = raw
            .trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .trim();
        if id.is_empty()
            || id
                .chars()
                .any(|c| c == ':' || c == '/' || c.is_whitespace() || c.is_ascii_control())
        {
            return None;
        }
        Some(ContentId(id.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HeadingLevel {
    One,
    Two,
    Three,
    Four,
    Five,
    Six,
}

impl HeadingLevel {
    pub const ALL: [HeadingLevel; 6] = [
        HeadingLevel::One,
        HeadingLevel::Two,
        HeadingLevel::Three,
        HeadingLevel::Four,
        HeadingLevel::Five,
        HeadingLevel::Six,
    ];

    pub fn digit(self) -> u8 {
        match self {
            HeadingLevel::One => 1,
            HeadingLevel::Two => 2,
            HeadingLevel::Three => 3,
            HeadingLevel::Four => 4,
            HeadingLevel::Five => 5,
            HeadingLevel::Six => 6,
        }
    }

    pub fn from_digit(digit: u8) -> Option<Self> {
        Self::ALL.get(usize::from(digit).checked_sub(1)?).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ListKind {
    Bullet,
    /// `start` may be `0`, as in HTML and CommonMark.
    Ordered {
        start: u32,
    },
}

impl ListKind {
    pub const NUMBERED: ListKind = ListKind::Ordered { start: 1 };

    pub fn is_ordered(self) -> bool {
        matches!(self, ListKind::Ordered { .. })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Task {
    Open,
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub task: Option<Task>,
    pub blocks: Vec<Block>,
}

impl ListItem {
    pub fn new(blocks: Vec<Block>) -> Self {
        ListItem { task: None, blocks }
    }

    pub fn task(task: Task, blocks: Vec<Block>) -> Self {
        ListItem {
            task: Some(task),
            blocks,
        }
    }
}

/// Built only by [`Language::parse`] (1 to 32 of ASCII letter, digit or `+ # . _ -`),
/// so `data-postio-lang` can never carry a quote.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Language(String);

impl Language {
    pub const MAX: usize = 32;

    /// No trimming: a space is refused, so a caller holding an info string picks its first word.
    pub fn parse(raw: &str) -> Option<Self> {
        let allowed =
            |c: char| c.is_ascii_alphanumeric() || matches!(c, '+' | '#' | '.' | '_' | '-');
        (!raw.is_empty() && raw.chars().count() <= Self::MAX && raw.chars().all(allowed))
            .then(|| Language(raw.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Alignment {
    #[default]
    None,
    Left,
    Center,
    Right,
}

pub type Cell = Vec<Inline>;

/// Rectangular: only [`Table::new`] builds one, padding short rows and truncating long ones as GFM
/// does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    alignments: Vec<Alignment>,
    header: Vec<Cell>,
    rows: Vec<Vec<Cell>>,
}

impl Table {
    /// As wide as `alignments`, else as `header`; never under one column.
    pub fn new(alignments: Vec<Alignment>, header: Vec<Cell>, rows: Vec<Vec<Cell>>) -> Self {
        Self::counting(alignments, header, rows).0
    }

    /// As [`Table::new`], also reporting the cells truncated: what a narrowing counts as lost.
    pub fn counting(
        mut alignments: Vec<Alignment>,
        header: Vec<Cell>,
        rows: Vec<Vec<Cell>>,
    ) -> (Self, usize) {
        if alignments.is_empty() {
            alignments = vec![Alignment::None; header.len().max(1)];
        }
        let width = alignments.len();
        let mut dropped = 0;
        let mut fit = |mut row: Vec<Cell>| {
            dropped += row.len().saturating_sub(width);
            row.resize_with(width, Vec::new);
            row
        };
        let header = fit(header);
        let rows = rows.into_iter().map(&mut fit).collect();
        (
            Table {
                alignments,
                header,
                rows,
            },
            dropped,
        )
    }

    pub fn alignments(&self) -> &[Alignment] {
        &self.alignments
    }

    pub fn header(&self) -> &[Cell] {
        &self.header
    }

    pub fn rows(&self) -> &[Vec<Cell>] {
        &self.rows
    }

    pub fn width(&self) -> usize {
        self.alignments.len()
    }

    pub fn cells(&self) -> impl Iterator<Item = &Cell> {
        self.header.iter().chain(self.rows.iter().flatten())
    }
}

/// An `http` or `https` [`Href`]. Holding one fetches nothing: it is written as a link,
/// and only "Download and attach" reads it as an address.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WebHref(Href);

impl WebHref {
    pub fn parse(raw: &str) -> Option<Self> {
        let href = Href::parse(raw)?;
        let scheme = href.as_str().split(':').next().unwrap_or_default();
        (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
            .then_some(WebHref(href))
    }

    pub fn as_href(&self) -> &Href {
        &self.0
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Code(String),
    Link {
        href: Href,
        inlines: Vec<Inline>,
    },
    Image {
        content_id: ContentId,
        alt: String,
    },
    /// Never an `<img>`: written as a link so a recipient's client fetches nothing. Only Markdown
    /// makes one.
    RemoteImage {
        href: WebHref,
        alt: String,
    },
    Break,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        level: HeadingLevel,
        inlines: Vec<Inline>,
    },
    List {
        kind: ListKind,
        items: Vec<ListItem>,
    },
    Quote(Vec<Block>),
    /// The quote a reply opens with, kept as the reader rendered it, not narrowed to the authoring
    /// subset (ADR 0033).
    /// Only [`crate::quote_of`] builds a [`crate::Quoted`], and the parser rebuilds it through the
    /// same constructor.
    Quoted(crate::Quoted),
    Code {
        language: Option<Language>,
        text: String,
    },
    Rule,
    Table(Table),
}

/// Marks a `<blockquote>` as a reply quote; the parser hands it back to `quote_of` instead of
/// narrowing it.
pub const QUOTED_MARKER: &str = "data-postio-quoted";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Block>,
}

impl Document {
    pub fn new() -> Self {
        Document::default()
    }

    /// Lossless with [`Document::to_text`] for text without carriage returns. Empty paragraphs and
    /// trailing newlines are kept: a reply opens with blank lines for the user to type into.
    pub fn from_text(text: &str) -> Self {
        let blocks = text
            .replace("\r\n", "\n")
            .split("\n\n")
            .map(|para| {
                let mut inlines = Vec::new();
                for (index, line) in para.split('\n').enumerate() {
                    if index > 0 {
                        inlines.push(Inline::Break);
                    }
                    if !line.is_empty() {
                        inlines.push(Inline::Text(line.to_owned()));
                    }
                }
                Block::Paragraph(inlines)
            })
            .collect();
        Document { blocks }
    }

    /// [`from_text`](Self::from_text), with each run of `>` lines a nesting [`Block::Quote`].
    pub fn from_text_quoting(text: &str) -> Self {
        let text = text.replace("\r\n", "\n");
        let mut blocks = Vec::new();
        for para in text.split("\n\n") {
            let lines: Vec<&str> = para.split('\n').collect();
            let mut start = 0;
            while start < lines.len() {
                let quoted = lines[start].starts_with('>');
                let end = lines[start..]
                    .iter()
                    .position(|line| line.starts_with('>') != quoted)
                    .map_or(lines.len(), |n| start + n);
                let run = &lines[start..end];
                if quoted {
                    let inner: Vec<&str> = run
                        .iter()
                        .map(|line| {
                            let line = &line[1..];
                            line.strip_prefix(' ').unwrap_or(line)
                        })
                        .collect();
                    blocks.push(Block::Quote(
                        Self::from_text_quoting(&inner.join("\n")).blocks,
                    ));
                } else {
                    blocks.extend(Self::from_text(&run.join("\n")).blocks);
                }
                start = end;
            }
        }
        Document { blocks }
    }

    /// Inverse of [`to_flowed_text`](Self::to_flowed_text). Only for text that declared
    /// `format=flowed`:
    /// otherwise a real break would be joined onto its neighbour (#456).
    pub fn from_flowed_text(text: &str) -> Self {
        let blocks = text
            .replace("\r\n", "\n")
            .split("\n\n")
            .map(|para| {
                let mut inlines = Vec::new();
                let mut span: Vec<&str> = Vec::new();
                // Not `inlines.is_empty()`: a span that unwrapped to nothing would drop a leading
                // hard break.
                let mut first_span = true;
                for line in para.split('\n') {
                    let fixed = !line.ends_with(' ');
                    span.push(line);
                    if fixed {
                        if !first_span {
                            inlines.push(Inline::Break);
                        }
                        first_span = false;
                        let joined = span.join("\n");
                        let unwrapped = flowed::unwrap(&joined, "");
                        if !unwrapped.is_empty() {
                            inlines.push(Inline::Text(unwrapped));
                        }
                        span.clear();
                    }
                }
                Block::Paragraph(inlines)
            })
            .collect();
        Document { blocks }
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// True for only paragraphs and unstyled text; decides whether a message needs an HTML
    /// alternative at all.
    pub fn is_plain_text(&self) -> bool {
        self.blocks.iter().all(|block| match block {
            Block::Paragraph(inlines) => inlines
                .iter()
                .all(|inline| matches!(inline, Inline::Text(_) | Inline::Break)),
            _ => false,
        })
    }

    /// Hosts only: `mailto:` links and hrefs with no host are skipped.
    pub fn link_hosts(&self) -> Vec<String> {
        let mut hosts = Vec::new();
        collect_link_hosts_in_blocks(&self.blocks, &mut hosts);
        hosts
    }

    pub fn remote_images(&self) -> Vec<(WebHref, String)> {
        let mut found = Vec::new();
        visit_inlines(&self.blocks, &mut |inline| {
            if let Inline::RemoteImage { href, alt } = inline {
                found.push((href.clone(), alt.clone()));
            }
        });
        found
    }

    pub fn attach_remote(&mut self, href: &WebHref, content_id: &ContentId) -> bool {
        replace_remote(&mut self.blocks, href, &mut |alt| Inline::Image {
            content_id: content_id.clone(),
            alt,
        })
    }

    pub fn unlink_remote(&mut self, href: &WebHref) -> bool {
        replace_remote(&mut self.blocks, href, &mut |alt| Inline::Link {
            href: href.as_href().clone(),
            inlines: vec![Inline::Text(if alt.is_empty() {
                href.as_str().to_owned()
            } else {
                alt
            })],
        })
    }

    /// The normal form: no whitespace between blocks, fixed attribute order, escaped text,
    /// so re-saving a draft is a no-op.
    pub fn to_html(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            write_block(&mut out, block, ImageScheme::Wire);
        }
        out
    }

    /// With image sources in the editing shell's `postio-cid:` scheme, the only form its CSP shows.
    pub fn editor_html(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            write_block(&mut out, block, ImageScheme::Editor);
        }
        out
    }

    /// Total over a closed set. Do not reach for `html2text`.
    pub fn to_text(&self) -> String {
        self.render_text(Links::Spelled)
    }

    /// For a search index, so link addresses and the `[image]` placeholder are left out:
    /// a tracker URL would make a message a hit for words it never shows.
    /// A link whose visible text is its address still indexes as that.
    pub fn to_search_text(&self) -> String {
        self.render_text(Links::LabelOnly)
    }

    /// Not [`to_text`](Self::to_text): wrapping that would move the user's cursor and whitespace
    /// while they type.
    pub fn to_flowed_text(&self) -> String {
        self.to_flowed_text_at(flowed::WIDTH)
    }

    /// Split out so a property test can force wrapping below a paragraph's length.
    pub fn to_flowed_text_at(&self, width: usize) -> String {
        let mut out = String::new();
        for (index, block) in self.blocks.iter().enumerate() {
            if index > 0 {
                out.push_str("\n\n");
            }
            write_block_flowed(&mut out, block, 0, width);
        }
        out
    }

    fn render_text(&self, links: Links) -> String {
        let mut out = String::new();
        for (index, block) in self.blocks.iter().enumerate() {
            if index > 0 {
                out.push_str("\n\n");
            }
            write_block_text(&mut out, block, 0, links);
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Links {
    Spelled,
    LabelOnly,
}

fn each_block_inlines(blocks: &[Block], visit: &mut dyn FnMut(&[Inline])) {
    for block in blocks {
        match block {
            Block::Paragraph(inlines) | Block::Heading { inlines, .. } => visit(inlines),
            Block::List { items, .. } => {
                for item in items {
                    each_block_inlines(&item.blocks, visit);
                }
            }
            Block::Quote(blocks) => each_block_inlines(blocks, visit),
            Block::Table(table) => {
                for cell in table.cells() {
                    visit(cell);
                }
            }
            Block::Quoted(_) | Block::Code { .. } | Block::Rule => {}
        }
    }
}

fn visit_inlines(blocks: &[Block], visit: &mut dyn FnMut(&Inline)) {
    fn walk(inlines: &[Inline], visit: &mut dyn FnMut(&Inline)) {
        for inline in inlines {
            visit(inline);
            match inline {
                Inline::Strong(inner)
                | Inline::Emphasis(inner)
                | Inline::Strikethrough(inner)
                | Inline::Link { inlines: inner, .. } => walk(inner, visit),
                _ => {}
            }
        }
    }
    each_block_inlines(blocks, &mut |inlines| walk(inlines, visit));
}

fn replace_remote(
    blocks: &mut [Block],
    href: &WebHref,
    with: &mut dyn FnMut(String) -> Inline,
) -> bool {
    fn in_inlines(
        inlines: &mut [Inline],
        href: &WebHref,
        with: &mut dyn FnMut(String) -> Inline,
    ) -> bool {
        for inline in inlines.iter_mut() {
            match inline {
                Inline::RemoteImage { href: at, alt } if at == href => {
                    *inline = with(std::mem::take(alt));
                    return true;
                }
                Inline::Strong(inner)
                | Inline::Emphasis(inner)
                | Inline::Strikethrough(inner)
                | Inline::Link { inlines: inner, .. } => {
                    if in_inlines(inner, href, with) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
    for block in blocks.iter_mut() {
        let done = match block {
            Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
                in_inlines(inlines, href, with)
            }
            Block::List { items, .. } => items
                .iter_mut()
                .any(|item| replace_remote(&mut item.blocks, href, with)),
            Block::Quote(blocks) => replace_remote(blocks, href, with),
            Block::Table(table) => {
                let mut header = table.header().to_vec();
                let mut rows = table.rows().to_vec();
                let done = header.iter_mut().any(|cell| in_inlines(cell, href, with))
                    || rows
                        .iter_mut()
                        .flatten()
                        .any(|cell| in_inlines(cell, href, with));
                if done {
                    *table = Table::new(table.alignments().to_vec(), header, rows);
                }
                done
            }
            Block::Quoted(_) | Block::Code { .. } | Block::Rule => false,
        };
        if done {
            return true;
        }
    }
    false
}

fn collect_link_hosts_in_blocks(blocks: &[Block], hosts: &mut Vec<String>) {
    for block in blocks {
        match block {
            Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
                collect_link_hosts_in_inlines(inlines, hosts);
            }
            Block::List { items, .. } => {
                for item in items {
                    collect_link_hosts_in_blocks(&item.blocks, hosts);
                }
            }
            Block::Quote(blocks) => collect_link_hosts_in_blocks(blocks, hosts),
            // A quote's links are the original sender's; `link_hosts` is about what the draft
            // reaches out to.
            Block::Quoted(_) => {}
            Block::Table(table) => {
                for cell in table.cells() {
                    collect_link_hosts_in_inlines(cell, hosts);
                }
            }
            Block::Code { .. } | Block::Rule => {}
        }
    }
}

fn collect_link_hosts_in_inlines(inlines: &[Inline], hosts: &mut Vec<String>) {
    for inline in inlines {
        match inline {
            Inline::Link { href, inlines } => {
                if let Some(host) = host_of(href.as_str())
                    && !hosts.contains(&host)
                {
                    hosts.push(host);
                }
                collect_link_hosts_in_inlines(inlines, hosts);
            }
            Inline::Strong(inlines)
            | Inline::Emphasis(inlines)
            | Inline::Strikethrough(inlines) => {
                collect_link_hosts_in_inlines(inlines, hosts);
            }
            Inline::RemoteImage { href, .. } => {
                if let Some(host) = host_of(href.as_str())
                    && !hosts.contains(&host)
                {
                    hosts.push(host);
                }
            }
            Inline::Text(_) | Inline::Code(_) | Inline::Image { .. } | Inline::Break => {}
        }
    }
}

/// `None` for `mailto:` and anything else with no authority.
fn host_of(href: &str) -> Option<String> {
    ammonia::Url::parse(href)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

fn escape_text(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
}

fn escape_attribute(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
}

pub fn editor_image_src(content_id: &ContentId) -> String {
    format!(
        "postio-cid:{}",
        crate::sanitize::percent_encode(content_id.as_str())
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImageScheme {
    Wire,
    Editor,
}

fn write_block(out: &mut String, block: &Block, scheme: ImageScheme) {
    match block {
        Block::Paragraph(inlines) => {
            out.push_str("<p>");
            write_inlines(out, inlines, scheme);
            out.push_str("</p>");
        }
        Block::Heading { level, inlines } => {
            let _ = write!(out, "<h{}>", level.digit());
            write_inlines(out, inlines, scheme);
            let _ = write!(out, "</h{}>", level.digit());
        }
        Block::List { kind, items } => write_list(out, *kind, items, scheme),
        Block::Quote(blocks) => {
            out.push_str("<blockquote>");
            for block in blocks {
                write_block(out, block, scheme);
            }
            out.push_str("</blockquote>");
        }
        // Verbatim: narrowing is what this variant exists to stop. The marker lets `parse`
        // return a `Quoted` after a round trip through the editor's DOM.
        Block::Quoted(quoted) => {
            // A reply's quote is a `<blockquote>`; a forward's carried body is not (#1483).
            let tag = match quoted.presentation() {
                crate::Presentation::Quote => "blockquote",
                crate::Presentation::Carried => "div",
            };
            let _ = write!(
                out,
                "<{tag} {QUOTED_MARKER}=\"{}\">",
                quoted.presentation_id()
            );
            out.push_str(quoted.html());
            let _ = write!(out, "</{tag}>");
        }
        Block::Code { language, text } => {
            out.push_str("<pre");
            if let Some(language) = language {
                let _ = write!(out, " {LANGUAGE_MARKER}=\"{}\"", language.as_str());
            }
            out.push('>');
            escape_text(out, text);
            out.push_str("</pre>");
        }
        Block::Rule => out.push_str("<hr>"),
        Block::Table(table) => write_table(out, table, scheme),
    }
}

/// Not `class="language-x"`: ADR 0004 admits no `class`.
pub const LANGUAGE_MARKER: &str = "data-postio-lang";

pub const TASK_MARKER: &str = "data-postio-task";

pub const REMOTE_IMAGE_MARKER: &str = "data-postio-remote-image";

pub const OPEN_BOX: &str = "\u{2610} ";

pub const DONE_BOX: &str = "\u{2611} ";

/// A box is a glyph, not `<input type=checkbox>`, which Gmail and Outlook drop. It goes inside the
/// first paragraph; before a `<p>` it would be drawn on a line of its own.
fn write_list(out: &mut String, kind: ListKind, items: &[ListItem], scheme: ImageScheme) {
    let tag = if kind.is_ordered() { "ol" } else { "ul" };
    let _ = write!(out, "<{tag}");
    if let ListKind::Ordered { start } = kind
        && start != 1
    {
        let _ = write!(out, " start=\"{start}\"");
    }
    if kind == ListKind::Bullet && !items.is_empty() && items.iter().all(|item| item.task.is_some())
    {
        let _ = write!(
            out,
            " {TASK_MARKER}=\"list\" style=\"list-style-type:none\""
        );
    }
    out.push('>');
    for item in items {
        let (state, glyph) = match item.task {
            None => {
                out.push_str("<li>");
                for block in &item.blocks {
                    write_block(out, block, scheme);
                }
                out.push_str("</li>");
                continue;
            }
            Some(Task::Open) => ("open", OPEN_BOX),
            Some(Task::Done) => ("done", DONE_BOX),
        };
        let _ = write!(out, "<li {TASK_MARKER}=\"{state}\">");
        match item.blocks.split_first() {
            Some((Block::Paragraph(inlines), rest)) => {
                out.push_str("<p>");
                out.push_str(glyph);
                write_inlines(out, inlines, scheme);
                out.push_str("</p>");
                for block in rest {
                    write_block(out, block, scheme);
                }
            }
            _ => {
                out.push_str(glyph);
                for block in &item.blocks {
                    write_block(out, block, scheme);
                }
            }
        }
        out.push_str("</li>");
    }
    let _ = write!(out, "</{tag}>");
}

/// Layout attributes the Word engine honours: constant, and ignored by `parse`.
fn write_table(out: &mut String, table: &Table, scheme: ImageScheme) {
    out.push_str("<table border=\"1\" cellpadding=\"6\" cellspacing=\"0\"><thead><tr>");
    let row = |out: &mut String, cells: &[Cell], tag: &str| {
        for (cell, alignment) in cells.iter().zip(table.alignments()) {
            let _ = write!(out, "<{tag}");
            match alignment {
                Alignment::None => {}
                Alignment::Left => out.push_str(" align=\"left\""),
                Alignment::Center => out.push_str(" align=\"center\""),
                Alignment::Right => out.push_str(" align=\"right\""),
            }
            out.push('>');
            write_inlines(out, cell, scheme);
            let _ = write!(out, "</{tag}>");
        }
    };
    row(out, table.header(), "th");
    out.push_str("</tr></thead><tbody>");
    for cells in table.rows() {
        out.push_str("<tr>");
        row(out, cells, "td");
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table>");
}

fn write_inlines(out: &mut String, inlines: &[Inline], scheme: ImageScheme) {
    for inline in inlines {
        write_inline(out, inline, scheme);
    }
}

fn write_inline(out: &mut String, inline: &Inline, scheme: ImageScheme) {
    match inline {
        Inline::Text(text) => escape_text(out, text),
        Inline::Strong(inlines) => {
            out.push_str("<strong>");
            write_inlines(out, inlines, scheme);
            out.push_str("</strong>");
        }
        Inline::Emphasis(inlines) => {
            out.push_str("<em>");
            write_inlines(out, inlines, scheme);
            out.push_str("</em>");
        }
        Inline::Strikethrough(inlines) => {
            out.push_str("<del>");
            write_inlines(out, inlines, scheme);
            out.push_str("</del>");
        }
        Inline::Code(text) => {
            out.push_str("<code>");
            escape_text(out, text);
            out.push_str("</code>");
        }
        Inline::Link { href, inlines } => {
            out.push_str("<a href=\"");
            escape_attribute(out, href.as_str());
            out.push_str("\">");
            write_inlines(out, inlines, scheme);
            out.push_str("</a>");
        }
        // `cid:` on the wire; the editor serialises its local scheme here because its input
        // is the trusted document, not a message.
        Inline::Image { content_id, alt } => {
            match scheme {
                ImageScheme::Wire => {
                    out.push_str("<img src=\"cid:");
                    escape_attribute(out, content_id.as_str());
                }
                ImageScheme::Editor => {
                    out.push_str("<img src=\"postio-cid:");
                    out.push_str(&crate::sanitize::percent_encode(content_id.as_str()));
                }
            }
            out.push_str("\" alt=\"");
            escape_attribute(out, alt);
            out.push_str("\">");
        }
        // A link, never an `<img>`: a recipient's client fetches nothing until "Download and
        // attach".
        Inline::RemoteImage { href, alt } => {
            out.push_str("<a href=\"");
            escape_attribute(out, href.as_str());
            let _ = write!(out, "\" {REMOTE_IMAGE_MARKER}=\"\">");
            escape_text(out, if alt.is_empty() { href.as_str() } else { alt });
            out.push_str("</a>");
        }
        Inline::Break => out.push_str("<br>"),
    }
}

fn write_block_text(out: &mut String, block: &Block, depth: usize, links: Links) {
    let indent = "    ".repeat(depth);
    match block {
        Block::Paragraph(inlines) => {
            out.push_str(&indent);
            write_inlines_text(out, inlines, &indent, links);
        }
        Block::Heading { level, inlines } => {
            let mut text = String::new();
            write_inlines_text(&mut text, inlines, "", links);
            for (index, line) in heading_lines(*level, &text, links).iter().enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                out.push_str(&indent);
                out.push_str(line);
            }
        }
        Block::List { kind, items } => {
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                out.push_str(&indent);
                out.push_str(&item_marker(*kind, index, item.task, links));
                for (nth, block) in item.blocks.iter().enumerate() {
                    if nth > 0 {
                        out.push_str(item_separator(block));
                    }
                    write_block_text(out, block, if nth > 0 { depth + 1 } else { 0 }, links);
                }
            }
        }
        // The original's own text alternative under the same `> ` convention, not a flattening of
        // its markup.
        Block::Quoted(quoted) => {
            for (index, line) in quoted.text().split('\n').enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                out.push_str(&indent);
                if line.is_empty() {
                    out.push('>');
                } else {
                    out.push_str("> ");
                    out.push_str(line);
                }
            }
        }
        // `quote::fold_html_quotes` recognises this convention coming back.
        Block::Quote(blocks) => {
            let mut inner = String::new();
            for (index, block) in blocks.iter().enumerate() {
                if index > 0 {
                    inner.push_str("\n\n");
                }
                write_block_text(&mut inner, block, 0, links);
            }
            for (index, line) in inner.split('\n').enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                out.push_str(&indent);
                out.push('>');
                if !line.is_empty() {
                    out.push(' ');
                    out.push_str(line);
                }
            }
        }
        Block::Code { language, text } => {
            for (index, line) in code_lines(language.as_ref(), text, links)
                .iter()
                .enumerate()
            {
                if index > 0 {
                    out.push('\n');
                }
                out.push_str(&indent);
                out.push_str(line);
            }
        }
        Block::Rule => {
            out.push_str(&indent);
            out.push_str("----");
        }
        Block::Table(table) => {
            for (index, line) in table_lines(table, links).iter().enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                out.push_str(&indent);
                out.push_str(line);
            }
        }
    }
}

fn item_separator(next: &Block) -> &'static str {
    if matches!(next, Block::List { .. }) {
        "\n"
    } else {
        "\n\n"
    }
}

/// The search text keeps the bullet and drops the box.
fn item_marker(kind: ListKind, index: usize, task: Option<Task>, links: Links) -> String {
    let mut marker = match kind {
        ListKind::Bullet => "- ".to_owned(),
        ListKind::Ordered { start } => format!("{}. ", u64::from(start) + index as u64),
    };
    if links == Links::Spelled {
        match task {
            None => {}
            Some(Task::Open) => marker.push_str("[ ] "),
            Some(Task::Done) => marker.push_str("[x] "),
        }
    }
    marker
}

fn heading_lines(level: HeadingLevel, text: &str, links: Links) -> Vec<String> {
    if links == Links::LabelOnly {
        return text.split('\n').map(str::to_owned).collect();
    }
    match level {
        HeadingLevel::One | HeadingLevel::Two => {
            let underline = if level == HeadingLevel::One { "=" } else { "-" };
            let mut lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
            let width = lines
                .iter()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0);
            lines.push(underline.repeat(width.max(1)));
            lines
        }
        _ => {
            let hashes = "#".repeat(usize::from(level.digit()));
            vec![format!("{hashes} {}", text.replace('\n', " "))]
        }
    }
}

/// The fence outruns any backtick run in the code.
fn code_lines(language: Option<&Language>, text: &str, links: Links) -> Vec<String> {
    let body = text.strip_suffix('\n').unwrap_or(text);
    let mut lines: Vec<String> = body.split('\n').map(str::to_owned).collect();
    if links == Links::LabelOnly {
        return lines;
    }
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    lines.insert(
        0,
        format!("{fence}{}", language.map_or("", Language::as_str)),
    );
    lines.push(fence);
    lines
}

fn table_lines(table: &Table, links: Links) -> Vec<String> {
    let render = |cell: &Cell| {
        let mut text = String::new();
        write_inlines_text(&mut text, cell, "", links);
        text.replace('\n', " ")
    };
    let rows: Vec<Vec<String>> = std::iter::once(table.header())
        .chain(table.rows().iter().map(Vec::as_slice))
        .map(|cells| cells.iter().map(render).collect())
        .collect();
    if links == Links::LabelOnly {
        return rows.iter().map(|cells| cells.join(" ")).collect();
    }
    let widths: Vec<usize> = (0..table.width())
        .map(|column| {
            rows.iter()
                .map(|cells| cells[column].chars().count())
                .max()
                .unwrap_or(0)
                .max(1)
        })
        .collect();
    let line = |cells: &[String]| {
        let mut out = String::from("|");
        for ((cell, width), alignment) in cells.iter().zip(&widths).zip(table.alignments()) {
            let gap = width - cell.chars().count();
            let (before, after) = match alignment {
                Alignment::Right => (gap, 0),
                Alignment::Center => (gap / 2, gap - gap / 2),
                Alignment::None | Alignment::Left => (0, gap),
            };
            let _ = write!(out, " {}{cell}{} |", " ".repeat(before), " ".repeat(after));
        }
        out
    };
    let mut lines = vec![line(&rows[0])];
    let mut delimiter = String::from("|");
    for (width, alignment) in widths.iter().zip(table.alignments()) {
        let dashes = |n: usize| "-".repeat(n);
        delimiter.push_str(&match alignment {
            Alignment::None => dashes(width + 2),
            Alignment::Left => format!(":{}", dashes(width + 1)),
            Alignment::Right => format!("{}:", dashes(width + 1)),
            Alignment::Center => format!(":{}:", dashes(*width)),
        });
        delimiter.push('|');
    }
    lines.push(delimiter);
    lines.extend(rows[1..].iter().map(|cells| line(cells)));
    lines
}

/// RFC 3676 §4.5: markers adjacent, one space before content; `> >` is not `>>`. `0` gets no
/// prefix.
fn quote_prefix(depth: usize) -> String {
    if depth == 0 {
        String::new()
    } else {
        format!("{} ", ">".repeat(depth))
    }
}

/// `depth` is quote nesting only; list continuation alignment comes from `flowed::wrap`'s prefix.
fn write_block_flowed(out: &mut String, block: &Block, depth: usize, width: usize) {
    write_block_flowed_at(out, block, depth, "", width);
}

fn write_block_flowed_at(
    out: &mut String,
    block: &Block,
    depth: usize,
    indent: &str,
    width: usize,
) {
    let prefix = format!("{}{indent}", quote_prefix(depth));
    // Fixed line: never reflowed, space-stuffed where it would read as a quote marker.
    let fixed = |out: &mut String, lines: &[String]| {
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            out.push_str(&prefix);
            out.push_str(&flowed::stuff_for_display(line));
        }
    };
    match block {
        Block::Paragraph(inlines) => {
            let mut text = String::new();
            write_inlines_text(&mut text, inlines, "", Links::Spelled);
            write_flowed_lines(out, &text, &prefix, width);
        }
        Block::Heading { level, inlines } => {
            let mut text = String::new();
            write_inlines_text(&mut text, inlines, "", Links::Spelled);
            fixed(out, &heading_lines(*level, &text, Links::Spelled));
        }
        Block::List { kind, items } => {
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                let marker = item_marker(*kind, index, item.task, Links::Spelled);
                let item_prefix = format!("{prefix}{marker}");
                // Continuation lines align under the marker, as a quote prefix does.
                let hang_prefix = format!("{prefix}{}", " ".repeat(marker.chars().count()));
                for (nth, block) in item.blocks.iter().enumerate() {
                    if nth > 0 {
                        out.push_str(item_separator(block));
                    }
                    let block_prefix = if nth == 0 { &item_prefix } else { &hang_prefix };
                    match block {
                        Block::Paragraph(inlines) => {
                            let mut text = String::new();
                            write_inlines_text(&mut text, inlines, "", Links::Spelled);
                            write_flowed_lines(out, &text, block_prefix, width);
                        }
                        other => write_block_flowed_at(
                            out,
                            other,
                            depth,
                            &format!("{indent}    "),
                            width,
                        ),
                    }
                }
            }
        }
        Block::Quote(blocks) => {
            // The blank line between a quote's blocks keeps its marker so the quote reads as one.
            let blank = quote_prefix(depth + 1);
            for (index, block) in blocks.iter().enumerate() {
                if index > 0 {
                    let _ = write!(out, "\n{}\n", blank.trim_end());
                }
                write_block_flowed_at(out, block, depth + 1, indent, width);
            }
        }
        Block::Quoted(quoted) => {
            // Through `from_text` so the `> ` markers and wrapping come from one place.
            let inner = Document::from_text(quoted.text());
            for (index, block) in inner.blocks.iter().enumerate() {
                if index > 0 {
                    out.push_str("\n\n");
                }
                write_block_flowed_at(out, block, depth + 1, indent, width);
            }
        }
        Block::Code { language, text } => {
            fixed(out, &code_lines(language.as_ref(), text, Links::Spelled));
        }
        Block::Rule => {
            out.push_str(&prefix);
            out.push_str("----");
        }
        // A row ends in `|` so it never reads as a soft break; RFC 3676 allows 998 columns.
        Block::Table(table) => fixed(out, &table_lines(table, Links::Spelled)),
    }
}

/// Each `\n`-separated span is one [`Inline::Break`]-delimited run, kept as its own fixed line.
fn write_flowed_lines(out: &mut String, text: &str, prefix: &str, width: usize) {
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&flowed::wrap(line, prefix, width));
    }
}

fn write_inlines_text(out: &mut String, inlines: &[Inline], indent: &str, links: Links) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => out.push_str(text),
            Inline::Strong(inner) | Inline::Emphasis(inner) => {
                write_inlines_text(out, inner, indent, links)
            }
            // "~~Tuesday~~ Wednesday" is not "Tuesday Wednesday"; not in a search index.
            Inline::Strikethrough(inner) => {
                let spelled = links == Links::Spelled;
                if spelled {
                    out.push_str("~~");
                }
                write_inlines_text(out, inner, indent, links);
                if spelled {
                    out.push_str("~~");
                }
            }
            Inline::RemoteImage { href, alt } => match links {
                Links::LabelOnly => out.push_str(alt),
                Links::Spelled if alt.trim().is_empty() => out.push_str(href.as_str()),
                Links::Spelled => {
                    let _ = write!(out, "{alt} <{}>", href.as_str());
                }
            },
            Inline::Code(text) => out.push_str(text),
            // The address is the point of a link in plain text; only an index wants the label
            // alone.
            Inline::Link { href, inlines } => {
                let mut label = String::new();
                write_inlines_text(&mut label, inlines, indent, links);
                match links {
                    Links::LabelOnly => out.push_str(&label),
                    Links::Spelled if label.trim() == href.as_str() || label.trim().is_empty() => {
                        out.push_str(href.as_str())
                    }
                    Links::Spelled => {
                        let _ = write!(out, "{label} <{}>", href.as_str());
                    }
                }
            }
            Inline::Image { alt, .. } => match (links, alt.is_empty()) {
                (Links::LabelOnly, true) => {}
                (Links::LabelOnly, false) => out.push_str(alt),
                (Links::Spelled, true) => out.push_str("[image]"),
                (Links::Spelled, false) => {
                    let _ = write!(out, "[image: {alt}]");
                }
            },
            Inline::Break => {
                out.push('\n');
                out.push_str(indent);
            }
        }
    }
}

#[cfg(test)]
mod shapes {
    use super::*;

    fn text(s: &str) -> Inline {
        Inline::Text(s.to_owned())
    }

    #[test]
    fn six_heading_levels_and_none_narrows() {
        for digit in 1..=6u8 {
            let level = HeadingLevel::from_digit(digit).expect("a level");
            assert_eq!(level.digit(), digit);
        }
        assert_eq!(HeadingLevel::from_digit(4), Some(HeadingLevel::Four));
        assert_eq!(HeadingLevel::from_digit(0), None);
        assert_eq!(HeadingLevel::from_digit(7), None);
    }

    #[test]
    fn an_ordered_list_carries_its_start_and_a_bullet_list_cannot() {
        let list = Block::List {
            kind: ListKind::Ordered { start: 7 },
            items: vec![ListItem::new(vec![Block::Paragraph(vec![text("seventh")])])],
        };
        let Block::List {
            kind: ListKind::Ordered { start },
            ..
        } = list
        else {
            panic!("{list:?}")
        };
        assert_eq!(start, 7);
        assert_eq!(ListKind::NUMBERED, ListKind::Ordered { start: 1 });
        assert!(!ListKind::Bullet.is_ordered());
    }

    #[test]
    fn a_task_is_on_the_item_not_the_list() {
        let open = ListItem::task(Task::Open, vec![Block::Paragraph(vec![text("a")])]);
        let plain = ListItem::new(vec![Block::Paragraph(vec![text("b")])]);
        assert_eq!(open.task, Some(Task::Open));
        assert_eq!(plain.task, None);
    }

    #[test]
    fn a_language_is_one_short_word() {
        for word in ["rust", "c++", "objective-c", "c#", "x.y_z"] {
            assert_eq!(
                Language::parse(word).map(|l| l.as_str().to_owned()),
                Some(word.to_owned())
            );
        }
        for refused in ["a b", "\"x\"", "", "<x>", &"a".repeat(33)] {
            assert_eq!(Language::parse(refused), None, "{refused:?}");
        }
        assert!(Language::parse(&"a".repeat(32)).is_some());
    }

    #[test]
    fn a_table_is_rectangular_by_construction() {
        let (table, dropped) = Table::counting(
            vec![Alignment::Left, Alignment::Right],
            vec![vec![text("Item")], vec![text("Qty")]],
            vec![
                vec![vec![text("short")]],
                vec![vec![text("a")], vec![text("b")], vec![text("c")]],
            ],
        );
        assert_eq!(table.width(), 2);
        assert_eq!(table.rows()[0], vec![vec![text("short")], vec![]]);
        assert_eq!(table.rows()[1], vec![vec![text("a")], vec![text("b")]]);
        assert_eq!(dropped, 1, "the cell past the edge is counted");
        assert_eq!(
            Table::new(vec![], vec![vec![text("x")], vec![text("y")]], vec![]).alignments(),
            &[Alignment::None, Alignment::None],
            "with no alignments, the header sets the width"
        );
        assert_eq!(Table::new(vec![], vec![], vec![]).width(), 1);
    }

    #[test]
    fn strikethrough_nests_like_strong() {
        let inline = Inline::Strikethrough(vec![Inline::Strong(vec![text("x")])]);
        assert!(matches!(inline, Inline::Strikethrough(inner) if inner.len() == 1));
    }

    #[test]
    fn a_remote_image_is_only_ever_on_the_web() {
        assert!(WebHref::parse("https://example.com/x.png").is_some());
        assert!(WebHref::parse("HTTP://example.com/x.png").is_some());
        for refused in [
            "mailto:ada@example.com",
            "cid:x@example.com",
            "javascript:x",
            "ftp://example.com/x",
        ] {
            assert_eq!(WebHref::parse(refused), None, "{refused}");
        }
        let image = Inline::RemoteImage {
            href: WebHref::parse("https://example.com/x.png").unwrap(),
            alt: "x".to_owned(),
        };
        assert!(matches!(image, Inline::RemoteImage { .. }));
    }
}

#[cfg(test)]
mod plain_text {
    use super::*;

    fn text(s: &str) -> Inline {
        Inline::Text(s.to_owned())
    }

    fn para(s: &str) -> Block {
        Block::Paragraph(vec![text(s)])
    }

    fn doc(blocks: Vec<Block>) -> Document {
        Document { blocks }
    }

    fn both(document: &Document) -> [String; 2] {
        [document.to_text(), document.to_flowed_text()]
    }

    #[test]
    fn strikethrough_keeps_its_tildes_and_bold_does_not_keep_its_stars() {
        let document = doc(vec![Block::Paragraph(vec![
            Inline::Strikethrough(vec![text("Tuesday")]),
            text(" "),
            Inline::Strong(vec![text("Wednesday")]),
        ])]);
        for rendered in both(&document) {
            assert_eq!(rendered, "~~Tuesday~~ Wednesday");
        }
    }

    #[test]
    fn headings_one_and_two_are_underlined_and_the_rest_are_hashes() {
        let heading = |level, s: &str| Block::Heading {
            level,
            inlines: vec![text(s)],
        };
        let document = doc(vec![
            heading(HeadingLevel::One, "Plan"),
            heading(HeadingLevel::Two, "Steps"),
            heading(HeadingLevel::Three, "Three"),
            heading(HeadingLevel::Six, "Six"),
        ]);
        for rendered in both(&document) {
            assert_eq!(
                rendered,
                "Plan\n====\n\nSteps\n-----\n\n### Three\n\n###### Six"
            );
        }
    }

    #[test]
    fn a_task_keeps_its_box_and_a_list_keeps_its_start() {
        let document = doc(vec![
            Block::List {
                kind: ListKind::Bullet,
                items: vec![
                    ListItem::task(Task::Open, vec![para("open task")]),
                    ListItem::task(Task::Done, vec![para("done task")]),
                ],
            },
            Block::List {
                kind: ListKind::Ordered { start: 7 },
                items: vec![
                    ListItem::new(vec![para("seventh")]),
                    ListItem::new(vec![para("eighth")]),
                ],
            },
        ]);
        for rendered in both(&document) {
            assert_eq!(
                rendered,
                "- [ ] open task\n- [x] done task\n\n7. seventh\n8. eighth"
            );
        }
    }

    #[test]
    fn a_nested_list_sits_under_its_item_without_a_blank_line() {
        let document = doc(vec![Block::List {
            kind: ListKind::NUMBERED,
            items: vec![
                ListItem::new(vec![para("first")]),
                ListItem::new(vec![
                    para("second"),
                    Block::List {
                        kind: ListKind::Bullet,
                        items: vec![ListItem::new(vec![para("nested")])],
                    },
                ]),
            ],
        }]);
        for rendered in both(&document) {
            assert_eq!(rendered, "1. first\n2. second\n    - nested");
        }
    }

    #[test]
    fn a_code_block_is_fenced_with_its_language() {
        let document = doc(vec![Block::Code {
            language: Language::parse("rust"),
            text: "fn main() {}\n".to_owned(),
        }]);
        for rendered in both(&document) {
            assert_eq!(rendered, "```rust\nfn main() {}\n```");
        }
        let unnamed = doc(vec![Block::Code {
            language: None,
            text: "a\nb".to_owned(),
        }]);
        assert_eq!(unnamed.to_text(), "```\na\nb\n```");
    }

    #[test]
    fn a_code_line_that_looks_like_a_quote_is_stuffed_on_the_wire() {
        let document = doc(vec![Block::Code {
            language: None,
            text: ">not a quote\nFrom here".to_owned(),
        }]);
        assert_eq!(
            document.to_flowed_text(),
            "```\n >not a quote\n From here\n```"
        );
    }

    #[test]
    fn a_table_is_a_padded_pipe_table() {
        let document = doc(vec![Block::Table(Table::new(
            vec![
                Alignment::Left,
                Alignment::Right,
                Alignment::Center,
                Alignment::None,
            ],
            vec![
                vec![text("Item")],
                vec![text("Qty")],
                vec![text("c")],
                vec![text("d")],
            ],
            vec![vec![
                vec![text("Widget")],
                vec![text("3")],
                vec![text("mid")],
                vec![],
            ]],
        ))]);
        for rendered in both(&document) {
            assert_eq!(
                rendered,
                "| Item   | Qty |  c  | d |\n\
                 |:-------|----:|:---:|---|\n\
                 | Widget |   3 | mid |   |"
            );
        }
    }

    #[test]
    fn a_quote_keeps_its_marker_on_the_line_between_its_blocks() {
        let document = doc(vec![Block::Quote(vec![
            para("quoted"),
            Block::Quote(vec![para("nested quote")]),
        ])]);
        assert_eq!(document.to_flowed_text(), "> quoted\n>\n>> nested quote");
    }

    #[test]
    fn a_remote_image_is_its_words_and_its_address() {
        let document = doc(vec![Block::Paragraph(vec![Inline::RemoteImage {
            href: WebHref::parse("https://example.com/x.png").unwrap(),
            alt: "chart".to_owned(),
        }])]);
        for rendered in both(&document) {
            assert_eq!(rendered, "chart <https://example.com/x.png>");
        }
    }

    #[test]
    fn the_search_text_drops_every_marker() {
        let document = doc(vec![
            Block::Heading {
                level: HeadingLevel::One,
                inlines: vec![text("Plan")],
            },
            Block::Paragraph(vec![Inline::Strikethrough(vec![text("gone")])]),
            Block::List {
                kind: ListKind::Bullet,
                items: vec![ListItem::task(Task::Done, vec![para("done")])],
            },
            Block::Code {
                language: Language::parse("rust"),
                text: "fn x()".to_owned(),
            },
            Block::Table(Table::new(
                vec![],
                vec![vec![text("a")], vec![text("b")]],
                vec![vec![vec![text("1")], vec![text("2")]]],
            )),
            Block::Paragraph(vec![Inline::RemoteImage {
                href: WebHref::parse("https://example.com/x.png").unwrap(),
                alt: "chart".to_owned(),
            }]),
        ]);
        let search = document.to_search_text();
        for marker in ["====", "~~", "[x]", "```", "|", "rust", "example.com"] {
            assert!(!search.contains(marker), "{marker:?} in {search:?}");
        }
        for word in ["Plan", "gone", "done", "fn x()", "a b", "1 2", "chart"] {
            assert!(search.contains(word), "{word:?} not in {search:?}");
        }
    }

    #[test]
    fn a_remote_image_is_offered_then_attached_or_sent_as_a_link() {
        let href = WebHref::parse("https://example.com/x.png").unwrap();
        let mut document = Document {
            blocks: vec![Block::Paragraph(vec![
                text("see "),
                Inline::RemoteImage {
                    href: href.clone(),
                    alt: "x".to_owned(),
                },
            ])],
        };
        assert_eq!(
            document.remote_images(),
            vec![(href.clone(), "x".to_owned())]
        );
        let mut linked = document.clone();
        assert!(linked.unlink_remote(&href));
        assert!(linked.remote_images().is_empty());
        assert!(
            matches!(&linked.blocks[0], Block::Paragraph(i) if matches!(&i[1], Inline::Link { .. }))
        );
        let cid = ContentId::parse("x@postio.invalid").unwrap();
        assert!(document.attach_remote(&href, &cid));
        assert!(document.remote_images().is_empty());
        assert!(matches!(&document.blocks[0], Block::Paragraph(i)
            if matches!(&i[1], Inline::Image { alt, .. } if alt == "x")));
        assert!(!document.attach_remote(&href, &cid), "only once");
    }
}
