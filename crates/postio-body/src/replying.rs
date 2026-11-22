//! The documents a reply and a forward start from (ADR 0003 Q3, amended by ADR 0033). The quote is
//! built here because quoting parses untrusted markup and the parser sits above the model.
//! `Quoted` is a gate: only `quote_of` builds one, through the reader's own sanitiser with remote
//! images blocked, so reader and reply share one policy.

use postio_model::MessageBody;
use postio_model::account::Signature;

use crate::document::{Block, Document, Inline};
use crate::sanitize::{RemoteImages, Sanitized, sanitize_body_in};

/// Mirrors `postio_model::signature::SEPARATOR`; the model sits below this crate.
const SEPARATOR: &str = "--";

/// Opaque on purpose: the only constructor is `quote_of`, so a `Quoted` is always markup that went
/// through `sanitize` with remote images blocked (ADR 0003 Q3, ADR 0033).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Quoted {
    html: String,
    styles: String,
    text: String,
    presentation: Presentation,
}

/// A reply marks what it answers as a quote; a forward presents the whole message and does not
/// (#1483).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Presentation {
    #[default]
    Quote,
    Carried,
}

impl Quoted {
    pub fn html(&self) -> &str {
        &self.html
    }

    /// Scoped so it cannot match outside this quote; CSS Postio parsed and rewrote, never the
    /// sender's `<style>` passed through.
    pub fn styles(&self) -> &str {
        &self.styles
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn presentation(&self) -> Presentation {
        self.presentation
    }

    pub fn presentation_id(&self) -> &'static str {
        match self.presentation {
            Presentation::Quote => "1",
            Presentation::Carried => "carried",
        }
    }

    pub fn carried(mut self) -> Self {
        self.presentation = Presentation::Carried;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.html.trim().is_empty() && self.text.trim().is_empty()
    }
}

/// `html` is `None` for a plain-text original; the text alternative is also used when the HTML
/// sanitises away entirely, so a reply never opens with an empty quote.
/// Remote images are always blocked whatever the reader allowed (ADR 0033 Q2): re-emitting them
/// would give the sender a beacon in other people's clients.
pub fn quote_of(html: Option<&str>, text: &str, scope: &str) -> Quoted {
    // Returned before the fallback: `Document::from_text("")` still renders an empty paragraph,
    // which would say something was quoted.
    if html.is_none_or(|html| html.trim().is_empty()) && text.trim().is_empty() {
        return Quoted::default();
    }
    if let Some(html) = html.filter(|html| !html.trim().is_empty()) {
        let sanitized = sanitize_body_in(html, RemoteImages::Blocked, Some(scope));
        let sanitized = Sanitized {
            html: caption_images(&sanitized.html),
            ..sanitized
        };
        if !sanitized.html.trim().is_empty() {
            // An HTML-only message still needs a `text/plain` half; narrowing the sanitised markup
            // is the reduction the reader shows with markup off.
            let text = if text.trim().is_empty() {
                crate::parse::parse(&sanitized.html).to_text()
            } else {
                text.to_owned()
            };
            return Quoted {
                html: sanitized.html,
                styles: sanitized.styles,
                text,
                presentation: Presentation::Quote,
            };
        }
    }
    // Through `from_text`, not one `<p>`: a sender's line break is content (#456) and HTML would
    // collapse it to a space.
    let escaped = sanitize_body_in(
        &Document::from_text(text).to_html(),
        RemoteImages::Blocked,
        Some(scope),
    );
    Quoted {
        html: escaped.html,
        styles: String::new(),
        text: text.to_owned(),
        presentation: Presentation::Quote,
    }
}

/// A reply carries none of the original's parts, so every quote image is broken and the recipient
/// has no banner to explain it. Each becomes its `alt` text; one with none leaves nothing.
/// Rejected: carrying the parts, which inflates every reply and forwards the sender's content to
/// third parties.
fn caption_images(html: &str) -> String {
    // Text, not markup: a description must not be parsed as tags.
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find("<img") {
        out.push_str(&rest[..at]);
        let Some(end) = rest[at..].find('>') else {
            break;
        };
        let tag = &rest[at..at + end + 1];
        if let Some(alt) = attribute_value(tag, "alt").filter(|alt| !alt.trim().is_empty()) {
            out.push_str("<span>[");
            out.push_str(&alt);
            out.push_str("]</span>");
        }
        rest = &rest[at + end + 1..];
    }
    out.push_str(rest);
    out
}

