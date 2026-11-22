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
//! Eight block kinds and nine inline kinds is the whole language (spec 014
//! widened it from six and seven). Outgoing
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

use crate::flowed;

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

/// How deep a heading goes: `<h1>` to `<h6>`, one to one (spec 014 D3).
///
/// Six, because HTML and Markdown have six and the composer offers a command
/// for each. A level that narrowed on the way in would be a document that
/// does not read back as it was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HeadingLevel {
    /// `<h1>`.
    One,
    /// `<h2>`.
    Two,
    /// `<h3>`.
    Three,
    /// `<h4>`.
    Four,
    /// `<h5>`.
    Five,
    /// `<h6>`.
    Six,
}

impl HeadingLevel {
    /// Every level, shallowest first.
    pub const ALL: [HeadingLevel; 6] = [
        HeadingLevel::One,
        HeadingLevel::Two,
        HeadingLevel::Three,
        HeadingLevel::Four,
        HeadingLevel::Five,
        HeadingLevel::Six,
    ];

    /// The level as its HTML digit.
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

    /// The level a `<hN>` tag names. Nothing narrows: `1` to `6` map one to
    /// one, and anything else is not a heading.
    pub fn from_digit(digit: u8) -> Option<Self> {
        Self::ALL.get(usize::from(digit).checked_sub(1)?).copied()
    }
}

/// What kind of list a [`Block::List`] is.
///
/// An enum rather than `ordered: bool` so that a bullet list cannot carry a
/// start number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ListKind {
    /// `<ul>`.
    Bullet,
    /// `<ol>`, numbered from `start`. `1` is the default and `0` is allowed,
    /// as in HTML and CommonMark.
    Ordered {
        /// The first item's number.
        start: u32,
    },
}

impl ListKind {
    /// An ordered list numbered from one.
    pub const NUMBERED: ListKind = ListKind::Ordered { start: 1 };

    /// Whether the list is numbered.
    pub fn is_ordered(self) -> bool {
        matches!(self, ListKind::Ordered { .. })
    }
}

/// A task box's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Task {
    /// Not done: `- [ ]`.
    Open,
    /// Done: `- [x]`.
    Done,
}

/// One item of a [`Block::List`].
///
/// GFM puts a task box on an item, not on a list, so a list may mix tasks
/// and ordinary items, and a numbered list may hold tasks too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    /// The item's box, if it is a task.
    pub task: Option<Task>,
    /// What the item holds: a paragraph, and perhaps a nested list.
    pub blocks: Vec<Block>,
}

impl ListItem {
    /// An ordinary item.
    pub fn new(blocks: Vec<Block>) -> Self {
        ListItem { task: None, blocks }
    }

    /// A task item.
    pub fn task(task: Task, blocks: Vec<Block>) -> Self {
        ListItem {
            task: Some(task),
            blocks,
        }
    }
}

/// A code block's language: the info word after a fence.
///
/// Built only by [`Language::parse`], which accepts 1 to 32 characters, each
/// an ASCII letter, a digit or one of `+ # . _ -`. The same pattern as
/// [`Href`] and [`ContentId`]: the type cannot hold what the serialiser
/// would have to escape, so `data-postio-lang` can never carry a quote.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Language(String);

impl Language {
    /// The longest language word kept.
    pub const MAX: usize = 32;

    /// Parse `raw`, or refuse it. No trimming: a space is refused, because a
    /// caller holding a whole info string must choose its first word itself.
    pub fn parse(raw: &str) -> Option<Self> {
        let allowed =
            |c: char| c.is_ascii_alphanumeric() || matches!(c, '+' | '#' | '.' | '_' | '-');
        (!raw.is_empty() && raw.chars().count() <= Self::MAX && raw.chars().all(allowed))
            .then(|| Language(raw.to_owned()))
    }

    /// The word, as it will be written.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How a table column's cells sit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Alignment {
    /// The reader's default.
    #[default]
    None,
    /// `:--`.
    Left,
    /// `:-:`.
    Center,
    /// `--:`.
    Right,
}

