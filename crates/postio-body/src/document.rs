//! The composer's document: a closed type, and the HTML subset it maps to.
//!
//! This is the model a Postio body is edited *as*, on every frontend. It is
//! deliberately not the toolkit's buffer: `GtkTextBuffer`, `NSTextStorage`
//! and a `contenteditable` DOM disagree about what a rich-text document is —
//! attribute runs against nested spans, what constitutes one undo step, how
//! lists nest — so a composer whose state is "whatever is in the buffer"
//! makes a second frontend a rewrite rather than a port, and makes the two
//! produce different HTML from the same gestures. See [ADR 0004].
//!
//! # The set is closed, and that is the security argument
//!
//! Six block kinds and seven inline kinds is the whole language. Outgoing
//! HTML is *generated* from this type, never passed through, so:
//!
//! * [`Inline::Image`] carries a [`ContentId`] and there is no variant that
//!   can hold `https://tracker.example.com/pixel.gif`. A remote image in a
//!   quoted message does not get *stripped* — it has **no representation**.
//! * [`Href`] only constructs for `http`, `https` and `mailto`. A
//!   `javascript:` URL fails to parse rather than being filtered later.
//! * There is no styling: no colours, no fonts, no `style`, no `class`. Every
//!   styling attribute is an attribute a sanitiser would then have to reason
//!   about in both directions.
//!
//! That is why [`fn@crate::parse`] can be *total*. Its input on the quoting path is
//! attacker-controlled, and a parse error there would be a denial of service
//! on replying to a hostile message. It never fails; it **narrows**.
//!
//! # Two round trips, both properties
//!
//! * `parse(to_html(d)) == d` for every document — structure survives.
//! * `to_html(parse(h)) == h` for `h` already in the subset — [`to_html`] is
//!   the *normal form*, so re-saving a draft is a no-op rather than a slow
//!   rewrite of the user's markup.
//!
//! [`to_html`]: Document::to_html
//! [ADR 0004]: https://github.com/dlapiduz/postio/blob/main/docs/decisions/0004-composer-document-model.md

use std::fmt::Write as _;

// ---------------------------------------------------------------------------
// Leaf types
// ---------------------------------------------------------------------------

/// A link target Postio is willing to emit.
///
/// Constructed only by [`Href::parse`], which accepts three schemes. This is
/// the type-level half of "no `javascript:` in an outgoing body": there is no
/// way to build one holding anything else, so no serialiser has to remember
/// to check.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Href(String);