/// Reads only tags `ammonia` just wrote (double-quoted, escaped); not a parser, never for anyone
/// else's markup.
fn attribute_value(tag: &str, name: &str) -> Option<String> {
    let needle = format!(" {name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let end = start + tag[start..].find('"')?;
    Some(tag[start..end].to_owned())
}

/// The caret line is a `Break` paragraph, since `parse` narrows an empty one away on the first
/// editor round trip.
pub fn quoted_reply(source: &Quoted, attribution: &str) -> Document {
    let mut blocks = vec![
        Block::Paragraph(vec![Inline::Break]),
        Block::Paragraph(vec![Inline::Text(attribution.to_owned())]),
    ];
    if !source.is_empty() {
        blocks.push(Block::Quoted(source.clone()));
    }
    Document { blocks }
}

/// The same `Quoted` a reply carries (#1483), so forwarding and replying to a table-based
/// newsletter keep it alike. Carried, not frozen: `parse` rebuilds a `Quoted` from the editor's
/// DOM, so trimming a forward survives.
pub fn forwarded(source: &Quoted, header_lines: &[String]) -> Document {
    let mut header = Vec::new();
    for (index, line) in header_lines.iter().enumerate() {
        if index > 0 {
            header.push(Inline::Break);
        }
        header.push(Inline::Text(line.clone()));
    }
    let mut blocks = vec![Block::Paragraph(vec![Inline::Break])];
    if !header.is_empty() {
        blocks.push(Block::Paragraph(header));
    }
    if !source.is_empty() {
        blocks.push(Block::Quoted(source.clone().carried()));
    }
    Document { blocks }
}

/// Swaps the quote in a reply in progress for `source` ("Reply to latest"). The person's words
/// (above the quote minus its attribution, and anything below it such as a signature) are kept byte
/// for byte in both halves.
/// A body with no quote gets the new one at the end.
pub fn requote(old: &MessageBody, attribution: &str, source: &Quoted) -> MessageBody {
    MessageBody {
        text: old
            .text
            .as_deref()
            .map(|text| requote_text(text, attribution, source)),
        html: old
            .html
            .as_deref()
            .map(|html| requote_html(html, attribution, source)),
    }
}

fn requote_text(text: &str, attribution: &str, source: &Quoted) -> String {
    use crate::quote::{Stretch, text_stretches, without_attribution};
    let stretches = text_stretches(text);
    let first = stretches
        .iter()
        .position(|stretch| matches!(stretch, Stretch::Quoted(_)));
    let last = stretches
        .iter()
        .rposition(|stretch| matches!(stretch, Stretch::Quoted(_)));
    let (mut typed, after) = match (first, last) {
        (Some(first), Some(last)) => {
            let own = |range: &[Stretch<'_>]| {
                range
                    .iter()
                    .map(|stretch| match stretch {
                        Stretch::Own(words) | Stretch::Quoted(words) => *words,
                    })
                    .collect::<String>()
            };
            let before = own(&stretches[..first]);
            (
                without_attribution(&before).to_owned(),
                own(&stretches[last + 1..]),
            )
        }
        _ => (text.to_owned(), String::new()),
    };
    if !typed.is_empty() && !typed.ends_with("\n\n") {
        typed.push_str(if typed.ends_with('\n') { "\n" } else { "\n\n" });
    }
    let mut out = typed;
    out.push_str(attribution);
    out.push('\n');
    let quoted: Vec<String> = source
        .text()
        .lines()
        .map(|line| {
            if line.is_empty() {
                ">".to_owned()
            } else {
                format!("> {line}")
            }
        })
        .collect();
    out.push_str(&quoted.join("\n"));
    if quoted.is_empty() {
        out.pop();
    }
    if !after.is_empty() {
        out.push('\n');
        out.push_str(&after);
    }
    out
}

fn requote_html(html: &str, attribution: &str, source: &Quoted) -> String {
    use crate::quote::{Stretch, html_stretches, html_without_attribution};
    let stretches = html_stretches(html);
    let first = stretches
        .iter()
        .position(|stretch| matches!(stretch, Stretch::Quoted(_)));
    let last = stretches
        .iter()
        .rposition(|stretch| matches!(stretch, Stretch::Quoted(_)));
    let join = |range: &[Stretch<'_>]| {
        range
            .iter()
            .map(|stretch| match stretch {
                Stretch::Own(markup) | Stretch::Quoted(markup) => *markup,
            })
            .collect::<String>()
    };
    let (typed, after) = match (first, last) {
        (Some(first), Some(last)) => {
            let before = join(&stretches[..first]);
            (
                html_without_attribution(&before).to_owned(),
                join(&stretches[last + 1..]),
            )
        }
        _ => (html.to_owned(), String::new()),
    };
    let mut blocks = vec![Block::Paragraph(vec![Inline::Text(attribution.to_owned())])];
    if !source.is_empty() {
        blocks.push(Block::Quoted(source.clone()));
    }
    format!("{typed}{}{after}", Document { blocks }.to_html())
}

/// Both conventions are in wide use, so the choice is the user's per draft kind (#12); moving a
/// signature has to remove the old one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    #[default]
    BelowQuote,
    AboveQuote,
}