/// One table cell: inlines only, so no block can nest in a cell.
/// [`Inline::Break`] is the line break inside one, as GFM's `<br>` is.
pub type Cell = Vec<Inline>;

/// A GFM table: a header row, body rows, an alignment per column.
///
/// **Rectangular**, which only [`Table::new`] can promise: it pads a short
/// row with empty cells and truncates a long one, as GFM does. There are no
/// spans, because the type has nowhere to put one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    alignments: Vec<Alignment>,
    header: Vec<Cell>,
    rows: Vec<Vec<Cell>>,
}

impl Table {
    /// A table `alignments.len()` columns wide, or as wide as `header` when
    /// no alignment is given, and never narrower than one column.
    pub fn new(alignments: Vec<Alignment>, header: Vec<Cell>, rows: Vec<Vec<Cell>>) -> Self {
        Self::counting(alignments, header, rows).0
    }

    /// As [`Table::new`], with the number of cells truncated to fit: what a
    /// narrowing reports as lost.
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

    /// One alignment per column.
    pub fn alignments(&self) -> &[Alignment] {
        &self.alignments
    }

    /// The header row, one cell per column.
    pub fn header(&self) -> &[Cell] {
        &self.header
    }

    /// The body rows, each one cell per column.
    pub fn rows(&self) -> &[Vec<Cell>] {
        &self.rows
    }

    /// How many columns.
    pub fn width(&self) -> usize {
        self.alignments.len()
    }

    /// Every cell, header first, row by row.
    pub fn cells(&self) -> impl Iterator<Item = &Cell> {
        self.header.iter().chain(self.rows.iter().flatten())
    }
}

/// The address of an image the person named but has not attached (spec 014
/// D7): an [`Href`] whose scheme is `http` or `https`.
///
/// A `mailto:` image is not an image, so it does not construct. Holding one
/// fetches nothing: [`Inline::RemoteImage`] is written out as a link, and
/// only the composer's "Download and attach" press reads it as an address.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WebHref(Href);

