//! The composer's Markdown source view, as text in and text out (spec 014
//! US7, `contracts/source-view.md`).
//!
//! The draft is still a document (ADR 0003): the view shows it as
//! `postio_body::markdown::from_document` writes it and reads it back with
//! `to_document`, which lose nothing (FR-024). What has no Markdown -- a
//! reply's carried quote, `Block::Quoted` -- is one line holding
//! [`QUOTE_LINE`] and the chip's words, at its place among the blocks, and
//! is put back from that line on return (FR-025). The caret crosses through
//! the source maps (FR-027). Each frontend only draws the text.

use postio_body::markdown::{self, DocPosition, Lost};
use postio_body::{Block, Document};

/// The private character that starts a carried quote's line in the view.
/// Stripped from what is typed and pasted, so the line cannot be forged.
pub const QUOTE_LINE: char = '\u{E001}';

/// What a carried quote's line reads as.
pub const QUOTE_CHIP: &str = "\u{25b8} Quoted message \u{b7} kept as sent";

/// The text the view opens with, and where its caret goes (a byte offset).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    /// The Markdown, with a line per carried quote.
    pub text: String,
    /// The caret, as a byte offset into `text`.
    pub caret: Option<usize>,
    /// The carried quotes, in order: what each quote line stands for.
    pub quotes: Vec<Block>,
}

/// What the view's text returns as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Returned {
    /// The draft's document.
    pub document: Document,
    /// The caret, as a place in it.
    pub caret: Option<DocPosition>,
    /// What the typed Markdown said that the document narrowed away.
    pub lost: Lost,
}

/// The whole line a carried quote stands on in the view.
pub fn quote_line() -> String {
    format!("{QUOTE_LINE}{QUOTE_CHIP}")
}

/// `document` as the view's text, with the caret at `caret`'s character.
pub fn open(document: &Document, caret: Option<&DocPosition>) -> Opened {
    let mut text = String::new();
    let mut at = None;
    let mut quotes = Vec::new();
    let mut run: Vec<Block> = Vec::new();
    let mut run_start = 0;
    let flush =
        |text: &mut String, run: &mut Vec<Block>, run_start: usize, at: &mut Option<usize>| {
            if run.is_empty() {
                return;
            }
            let segment = Document {
                blocks: std::mem::take(run),
            };
            let (written, map) = markdown::from_document_mapped(&segment);
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            if let Some(caret) = caret
                && let Some((first, rest)) = caret.path.split_first()
                && (run_start..run_start + segment.blocks.len()).contains(first)
            {
                let mut path = vec![first - run_start];
                path.extend_from_slice(rest);
                *at = Some(
                    text.len()
                        + map.to_source(&DocPosition {
                            path,
                            offset: caret.offset,
                        }),
                );
            }
            text.push_str(written.trim_end_matches('\n'));
        };
    for (index, block) in document.blocks.iter().enumerate() {
        if matches!(block, Block::Quoted(_)) {
            flush(&mut text, &mut run, run_start, &mut at);
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&quote_line());
            quotes.push(block.clone());
            run_start = index + 1;
        } else {
            if run.is_empty() {
                run_start = index;
            }
            run.push(block.clone());
        }
    }
    flush(&mut text, &mut run, run_start, &mut at);
    let caret = caret.map(|_| at.unwrap_or(0));
    Opened {
        text,
        caret,
        quotes,
    }
}

