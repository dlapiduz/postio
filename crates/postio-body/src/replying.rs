//! The documents a reply and a forward start from: ADR 0003 Q3's inversion.
//!
//! `postio_model::reply` computes recipients, subjects and threading; it
//! cannot compute a quote, because quoting means parsing untrusted markup
//! and the parser lives here, *above* the model. So the quote is built here
//! from the already-parsed [`Document`] and handed down — which is also the
//! security property (hardening requirement 6): a reply re-emits quoted
//! content into the world, and building it from the closed type means a
//! script or a tracking pixel has no representation rather than being
//! stripped on the way out.

use crate::document::{Block, Document, Inline};

/// What a plain separator line says. Mirrors
/// `postio_model::signature::SEPARATOR`, spelled here because the model sits
/// below this crate and a signature is a convention of the wire, not of any
/// one crate.
const SEPARATOR: &str = "--";

/// The document a reply starts from: a blank line for the caret, the
/// attribution, and `source` as a quote.
///
/// The caret line is a [`Inline::Break`] paragraph, not an empty one —
/// `parse` narrows a truly empty paragraph away on the first round trip
/// through the editor, and the caret needs its place above the quote to
/// survive that.
pub fn quoted_reply(source: &Document, attribution: &str) -> Document {
    let mut blocks = vec![
        Block::Paragraph(vec![Inline::Break]),
        Block::Paragraph(vec![Inline::Text(attribution.to_owned())]),
    ];
    if !source.is_empty() {
        blocks.push(Block::Quote(source.blocks.clone()));
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

/// Puts `signature` at the end of `document`, replacing any signature
/// already there — `postio_model::signature::apply`, at the block level, so
/// applying one to a rich body never flattens it.
///
/// The separator is a top-level paragraph saying `--` alone; the *last* one
/// wins, and one with a [`Block::Quote`] after it is not a separator at all,
/// both mirroring the text form's rules for the same reasons: a quoted
/// message's own signature is somebody else's, and rewriting it would edit
/// the quote.
pub fn apply_signature(document: &Document, signature: Option<&str>) -> Document {
    let mut separator: Option<usize> = None;
    for (index, block) in document.blocks.iter().enumerate() {
        if is_separator(block) {
            separator = Some(index);
        } else if separator.is_some() && matches!(block, Block::Quote(_)) {
            separator = None;
        }
    }

    let written = match separator {
        Some(index) => &document.blocks[..index],
        None => &document.blocks[..],
    };
    let mut blocks: Vec<Block> = written.to_vec();

    if let Some(signature) = signature.map(str::trim_end).filter(|text| !text.is_empty()) {
        blocks.push(Block::Paragraph(vec![Inline::Text(format!(
            "{SEPARATOR} "
        ))]));
        blocks.extend(Document::from_text(signature).blocks);
    }
    Document { blocks }
}

/// Whether `block` is the signature separator: a paragraph of exactly `--`,
/// give or take the traditional trailing space.
fn is_separator(block: &Block) -> bool {
    let Block::Paragraph(inlines) = block else {
        return false;
    };
    let [Inline::Text(text)] = inlines.as_slice() else {
        return false;
    };
    text.trim_end() == SEPARATOR && !text.starts_with(' ')
}