/// `postio_model::signature::apply` at the block level, so a rich body is never flattened. The
/// separator is a top-level `--` paragraph; the last one wins, and one followed by a `Block::Quote`
/// is not a separator, since a quoted message's signature is somebody else's.
/// Above-quote placement finds its own separator by position (`existing_signature`). The rich
/// signature is used when there is one.
pub fn apply_signature(
    document: &Document,
    signature: Option<&Signature>,
    placement: Placement,
) -> Document {
    let mut blocks = document.blocks.clone();
    if let Some(range) = existing_signature(&blocks) {
        blocks.drain(range);
    }

    let Some(signature) = signature.filter(|signature| !signature.text.trim().is_empty()) else {
        return Document { blocks };
    };

    let inserted = separated(signature_blocks(signature));

    // Somewhere to type: otherwise the caret opens inside the separator paragraph of an empty
    // draft.
    if blocks.is_empty() {
        // Empty, not a hard break: the plain-text pipeline's spelling (one blank line, then the
        // separator).
        blocks.push(Block::Paragraph(Vec::new()));
    }

    let at = match placement {
        Placement::BelowQuote => blocks.len(),
        Placement::AboveQuote => blocks
            .iter()
            .position(|block| matches!(block, Block::Quote(_) | Block::Quoted(_)))
            .unwrap_or(blocks.len()),
    };
    blocks.splice(at..at, inserted);
    Document { blocks }
}

/// Directly above, in the same paragraph: a separate paragraph renders a blank line under the
/// separator, which stops other clients recognising it.
fn separated(blocks: Vec<Block>) -> Vec<Block> {
    let separator = Inline::Text(format!("{SEPARATOR} "));
    let mut blocks = blocks.into_iter();
    match blocks.next() {
        Some(Block::Paragraph(inlines)) => {
            let mut first = vec![separator, Inline::Break];
            first.extend(inlines);
            std::iter::once(Block::Paragraph(first))
                .chain(blocks)
                .collect()
        }
        Some(other) => std::iter::once(Block::Paragraph(vec![separator]))
            .chain(std::iter::once(other))
            .chain(blocks)
            .collect(),
        None => vec![Block::Paragraph(vec![separator])],
    }
}

fn signature_blocks(signature: &Signature) -> Vec<Block> {
    match signature.html.as_deref().filter(|html| !html.is_empty()) {
        // Through `parse`, so a signature cannot smuggle in what a body could not.
        Some(html) => crate::parse(html).blocks,
        None => Document::from_text(signature.text.trim_end()).blocks,
    }
}