/// The view's `text` as the draft's document again, each quote line put
/// back as the carried quote it stood for, and the caret at `caret`'s
/// character: inside a marker, the text it formats; on a quote's line, the
/// start of the block after it.
pub fn close(text: &str, quotes: &[Block], caret: Option<usize>) -> Returned {
    let mut blocks = Vec::new();
    let mut lost = Lost::default();
    let mut found: Option<DocPosition> = None;
    // One run of Markdown between quote lines, read into the blocks.
    let read = |segment: &str,
                start: usize,
                blocks: &mut Vec<Block>,
                found: &mut Option<DocPosition>,
                lost: &mut Lost| {
        if segment.trim().is_empty() {
            return;
        }
        let (document, map) = markdown::to_document_mapped(segment);
        if let Some(caret) = caret
            && found.is_none()
            && (start..=start + segment.len()).contains(&caret)
        {
            let mut at = map.to_document(caret - start);
            if let Some(first) = at.path.first_mut() {
                *first += blocks.len();
            }
            *found = Some(at);
        }
        let more = map.lost();
        lost.cells += more.cells;
        lost.links += more.links;
        lost.images += more.images;
        blocks.extend(document.blocks);
    };
    let mut quotes = quotes.iter();
    let mut segment = String::new();
    let mut segment_start = 0;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if line.starts_with(QUOTE_LINE) {
            read(&segment, segment_start, &mut blocks, &mut found, &mut lost);
            segment.clear();
            if let Some(quote) = quotes.next() {
                if found.is_none()
                    && caret.is_some_and(|caret| (at..at + line.len()).contains(&caret))
                {
                    // On the quote's line: the start of the block after it.
                    found = Some(DocPosition {
                        path: vec![blocks.len() + 1],
                        offset: 0,
                    });
                }
                blocks.push(quote.clone());
            }
            segment_start = at + line.len();
        } else {
            if segment.is_empty() {
                segment_start = at;
            }
            segment.push_str(line);
        }
        at += line.len();
    }
    read(&segment, segment_start, &mut blocks, &mut found, &mut lost);
    // A quote with nothing after it: no text to put the caret in, so the
    // editor's own default holds.
    if found.as_ref().is_some_and(|position| {
        position
            .path
            .first()
            .is_none_or(|first| *first >= blocks.len())
    }) {
        found = None;
    }
    Returned {
        document: Document { blocks },
        caret: found,
        lost,
    }
}

/// The view's text without its quote lines: the person's Markdown, as a
/// draft keeps it beside the HTML (spec 014 FR-028).
pub fn markdown_of(text: &str) -> String {
    let kept: Vec<&str> = text
        .split('\n')
        .filter(|line| !line.starts_with(QUOTE_LINE))
        .collect();
    kept.join("\n").trim_end().to_owned()
}

/// `text` without the quote line's private character: what typing and
/// pasting into the view may insert.
pub fn strip_quote_marks(text: &str) -> String {
    text.chars().filter(|c| *c != QUOTE_LINE).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quoted() -> Block {
        Block::Quoted(postio_body::quote_of(
            Some("<p>they wrote</p>"),
            "they wrote",
            "quote",
        ))
    }

    fn reply() -> Document {
        let mut document = markdown::to_document("Thanks, ~~Tuesday~~ Wednesday.");
        document.blocks.push(quoted());
        document
    }

    #[test]
    fn a_reply_opens_as_its_words_then_the_quote_line() {
        let opened = open(&reply(), None);
        assert_eq!(
            opened.text,
            format!("Thanks, ~~Tuesday~~ Wednesday.\n\n{}", quote_line())
        );
        assert_eq!(opened.quotes, vec![quoted()]);
    }

    #[test]
    fn opening_and_closing_changes_nothing() {
        let document = reply();
        let opened = open(&document, None);
        let returned = close(&opened.text, &opened.quotes, None);
        assert_eq!(returned.document, document);
        assert!(returned.lost.is_empty());
    }

    #[test]
    fn the_caret_stays_on_its_character_both_ways() {
        let document = reply();
        // After "Tues" in the struck word: "Thanks, " is 8, "Tues" 4 more.
        let caret = DocPosition {
            path: vec![0],
            offset: 12,
        };
        let opened = open(&document, Some(&caret));
        let at = opened.caret.expect("a caret");
        assert_eq!(&opened.text[..at], "Thanks, ~~Tues");
        let returned = close(&opened.text, &opened.quotes, Some(at));
        assert_eq!(returned.caret, Some(caret));
    }

    #[test]
    fn the_caret_on_the_quote_line_goes_to_the_block_after_it() {
        let mut document = reply();
        document
            .blocks
            .push(Block::Paragraph(vec![postio_body::Inline::Text(
                "PS".into(),
            )]));
        let opened = open(&document, None);
        let line = opened.text.find(QUOTE_LINE).unwrap();
        let returned = close(&opened.text, &opened.quotes, Some(line + 2));
        assert_eq!(
            returned.caret,
            Some(DocPosition {
                path: vec![2],
                offset: 0
            })
        );
    }

    #[test]
    fn a_deleted_quote_line_takes_the_quote_with_it() {
        let returned = close("Just the words.", &[quoted()], None);
        assert_eq!(returned.document, markdown::to_document("Just the words."));
    }

    #[test]
    fn a_ragged_row_is_reported() {
        let returned = close("| a |\n|---|\n| b | c |", &[], None);
        assert_eq!(returned.lost.cells, 1);
    }

    #[test]
    fn the_quote_mark_cannot_be_typed() {
        assert_eq!(strip_quote_marks("a\u{E001}b"), "ab");
    }
}
