//! The documents a reply and a forward start from: ADR 0003 Q3, as ADR 0033
//! amended it.
//!
//! `postio_model::reply` computes recipients, subjects and threading; it
//! cannot compute a quote, because quoting means parsing untrusted markup
//! and the parser lives here, *above* the model. So the quote is built here
//! and handed down.
//!
//! # What ADR 0033 changed, and what it did not
//!
//! It used to be built from the already-parsed [`Document`], the closed
//! authoring type, and the argument was that a script or a tracking pixel
//! then has *no representation* rather than being stripped on the way out.
//! The cost was fidelity: a table, a colour, a class had no representation
//! either, so a reply to a rich message quoted something that did not look
//! like the message being answered. ADR 0033 decided fidelity wins.
//!
//! The argument survives; only the representation moved. [`Quoted`] is still
//! a gate — it cannot be constructed except through [`quote_of`], which runs
//! the reader's own sanitiser with remote images blocked. What changed is
//! *what survives* sanitising: a table now does, a script still does not, and
//! the permitted set is [`crate::sanitize`]'s, not a second one written here.
//! A hole in that gate is a hole in the reader too, which is the point: one
//! policy, one place, tested once.

use postio_model::account::Signature;

use crate::document::{Block, Document, Inline};
use crate::sanitize::{RemoteImages, sanitize_body_in};

/// What a plain separator line says. Mirrors
/// `postio_model::signature::SEPARATOR`, spelled here because the model sits
/// below this crate and a signature is a convention of the wire, not of any
/// one crate.
const SEPARATOR: &str = "--";

/// A quote: the original as the reader would render it.
///
/// Opaque on purpose. The fields are private and the only constructor is
/// [`quote_of`], so a `Quoted` in hand is markup that has been through
/// [`crate::sanitize`] with remote images blocked — there is no path that
/// puts a sender's raw bytes in one. That is ADR 0003 Q3's guarantee kept
/// under ADR 0033's representation: the type is the gate, and what it admits
/// is the reader's policy rather than a second policy written for replies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Quoted {
    html: String,
    styles: String,
    text: String,
}

impl Quoted {
    /// The sanitised markup, ready to sit inside the reply's `<blockquote>`.
    pub fn html(&self) -> &str {
        &self.html
    }

    /// The sender's stylesheet, scoped so it cannot match outside this quote.
    ///
    /// Separate from [`Self::html`] for the reason [`crate::sanitize::Sanitized::styles`]
    /// gives: what is emitted is CSS Postio parsed and rewrote, never the
    /// sender's own `<style>` element passed through.
    pub fn styles(&self) -> &str {
        &self.styles
    }

    /// The plain-text rendering, for the `text/plain` half of the reply.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether there is anything to quote at all.
    pub fn is_empty(&self) -> bool {
        self.html.trim().is_empty() && self.text.trim().is_empty()
    }
}

/// Builds the quote of a message whose HTML is `html` and whose plain-text
/// alternative is `text`, scoped under `scope`.
///
/// `html` is `None` for a `text/plain`-only original — and the fallback is
/// used for a *second* case that a naive `is_some` check gets wrong: an HTML
/// part every byte of which sanitises away. Either way the result is the text
/// alternative rather than an empty quote (FR-045), because a reply that
/// opens with an attribution and an empty box is a silent failure.
///
/// Remote images are always [`RemoteImages::Blocked`] here, whatever the
/// reader was allowed to show (ADR 0033 Q2). Allowing a sender's remote
/// images is a decision made on this machine about this mailbox; re-emitting
/// them would carry it to every recipient of the reply, and hand the sender a
/// beacon that fires in other people's clients.
pub fn quote_of(html: Option<&str>, text: &str, scope: &str) -> Quoted {
    // Nothing to quote at all: a reply to a message whose body never arrived,
    // most often. Returned before the fallback runs, because
    // `Document::from_text("")` still renders an empty paragraph and a quote
    // holding one says something was quoted when nothing was.
    if html.is_none_or(|html| html.trim().is_empty()) && text.trim().is_empty() {
        return Quoted::default();
    }
    if let Some(html) = html.filter(|html| !html.trim().is_empty()) {
        let sanitized = sanitize_body_in(html, RemoteImages::Blocked, Some(scope));
        if !sanitized.html.trim().is_empty() {
            // An HTML-only message has no text alternative to carry, and a
            // reply to one must still have a `text/plain` half -- otherwise
            // the quote is an attribution followed by a bare `> `. Narrowing
            // the sanitised markup is exactly the right source for it: the
            // text part is a reduction by definition, and this is the same
            // reduction the reader would show with markup turned off.
            let text = if text.trim().is_empty() {
                crate::parse::parse(&sanitized.html).to_text()
            } else {
                text.to_owned()
            };
            return Quoted {
                html: sanitized.html,
                styles: sanitized.styles,
                text,
            };
        }
    }
    // The fallback, through `Document::from_text` rather than one `<p>` around
    // the lot: a sender's own line break is content (#456), and wrapping the
    // whole text in a single paragraph turns every one of them into the space
    // HTML collapses it to. `from_text` gives breaks and blank-line paragraph
    // splits their markup, and the result still goes through the sanitiser --
    // one gate, and this is someone else's text.
    let escaped = sanitize_body_in(
        &Document::from_text(text).to_html(),
        RemoteImages::Blocked,
        Some(scope),
    );
    Quoted {
        html: escaped.html,
        styles: String::new(),
        text: text.to_owned(),
    }
}