/// Runs from the separator to the end, or to the quote after it when placed above one. Separators
/// inside a `Block::Quote` are never top level, so a quoted message's signature is skipped.
fn existing_signature(blocks: &[Block]) -> Option<std::ops::Range<usize>> {
    let separator = blocks.iter().rposition(is_separator)?;
    let end = blocks[separator..]
        .iter()
        .position(|block| matches!(block, Block::Quote(_) | Block::Quoted(_)))
        .map(|offset| separator + offset)
        .unwrap_or(blocks.len());
    Some(separator..end)
}

/// "Opens": `separated` puts separator and signature in one paragraph, so the paragraph's first
/// line marks the boundary.
fn is_separator(block: &Block) -> bool {
    let Block::Paragraph(inlines) = block else {
        return false;
    };
    let Some(Inline::Text(text)) = inlines.first() else {
        return false;
    };
    if text.trim_end() != SEPARATOR || text.starts_with(' ') {
        return false;
    }
    matches!(inlines.get(1), None | Some(Inline::Break))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyKind {
    Reply,
    ReplyAll,
    Forward,
}

/// Here, not in a frontend, so desktop and terminal start a reply from the same draft.
pub fn reply_draft(
    kind: ReplyKind,
    source: &postio_model::Message,
    account: &postio_model::Account,
) -> postio_model::Draft {
    match kind {
        ReplyKind::Reply => postio_model::reply::reply(source, account, quoted_body(source, false)),
        ReplyKind::ReplyAll => {
            postio_model::reply::reply_all(source, account, quoted_body(source, false))
        }
        ReplyKind::Forward => {
            postio_model::reply::forward(source, account, quoted_body(source, true))
        }
    }
}

/// Rich in both halves: the editor opens the HTML, and the text keeps the `> ` convention.
/// A reply and a forward both carry the sender's markup through `quote_of` (ADR 0033); a forward is
/// presented as the message, not a quote (#1483).
pub fn quoted_body(source: &postio_model::Message, forward: bool) -> postio_model::MessageBody {
    // A source whose body has not arrived has nothing to quote; an attribution above nothing would
    // say "X wrote:" and stop. The composer notes the original is still downloading.
    if source.body.text.is_none() && source.body.html.is_none() {
        return postio_model::MessageBody::default();
    }
    let rich = if forward {
        // Carried content like a reply's (#1483); only the presentation differs, since a forward is
        // not a quote.
        let carried = quote_of(
            source.body.html.as_deref(),
            &forward_text(source),
            QUOTE_SCOPE,
        );
        forwarded(&carried, &postio_model::reply::forward_header(source))
    } else {
        // Without markup the text goes through `source_document`, where `format=flowed` is
        // unwrapped (#456); with markup the sender's alternative is taken as written.
        let text = match source.body.html {
            Some(_) => source.body.text.clone().unwrap_or_default(),
            None => source_document(source).to_text(),
        };
        let quoted = quote_of(source.body.html.as_deref(), &text, QUOTE_SCOPE);
        quoted_reply(&quoted, &postio_model::reply::attribution(source))
    };
    let (text, html) = crate::render(&rich);
    postio_model::MessageBody {
        text: Some(text),
        html: Some(html),
    }
}

pub use crate::parse::QUOTE_SCOPE;

/// The sender's own text alternative, else the flowed-aware narrowing, so `format=flowed` is not
/// quoted back with breaks nobody typed (#456).
pub fn forward_text(source: &postio_model::Message) -> String {
    match source.body.html {
        Some(_) => source.body.text.clone().unwrap_or_default(),
        None => source_document(source).to_text(),
    }
}

/// The markup the reader showed, else the plain text. The fallback unwraps only when `source`
/// declared `format=flowed` (#456): always unwrapping joins short lines, never unwrapping shows
/// wrapped sentences as breaks.
pub fn source_document(source: &postio_model::Message) -> Document {
    match (&source.body.html, &source.body.text) {
        (Some(html), _) => crate::parse(html),
        (None, Some(text)) if source.text_is_flowed => Document::from_flowed_text(text),
        (None, Some(text)) => Document::from_text(text),
        (None, None) => Document::new(),
    }
}
