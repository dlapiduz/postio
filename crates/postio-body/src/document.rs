//! The composer's document: a closed type and the HTML subset it maps to (ADR 0004).
//! Outgoing HTML is generated from it, so a remote image or `javascript:` link has no
//! representation.
//! `parse` is total, since its quoting-path input is attacker-controlled: it narrows, never fails.

use std::fmt::Write as _;

use crate::flowed;

/// A link target limited to `http`, `https` and `mailto`; only `Href::parse` builds one.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Href(String);

impl Href {
    const SCHEMES: [&'static str; 3] = ["http", "https", "mailto"];

    /// `None` for any other scheme, or any ASCII control character: `java&#9;script:` is a bypass
    /// if the scheme check runs before entity decoding.
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.chars().any(|c| c.is_ascii_control()) {
            return None;
        }
        let (scheme, rest) = trimmed.split_once(':')?;
        if rest.is_empty() {
            return None;
        }
        // A relative reference has no base document to resolve against once sent.
        Self::SCHEMES
            .iter()
            .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
            .then(|| Href(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The `Content-ID` of a part the message carries; never a URL, so a tracking pixel has no
/// representation.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentId(String);

impl ContentId {
    /// Accepts the id with or without angle brackets; `None` for anything readable as a URL (colon,
    /// whitespace, control character).
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
}

impl HeadingLevel {
    pub fn digit(self) -> u8 {
        match self {
            HeadingLevel::One => 1,
            HeadingLevel::Two => 2,
            HeadingLevel::Three => 3,
        }
    }

    /// `<h4>` and deeper narrow to `Three` rather than lose the text.
    pub fn from_digit(digit: u8) -> Option<Self> {
        match digit {
            1 => Some(HeadingLevel::One),
            2 => Some(HeadingLevel::Two),
            3..=6 => Some(HeadingLevel::Three),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Strong(Vec<Inline>),
    Emphasis(Vec<Inline>),
    Code(String),
    Link { href: Href, inlines: Vec<Inline> },
    Image { content_id: ContentId, alt: String },
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
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    Quote(Vec<Block>),
    /// The reply quote (ADR 0033): the original as rendered, not narrowed to the authoring subset,
    /// so tables and colours survive.
    /// Only `quote_of` builds the content, and the parser rebuilds it through the same gate.
    Quoted(crate::Quoted),
    Pre(String),
    Rule,
}

/// Marks a `<blockquote>` as a reply quote, for the parser to hand back to `quote_of` rather than
/// narrow.
pub const QUOTED_MARKER: &str = "data-postio-quoted";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
    pub blocks: Vec<Block>,
}

impl Document {
    pub fn new() -> Self {
        Document::default()
    }

    /// One paragraph per blank-line-separated run, `Break` for single newlines.
    /// Lossless: `to_text(from_text(t)) == t` for `t` without CR. Empty paragraphs and trailing
    /// newlines are kept so a reply's blank lines above the signature stay under the cursor.
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

    /// `from_text` with each run of `>` lines a `Block::Quote`; quotes nest.
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

    /// The inverse of `to_flowed_text` for `format=flowed` (RFC 3676) text.
    /// Use only when the source declared `format=flowed`: on other text it joins breaks the writer
    /// meant (#456).
    pub fn from_flowed_text(text: &str) -> Self {
        let blocks = text
            .replace("\r\n", "\n")
            .split("\n\n")
            .map(|para| {
                let mut inlines = Vec::new();
                let mut span: Vec<&str> = Vec::new();
                // Not `inlines.is_empty()`: a span that unwrapped to nothing would drop its leading
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

    /// True for only paragraphs of unstyled text; the composer then skips the HTML alternative.
    pub fn is_plain_text(&self) -> bool {
        self.blocks.iter().all(|block| match block {
            Block::Paragraph(inlines) => inlines
                .iter()
                .all(|inline| matches!(inline, Inline::Text(_) | Inline::Break)),
            _ => false,
        })
    }

    /// A `mailto:` link, or any href with no host, is skipped.
    pub fn link_hosts(&self) -> Vec<String> {
        let mut hosts = Vec::new();
        collect_link_hosts_in_blocks(&self.blocks, &mut hosts);
        hosts
    }

    /// The normal form (fixed attribute order, escaped text), so `to_html(parse(h)) == h` and
    /// re-saving a draft is a no-op.
    pub fn to_html(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            write_block(&mut out, block, ImageScheme::Wire);
        }
        out
    }

    /// Image sources use the shell's `postio-cid:` scheme, the only one its CSP shows; `parse`
    /// accepts it back.
    pub fn editor_html(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            write_block(&mut out, block, ImageScheme::Editor);
        }
        out
    }

    /// Do not reach for `html2text`; convert from this closed set.
    pub fn to_text(&self) -> String {
        self.render_text(Links::Spelled)
    }

    /// For the search index: link addresses and the `[image]` placeholder are omitted, or a
    /// tracking redirect and every picture would match words the message never shows.
    pub fn to_search_text(&self) -> String {
        self.render_text(Links::LabelOnly)
    }

    /// Not `to_text`, which round-trips with `from_text`: wrapping it would move the user's cursor
    /// and whitespace while typing.
    pub fn to_flowed_text(&self) -> String {
        self.to_flowed_text_at(flowed::WIDTH)
    }

    /// `to_flowed_text` at a chosen width, so tests can see wrapping below a paragraph's length.
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

fn collect_link_hosts_in_blocks(blocks: &[Block], hosts: &mut Vec<String>) {
    for block in blocks {
        match block {
            Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
                collect_link_hosts_in_inlines(inlines, hosts);
            }
            Block::List { items, .. } => {
                for item in items {
                    collect_link_hosts_in_blocks(item, hosts);
                }
            }
            Block::Quote(blocks) => collect_link_hosts_in_blocks(blocks, hosts),
            // A reply quote's links are the original sender's, not what this draft reaches out to.
            Block::Quoted(_) => {}
            Block::Pre(_) | Block::Rule => {}
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
            Inline::Strong(inlines) | Inline::Emphasis(inlines) => {
                collect_link_hosts_in_inlines(inlines, hosts);
            }
            Inline::Text(_) | Inline::Code(_) | Inline::Image { .. } | Inline::Break => {}
        }
    }
}

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
        Block::List { ordered, items } => {
            let tag = if *ordered { "ol" } else { "ul" };
            let _ = write!(out, "<{tag}>");
            for item in items {
                out.push_str("<li>");
                for block in item {
                    write_block(out, block, scheme);
                }
                out.push_str("</li>");
            }
            let _ = write!(out, "</{tag}>");
        }
        Block::Quote(blocks) => {
            out.push_str("<blockquote>");
            for block in blocks {
                write_block(out, block, scheme);
            }
            out.push_str("</blockquote>");
        }
        // Verbatim: narrowing is what this variant exists to stop. The marker lets `parse` return a
        // `Quoted` after the editor's DOM round trip.
        Block::Quoted(quoted) => {
            // A forward's carried body is a `div`, not a `blockquote` (#1483).
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
        Block::Pre(text) => {
            out.push_str("<pre>");
            escape_text(out, text);
            out.push_str("</pre>");
        }
        Block::Rule => out.push_str("<hr>"),
    }
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
        // `cid:` on the wire; the editor serialises its local scheme here because its input is the
        // trusted document, not a message.
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
        Block::Heading { inlines, .. } => {
            out.push_str(&indent);
            write_inlines_text(out, inlines, &indent, links);
        }
        Block::List { ordered, items } => {
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                out.push_str(&indent);
                if *ordered {
                    let _ = write!(out, "{}. ", index + 1);
                } else {
                    out.push_str("- ");
                }
                for (nth, block) in item.iter().enumerate() {
                    if nth > 0 {
                        out.push_str("\n\n");
                    }
                    write_block_text(out, block, if nth > 0 { depth + 1 } else { 0 }, links);
                }
            }
        }
        // The sender's text alternative under `> `, so `text/plain` recipients get it rather than a
        // flattening of the markup.
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
        Block::Pre(text) => {
            for (index, line) in text.split('\n').enumerate() {
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
    }
}

/// RFC 3676 §4.5: markers adjacent, one space before content (`>>`, never `> >`); `0` gets no
/// prefix.
fn quote_prefix(depth: usize) -> String {
    if depth == 0 {
        String::new()
    } else {
        format!("{} ", ">".repeat(depth))
    }
}

/// The flowed counterpart of `write_block_text`. `depth` is quote nesting only: list continuation
/// lines align through `flowed::wrap`'s prefix.
fn write_block_flowed(out: &mut String, block: &Block, depth: usize, width: usize) {
    let prefix = quote_prefix(depth);
    match block {
        Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
            let mut text = String::new();
            write_inlines_text(&mut text, inlines, "", Links::Spelled);
            write_flowed_lines(out, &text, &prefix, width);
        }
        Block::List { ordered, items } => {
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                let marker = if *ordered {
                    format!("{}. ", index + 1)
                } else {
                    "- ".to_owned()
                };
                let item_prefix = format!("{prefix}{marker}");
                let hang_prefix = format!("{prefix}{}", " ".repeat(marker.chars().count()));
                for (nth, block) in item.iter().enumerate() {
                    if nth > 0 {
                        out.push_str("\n\n");
                    }
                    let block_prefix = if nth == 0 { &item_prefix } else { &hang_prefix };
                    match block {
                        Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
                            let mut text = String::new();
                            write_inlines_text(&mut text, inlines, "", Links::Spelled);
                            write_flowed_lines(out, &text, block_prefix, width);
                        }
                        other => write_block_flowed(out, other, depth, width),
                    }
                }
            }
        }
        Block::Quote(blocks) => {
            for (index, block) in blocks.iter().enumerate() {
                if index > 0 {
                    out.push_str("\n\n");
                }
                write_block_flowed(out, block, depth + 1, width);
            }
        }
        Block::Quoted(quoted) => {
            let inner = Document::from_text(quoted.text());
            for (index, block) in inner.blocks.iter().enumerate() {
                if index > 0 {
                    out.push_str("\n\n");
                }
                write_block_flowed(out, block, depth + 1, width);
            }
        }
        Block::Pre(text) => {
            // Never reflowed, but it shares the wire with flowed lines, so quote-like and
            // continuation-like starts still need stuffing.
            for (index, line) in text.split('\n').enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                out.push_str(&prefix);
                out.push_str(&flowed::stuff_for_display(line));
            }
        }
        Block::Rule => {
            out.push_str(&prefix);
            out.push_str("----");
        }
    }
}

/// Each `Break`-delimited span stays its own fixed line, never joined by `flowed::unwrap`.
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
            Inline::Code(text) => out.push_str(text),
            // The address is the point of a link in plain text; only the index wants the label
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
                // A picture with no alt text says nothing to an index.
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