/// The document a reply starts from: a blank line for the caret, the
/// attribution, and `source` as a quote.
///
/// The caret line is a [`Inline::Break`] paragraph, not an empty one —
/// `parse` narrows a truly empty paragraph away on the first round trip
/// through the editor, and the caret needs its place above the quote to
/// survive that.
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

/// The document a forward starts from: a blank line for the caret, the
/// conventional header block as one paragraph of `header_lines`, then
/// `source` as itself — a forward presents the whole message rather than
/// answering a fragment of it, so nothing is wrapped in a quote.
pub fn forwarded(source: &Document, header_lines: &[String]) -> Document {
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
    blocks.extend(source.blocks.iter().cloned());
    Document { blocks }
}

/// Where a signature sits relative to the quoted message (#12).
///
/// Not a style opinion the code holds — both conventions are in wide use, and
/// which one is right is the user's answer per draft kind. It is here because
/// *moving* a signature has to remove the old one, which only the function
/// that knows how to find one can do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Placement {
    /// Under everything, quote included — the Usenet convention, and what a
    /// bottom-posting reply wants.
    #[default]
    BelowQuote,
    /// Under what was written and above the quote — where a top-posting
    /// client puts it, and what most mail now looks like.
    AboveQuote,
}

/// Puts `signature` into `document` at `placement`, replacing any signature
/// already there — `postio_model::signature::apply`, at the block level, so
/// applying one to a rich body never flattens it.
///
/// The separator is a top-level paragraph saying `--` alone; the *last* one
/// wins, and one with a [`Block::Quote`] after it is not a separator at all,
/// both mirroring the text form's rules for the same reasons: a quoted
/// message's own signature is somebody else's, and rewriting it would edit
/// the quote. Above-quote placement is the one case where a separator *does*
/// have a quote after it and is still this draft's own, so it is found by
/// position rather than by that rule — see [`existing_signature`].
///
/// The rich variant is used when the signature has one: the composer's body
/// is a document, and a signature with markup that arrived as flattened text
/// would be the one part of a message the user could not format.
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

    // Somewhere to type. Without it the caret opens *inside* the separator
    // paragraph on a draft that has nothing else in it, and the first word
    // typed lands in front of the `-- `.
    if blocks.is_empty() {
        // Empty rather than a hard break: this is the spelling the plain-text
        // pipeline has always produced — one blank line, then the separator —
        // and a draft that has not been typed into yet should not change what
        // goes on the wire.
        blocks.push(Block::Paragraph(Vec::new()));
    }

    let at = match placement {
        Placement::BelowQuote => blocks.len(),
        // Immediately before the first quote, so it lands under what was
        // written; with nothing quoted the two placements agree.
        Placement::AboveQuote => blocks
            .iter()
            .position(|block| matches!(block, Block::Quote(_) | Block::Quoted(_)))
            .unwrap_or(blocks.len()),
    };
    blocks.splice(at..at, inserted);
    Document { blocks }
}

/// `blocks` with the RFC 3676 separator on the line directly above them.
///
/// Directly above, in the same paragraph, is the whole point: the convention
/// every client folds on is a line of exactly `-- ` *immediately* before the
/// signature, and a separator in a paragraph of its own renders with a blank
/// line under it — which stops other clients recognising it at all.
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
        // A signature that opens with something other than a paragraph — a
        // list, a rule — keeps the separator as its own line above it.
        Some(other) => std::iter::once(Block::Paragraph(vec![separator]))
            .chain(std::iter::once(other))
            .chain(blocks)
            .collect(),
        None => vec![Block::Paragraph(vec![separator])],
    }
}

/// The signature's blocks: the rich variant when there is one, else the text.
fn signature_blocks(signature: &Signature) -> Vec<Block> {
    match signature.html.as_deref().filter(|html| !html.is_empty()) {
        // Through `parse`, like anything else that arrives as markup — the
        // subset is what can be represented, so a signature cannot smuggle in
        // what a message body could not.
        Some(html) => crate::parse(html).blocks,
        None => Document::from_text(signature.text.trim_end()).blocks,
    }
}

/// The span this draft's own signature occupies, separator included.
///
/// Runs from the separator to the end of the document, or to the quote that
/// follows it when the signature was placed above one. A separator with quoted
/// material after it *and* no later separator is somebody else's signature
/// inside the quote, which is why the search skips a separator that is itself
/// inside a [`Block::Quote`] — those never appear at the top level.
fn existing_signature(blocks: &[Block]) -> Option<std::ops::Range<usize>> {
    let separator = blocks.iter().rposition(is_separator)?;
    let end = blocks[separator..]
        .iter()
        .position(|block| matches!(block, Block::Quote(_) | Block::Quoted(_)))
        .map(|offset| separator + offset)
        .unwrap_or(blocks.len());
    Some(separator..end)
}

/// Whether `block` opens the signature: a top-level paragraph whose first
/// line is exactly `--`, give or take the traditional trailing space.
///
/// "Opens" rather than "is", because [`separated`] puts the separator and the
/// signature in one paragraph so they render on adjacent lines — so what
/// marks the boundary is the paragraph's first line, not the whole of it.
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
    // Either the paragraph is the separator alone, or the separator is its
    // first line — anything else is a paragraph that merely begins with two
    // hyphens.
    matches!(inlines.get(1), None | Some(Inline::Break))
}