impl WebHref {
    /// Parse `raw`, or refuse it: [`Href::parse`], then `http` or `https`.
    pub fn parse(raw: &str) -> Option<Self> {
        let href = Href::parse(raw)?;
        let scheme = href.as_str().split(':').next().unwrap_or_default();
        (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
            .then_some(WebHref(href))
    }

    /// The address, as a link target.
    pub fn as_href(&self) -> &Href {
        &self.0
    }

    /// The address, as it will be written.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
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
    /// `<del>`.
    Strikethrough(Vec<Inline>),
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
    /// An image named by address and not attached (spec 014 D7).
    ///
    /// Never an `<img>`: it is written out as a link, so a recipient's
    /// client fetches nothing, and the composer offers to download and
    /// attach it. Only Markdown makes one; `parse` reads back the marker
    /// [`Document::to_html`] writes and never makes one from a remote `<img>`.
    RemoteImage {
        /// Where the picture is.
        href: WebHref,
        /// Alternative text.
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
    /// `<h1>` to `<h6>`.
    Heading {
        /// How deep.
        level: HeadingLevel,
        /// What it says.
        inlines: Vec<Inline>,
    },
    /// `<ul>` or `<ol>`. Each item is itself a run of blocks, so a list can
    /// hold a paragraph and a nested list.
    List {
        /// Bullets, or numbers from a start.
        kind: ListKind,
        /// The items.
        items: Vec<ListItem>,
    },
    /// `<blockquote>` — a quote the *user* made, from the authoring subset.
    Quote(Vec<Block>),
    /// `<blockquote>` — the quote a reply opens with: the original message as
    /// the reader rendered it (ADR 0033).
    ///
    /// Distinct from [`Block::Quote`] because its content is not the
    /// authoring subset and must not be narrowed to it. Narrowing is exactly
    /// what the reply quote stopped doing: a table, a colour and an inline
    /// declaration have no `Block`, and losing them is what made a reply to a
    /// rich message quote something that did not look like the message being
    /// answered.
    ///
    /// The content is still a gate rather than a hole — see
    /// [`crate::Quoted`], which only [`crate::quote_of`] can build, and which
    /// the parser rebuilds through the same constructor on the way back in so
    /// nothing the editor's DOM picked up can widen it.
    Quoted(crate::Quoted),
    /// `<pre>`. Holds a string: code is not marked up.
    Code {
        /// The fence's info word, if it had one.
        language: Option<Language>,
        /// The lines, verbatim.
        text: String,
    },
    /// `<hr>`.
    Rule,
    /// `<table>`.
    Table(Table),
}

/// The attribute that marks a `<blockquote>` as a reply quote rather than one
/// the user made.
///
/// A `data-` attribute so it is valid HTML and so the editor's own DOM keeps
/// it across a round trip. It carries no content: it says only "the parser
/// should hand this back to `quote_of` rather than narrowing it".
pub const QUOTED_MARKER: &str = "data-postio-quoted";

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

    /// [`from_text`](Self::from_text), with each run of `>` lines a
    /// [`Block::Quote`] of the text under the marker.
    ///
    /// What the composer's Rich switch builds from the plain field, which
    /// for a reply ends in the quoted original. A quote within a quote
    /// nests. Text with no `>` line is exactly what `from_text` makes.
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

    /// The document `format=flowed` (RFC 3676) text means — the inverse of
    /// [`to_flowed_text`](Self::to_flowed_text): every soft-wrapped run of
    /// physical lines becomes the one logical line it was wrapped from, and
    /// every *fixed* line (RFC 3676's term for one with no trailing space,
    /// so no more than one per [`Inline::Break`]-delimited span) stays its
    /// own line.
    ///
    /// Unlike [`from_text`](Self::from_text), which treats every `\n` as a
    /// break the user typed on purpose — correct for the composer's own
    /// buffer, wrong for text a `format=flowed` sender wrapped for the
    /// terminal. Use this one only when the source actually declared
    /// `format=flowed`; anything else, including ordinary short lines with
    /// no such header, must go through `from_text` instead, or a real break
    /// the writer meant gets silently joined onto its neighbour (#456).
    pub fn from_flowed_text(text: &str) -> Self {
        let blocks = text
            .replace("\r\n", "\n")
            .split("\n\n")
            .map(|para| {
                let mut inlines = Vec::new();
                let mut span: Vec<&str> = Vec::new();
                // Whether a span has ended yet -- not `inlines.is_empty()`,
                // which a span that unwrapped to nothing (a leading hard
                // break with no text before it) would leave empty too,
                // silently dropping that break instead of counting it.
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
    /// all and is silently skipped, the same as any href [`url`](ammonia::Url) cannot find
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

    /// Render to the outgoing `text/plain; format=flowed` alternative — RFC
    /// 3676, at [`flowed::WIDTH`].
    ///
    /// Not [`to_text`](Self::to_text): that function is the composer's own
    /// round-trip partner for [`from_text`](Self::from_text), and wrapping
    /// it would move the user's cursor and whitespace out from under them
    /// while they are still typing — see `from_text`'s own doc and the
    /// `plain_text_survives_the_composer_exactly` test this crate's suite
    /// already has. This is a second rendering of the same structure, for
    /// the one caller that wants it wrapped: the bytes that actually leave
    /// the machine.
    pub fn to_flowed_text(&self) -> String {
        self.to_flowed_text_at(flowed::WIDTH)
    }

    /// As [`to_flowed_text`](Self::to_flowed_text), at a caller-chosen width
    /// — split out for the property test that wants to see wrapping happen
    /// at a width smaller than a paragraph, without waiting on a paragraph
    /// long enough to wrap at 72 columns for real.
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
                    collect_link_hosts_in_blocks(&item.blocks, hosts);
                }
            }
            Block::Quote(blocks) => collect_link_hosts_in_blocks(blocks, hosts),
            // The reply quote's links are the *original sender's*, already
            // in front of whoever read the message. `link_hosts` answers
            // "what would this draft reach out to", which is about what the
            // user is about to send on their own behalf, so a quoted
            // newsletter's fifty footer links do not belong in that answer.
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
            // A remote image is written as a link to its address, so its
            // host is one the draft names.
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

/// The host of an `http`/`https` href, lowercased. `None` for `mailto:` and
/// anything else with no authority component to name one.
fn host_of(href: &str) -> Option<String> {
    ammonia::Url::parse(href)
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
        Block::List { kind, items } => write_list(out, *kind, items, scheme),
        Block::Quote(blocks) => {
            out.push_str("<blockquote>");
            for block in blocks {
                write_block(out, block, scheme);
            }
            out.push_str("</blockquote>");
        }
        // Verbatim, because narrowing it is the thing this variant exists to
        // stop. The marker attribute is what `parse` recognises coming back,
        // so a round trip through the editor's DOM returns a `Quoted` rather
        // than collapsing to the authoring subset (FR-046).
        Block::Quoted(quoted) => {
            // A reply's quote is a `<blockquote>`; a forward's carried body is
            // not (#1483). Both went through the same gate and carry the same
            // bytes -- what differs is whether this is somebody else's words
            // inside yours, or the message itself.
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
                // `Language` holds nothing an attribute would need escaped.
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

/// The attribute a `<pre>` carries its language in (spec 014 R3). Not
/// `class="language-x"`: ADR 0004 admits no `class`, and no mail client
/// highlights it anyway.
pub const LANGUAGE_MARKER: &str = "data-postio-lang";

/// The attribute that marks a task list and each task item (spec 014 R3).
pub const TASK_MARKER: &str = "data-postio-task";

/// The attribute that marks a link as a remote image not yet attached
/// (spec 014 D7).
pub const REMOTE_IMAGE_MARKER: &str = "data-postio-remote-image";

/// The glyph an open task item starts with on the wire, and its space.
pub const OPEN_BOX: &str = "\u{2610} ";

/// The glyph a done task item starts with on the wire, and its space.
pub const DONE_BOX: &str = "\u{2611} ";

/// A list, with its task markers (spec 014 R3).
///
/// A box is a glyph rather than `<input type=checkbox>`, which Gmail and
/// Outlook drop and Postio's own reader refuses. It goes inside the item's
/// first paragraph, so it sits on the line it marks; a `☐` before a `<p>`
/// would be drawn on a line of its own. A list of nothing but tasks hides
/// its bullets, so a box is not drawn beside a dot.
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

/// A table, with the layout attributes the Word engine honours (spec 014
/// D5): constant, generated here, and ignored by `parse`.
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
        // A link, never an `<img>`: a recipient's client fetches nothing
        // until the person who wrote it pressed "Download and attach".
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
        // The plain rendering the original carried, under the same `> `
        // convention: the rich half is what the HTML part shows, and
        // `text/plain` recipients get the text alternative the sender wrote
        // rather than a flattening of their markup.
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

/// What goes between two blocks of one list item: a blank line, except
/// before a nested list, which sits right under the line it belongs to.
fn item_separator(next: &Block) -> &'static str {
    if matches!(next, Block::List { .. }) {
        "\n"
    } else {
        "\n\n"
    }
}

/// A list item's marker: `- `, a number from the list's start, and its box
/// (spec 014 R4). The search text keeps the bullet, as it always has, and
/// drops the box.
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

/// A heading's lines: setext underlines for the first two levels, which
/// read as headings to anyone and are valid Markdown, and hashes for the
/// rest (spec 014 R4). Only the words, for a search index.
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

/// A code block's lines, between fences carrying its language. The fence
/// outruns any backtick run in the code. Only the code, for a search index.
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

/// A table as a pipe table, every cell padded to its column's widest, with
/// the alignment row (spec 014 R4). Only the cells' words, a row to a line,
/// for a search index.
fn table_lines(table: &Table, links: Links) -> Vec<String> {
    let render = |cell: &Cell| {
        let mut text = String::new();
        write_inlines_text(&mut text, cell, "", links);
        // A row is one line: a break inside a cell is a space here.
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

/// The `> ` (or `>> `, `>>> `, …) RFC 3676 §4.5 gives one quote depth: every
/// marker adjacent, one space total before content, never a space between
/// markers — `> >` is two mail clients disagreeing about nesting, `>>` is
/// one agreeing with itself. `0` is unquoted text and gets no prefix at all.
fn quote_prefix(depth: usize) -> String {
    if depth == 0 {
        String::new()
    } else {
        format!("{} ", ">".repeat(depth))
    }
}

/// [`write_block_text`]'s counterpart for the flowed alternative: the same
/// structure, wrapped at `width` per [`flowed::wrap`] instead of rendered
/// verbatim.
///
/// `depth` is quote nesting only — unlike `write_block_text`'s `depth`,
/// which also carries list indentation, because list continuation lines
/// need their own alignment worked out from `flowed::wrap`'s prefix rather
/// than reusing quote depth's meaning for a different purpose.
fn write_block_flowed(out: &mut String, block: &Block, depth: usize, width: usize) {
    write_block_flowed_at(out, block, depth, "", width);
}

/// [`write_block_flowed`], under `indent`: the spaces a nested list sits
/// behind, inside whatever quote `depth` is.
fn write_block_flowed_at(
    out: &mut String,
    block: &Block,
    depth: usize,
    indent: &str,
    width: usize,
) {
    let prefix = format!("{}{indent}", quote_prefix(depth));
    // A fixed line, never reflowed, stuffed where it would otherwise read
    // as a quote marker or a soft-break continuation.
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
                // Every continuation line after the marker aligns under it,
                // the same shape `flowed::wrap`'s own quote prefix already
                // gives a quote -- a marker is just a prefix with an
                // unusual first line.
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
            // The blank line between a quote's blocks keeps its marker, so
            // the quote reads as one and not as two.
            let blank = quote_prefix(depth + 1);
            for (index, block) in blocks.iter().enumerate() {
                if index > 0 {
                    let _ = write!(out, "\n{}\n", blank.trim_end());
                }
                write_block_flowed_at(out, block, depth + 1, indent, width);
            }
        }
        Block::Quoted(quoted) => {
            // Through the same path as any other quote: `from_text` gives the
            // sender's text alternative the block structure this needs, and
            // the `> ` markers and wrapping then come from one place.
            let inner = Document::from_text(quoted.text());
            for (index, block) in inner.blocks.iter().enumerate() {
                if index > 0 {
                    out.push_str("\n\n");
                }
                write_block_flowed_at(out, block, depth + 1, indent, width);
            }
        }
        // Code is never reflowed -- "not marked up" is this variant's whole
        // point.
        Block::Code { language, text } => {
            fixed(out, &code_lines(language.as_ref(), text, Links::Spelled));
        }
        Block::Rule => {
            out.push_str(&prefix);
            out.push_str("----");
        }
        // A row ends in `|`, so it can never read as a soft break, and a
        // row wider than the wrap width stays whole: RFC 3676 allows 998.
        Block::Table(table) => fixed(out, &table_lines(table, Links::Spelled)),
    }
}

/// Wraps each `\n`-separated span of already-rendered inline text — each
/// span is one [`Inline::Break`]-delimited run, and RFC 3676 wants every one
/// of those kept as its own *fixed* line, never joined with its neighbour by
/// [`flowed::unwrap`].
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
            // Kept, because "~~Tuesday~~ Wednesday" is not "Tuesday
            // Wednesday" (spec 014 D6). Not in a search index.
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

#[cfg(test)]
mod shapes {
    //! Spec 014's shapes (`data-model.md`), one rule per case.
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
    //! Spec 014 D6 and research R4: the plain-text alternative keeps a
    //! marker wherever dropping it would change what the words mean.
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

    /// Both renderings, which agree for everything short enough not to wrap.
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
}