impl Href {
    /// The only schemes a body may link to.
    const SCHEMES: [&'static str; 3] = ["http", "https", "mailto"];

    /// Parse `raw`, or refuse it.
    ///
    /// Refuses anything without one of `http`, `https` or `mailto`, and anything
    /// carrying an ASCII control character *anywhere*. The control-character
    /// rule is not decoration: `java&#9;script:alert(1)` is a real bypass —
    /// HTML entity-decodes the tab, and a scheme check that ran before the
    /// decode would see `java\tscript` and a browser would see `javascript`.
    /// Refusing controls outright means the check cannot be walked around by
    /// choosing where to decode.
    pub fn parse(raw: &str) -> Option<Self> {
        let trimmed = raw.trim();
        if trimmed.chars().any(|c| c.is_ascii_control()) {
            return None;
        }
        let (scheme, rest) = trimmed.split_once(':')?;
        if rest.is_empty() {
            return None;
        }
        // A relative reference has no scheme and no meaning in mail: there is
        // no base document to resolve it against once the message is sent.
        Self::SCHEMES
            .iter()
            .any(|allowed| scheme.eq_ignore_ascii_case(allowed))
            .then(|| Href(trimmed.to_owned()))
    }

    /// The target, as it will be written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The `Content-ID` of a part carried by the message itself.
///
/// Never a URL. [`Inline::Image`] holds one of these and nothing else, which
/// is what makes "Postio cannot send a tracking pixel" a fact about the type
/// rather than a promise about a filter.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentId(String);

impl ContentId {
    /// Wrap a `Content-ID`, with or without the angle brackets MIME writes.
    ///
    /// Refuses anything that could be read as a URL — a colon, whitespace or
    /// a control character. A `cid:` reference whose body is `//evil.example`
    /// must not become an image source by being copied through.
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

    /// The id, without brackets.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How deep a heading goes. Three levels; a mail body is not a manual.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HeadingLevel {
    /// `<h1>`.
    One,
    /// `<h2>`.
    Two,
    /// `<h3>`.
    Three,
}

impl HeadingLevel {
    /// The level as its HTML digit.
    pub fn digit(self) -> u8 {
        match self {
            HeadingLevel::One => 1,
            HeadingLevel::Two => 2,
            HeadingLevel::Three => 3,
        }
    }

    /// The level a `<hN>` tag names, if Postio has one for it.
    ///
    /// `<h4>` and deeper narrow to [`HeadingLevel::Three`] rather than being
    /// dropped: the author meant "a heading", and losing the text entirely
    /// would be worse than losing one level of rank.
    pub fn from_digit(digit: u8) -> Option<Self> {
        match digit {
            1 => Some(HeadingLevel::One),
            2 => Some(HeadingLevel::Two),
            3..=6 => Some(HeadingLevel::Three),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// The document
// ---------------------------------------------------------------------------

/// One run of inline content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    /// Literal text. Escaped on the way out, never interpreted.
    Text(String),
    /// `<strong>`.
    Strong(Vec<Inline>),
    /// `<em>`.
    Emphasis(Vec<Inline>),
    /// `<code>`. Holds a string rather than inlines: code is not marked up.
    Code(String),
    /// `<a href>`, to somewhere [`Href`] would accept.
    Link {
        /// Where it points.
        href: Href,
        /// What it reads as.
        inlines: Vec<Inline>,
    },
    /// `<img>` of a part this message carries.
    Image {
        /// The part, by `Content-ID`.
        content_id: ContentId,
        /// Alternative text. Empty is allowed; absent is not.
        alt: String,
    },
    /// `<br>`.
    Break,
}

/// One block of content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// `<p>`.
    Paragraph(Vec<Inline>),
    /// `<h1>` to `<h3>`.
    Heading {
        /// How deep.
        level: HeadingLevel,
        /// What it says.
        inlines: Vec<Inline>,
    },
    /// `<ul>` or `<ol>`. Each item is itself a run of blocks, so a list can
    /// hold a paragraph and a nested list.
    List {
        /// `<ol>` when true.
        ordered: bool,
        /// The items.
        items: Vec<Vec<Block>>,
    },
    /// `<blockquote>` — what quoting a reply produces.
    Quote(Vec<Block>),
    /// `<pre>`. Holds a string: preformatted text is not marked up.
    Pre(String),
    /// `<hr>`.
    Rule,
}

/// A message body, as it is edited.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
    /// The blocks, in order.
    pub blocks: Vec<Block>,
}

impl Document {
    /// An empty document.
    pub fn new() -> Self {
        Document::default()
    }

    /// The document a plain-text composer produces.
    ///
    /// One paragraph per blank-line-separated run, `Break` for the single
    /// newlines inside one. A document of nothing but [`Block::Paragraph`]
    /// and [`Inline::Text`] *is* a plain-text document — which is why v1 can
    /// ship a plain-text editor over the neutral model without restricting
    /// the model.
    ///
    /// **Lossless**, and paired exactly with [`Document::to_text`]:
    /// `to_text(from_text(t)) == t` for any `t` with no carriage returns.
    /// Empty paragraphs and trailing newlines are kept rather than tidied
    /// away, because they are the user's. A reply opens with blank lines
    /// above the signature for the user to type into, and a composer that
    /// swallowed them on the first read would move the cursor out from under
    /// them — which is what the `gtk_identity` test caught when this did tidy.
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

    /// Whether there is anything to send.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Whether [`Document::to_text`] loses nothing.
    ///
    /// True for a document of nothing but paragraphs and unstyled text —
    /// which is what a plain-text composer produces, and what
    /// [`Document::from_text`] builds. The composer uses this to decide
    /// whether a message needs an HTML alternative at all: sending
    /// `multipart/alternative` for a message somebody typed as plain text
    /// adds bytes, adds a second thing to get wrong, and is exactly what
    /// mailing lists ask people not to do.
    pub fn is_plain_text(&self) -> bool {
        self.blocks.iter().all(|block| match block {
            Block::Paragraph(inlines) => inlines
                .iter()
                .all(|inline| matches!(inline, Inline::Text(_) | Inline::Break)),
            _ => false,
        })
    }

    /// Every link's host, lowercased and deduplicated.
    ///
    /// Not [`Href`] itself: a caller comparing against a sender's own domain
    /// (issue #116's composer banner is the first) wants just the host, and
    /// parsing one out of a raw href belongs with the type that already
    /// knows what a valid href looks like. A `mailto:` link names no host at
    /// all and is silently skipped, the same as any href [`url`] cannot find
    /// one in.
    pub fn link_hosts(&self) -> Vec<String> {
        let mut hosts = Vec::new();
        collect_link_hosts_in_blocks(&self.blocks, &mut hosts);
        hosts
    }

    /// Render to the HTML subset.
    ///
    /// The **normal form**: no whitespace between blocks, attributes in a
    /// fixed order, every text node escaped. That is what makes
    /// `to_html(parse(h)) == h` hold for anything already in the subset, and
    /// so what makes re-saving a draft a no-op.
    pub fn to_html(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            write_block(&mut out, block, ImageScheme::Wire);
        }
        out
    }

    /// As [`to_html`](Self::to_html), but with every image source in the
    /// editing shell's `postio-cid:` scheme rather than the wire's `cid:` —
    /// the only form the shell's CSP will show. `parse` accepts the editing
    /// scheme back, so `parse(editor_html(d)) == d` holds exactly as the
    /// wire round trip does.
    pub fn editor_html(&self) -> String {
        let mut out = String::new();
        for block in &self.blocks {
            write_block(&mut out, block, ImageScheme::Editor);
        }
        out
    }

    /// Render to the plain-text alternative.
    ///
    /// A total function over a closed set, which is the whole reason the set
    /// is closed: over arbitrary HTML this is the function that makes most
    /// mail's `text/plain` part unreadable. **Do not reach for `html2text`** —
    /// convert from here.
    pub fn to_text(&self) -> String {
        self.render_text(Links::Spelled)
    }

    /// The document as the words a person would read, for a search index.
    ///
    /// [`to_text`](Self::to_text) is written for *quoting*, where a link's
    /// address is the point: dropping it leaves "click here" pointing at
    /// nothing. An index wants the opposite. A message that links to
    /// `tracker.example` does not say "tracker.example" anywhere a reader can
    /// see, so indexing it makes that message a hit for a word it never
    /// contained — and one tracking redirect would then answer for every
    /// campaign that used the same shortener. Same for the `[image]`
    /// placeholder, which would make every message carrying a picture a hit
    /// for the word "image".
    ///
    /// A link whose visible text *is* its address still indexes as that
    /// address: that is what the message says.
    pub fn to_search_text(&self) -> String {
        self.render_text(Links::LabelOnly)
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

/// How a rendered-to-text link spells itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Links {
    /// Label and address, the way a quoted reply needs it.
    Spelled,
    /// The visible label alone, the way an index needs it.
    LabelOnly,
}

// ---------------------------------------------------------------------------
// Link hosts (issue #116)
// ---------------------------------------------------------------------------

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

/// The host of an `http`/`https` href, lowercased. `None` for `mailto:` and
/// anything else with no authority component to name one.
fn host_of(href: &str) -> Option<String> {
    url::Url::parse(href)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

// ---------------------------------------------------------------------------
// HTML out
// ---------------------------------------------------------------------------

/// Escape text for an element's content.
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

/// Escape text for a double-quoted attribute value.
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

/// One image source in the editing shell's scheme: what
/// [`Document::editor_html`] emits for it, and what an editing gesture that
/// inserts an image writes into the DOM.
pub fn editor_image_src(content_id: &ContentId) -> String {
    format!(
        "postio-cid:{}",
        crate::sanitize::percent_encode(content_id.as_str())
    )
}

/// Which scheme an `<img>` source carries: the wire's `cid:`, or the editing
/// shell's `postio-cid:` (percent-encoded, matching what the scheme handler
/// decodes).
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
        // `cid:` on the wire; the rewrite to a local scheme is a
        // display-time concern, and the editor is the one display that
        // serialises it here (`editor_html`) rather than through
        // `sanitize_body` — its input is the trusted document, not a
        // message.
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

// ---------------------------------------------------------------------------
// Text out
// ---------------------------------------------------------------------------

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
        // The convention every mail client understands, and the one
        // `quote::fold_html_quotes` recognises coming back the other way.
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

fn write_inlines_text(out: &mut String, inlines: &[Inline], indent: &str, links: Links) {
    for inline in inlines {
        match inline {
            Inline::Text(text) => out.push_str(text),
            Inline::Strong(inner) | Inline::Emphasis(inner) => {
                write_inlines_text(out, inner, indent, links)
            }
            Inline::Code(text) => out.push_str(text),
            // The address is the point of a link in plain text, and dropping
            // it would leave "click here" pointing at nothing. An index is
            // the one reader that wants the label alone -- see
            // [`Document::to_search_text`].
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
                // A picture with no alt text says nothing; only a reader
                // looking at the layout needs to be told one was here.
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
