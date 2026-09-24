//! The terminal composer's HTML is the desktop composer's (SC-006).
//!
//! Both build a `postio_body::Document` and send `render` of it, so the
//! question is whether a document written in Markdown is one the desktop
//! composer would itself hold. The desktop composer loads a draft's HTML with
//! `parse`; so for every construct Markdown and the document share, the HTML
//! the terminal sends has to parse back to the very document it came from --
//! otherwise the same draft would change shape the moment the desktop opened
//! it, and the two composers would disagree about what was sent.

use postio_body::markdown::to_document;
use postio_body::{parse, render};

const CONSTRUCTS: &[&str] = &[
    "Plain words.",
    "Some **bold** words.",
    "Some *italic* words.",
    "Some `code`.",
    "A [link](https://example.com/a?b=c) here.",
    "A [mail link](mailto:ada@example.com).",
    "# A heading",
    "## A second",
    "### A third",
    "- one\n- two",
    "1. first\n2. second",
    "- outer\n  - inner",
    "> a quote",
    "```\nlet x = 1;\n```",
    "---",
    "first line\\\nsecond line",
    "**bold with *italic* inside**",
];

#[test]
fn the_html_the_terminal_sends_is_html_the_desktop_would_hold() {
    for markdown in CONSTRUCTS {
        let document = to_document(markdown);
        let html = render(&document).1;
        assert_eq!(
            parse(&html),
            document,
            "{markdown:?} rendered {html:?}, which the desktop composer reads as something else"
        );
    }
}

#[test]
fn a_sentence_typed_in_either_composer_sends_the_same_html() {
    use postio_body::{Block, Document, Href, Inline};
    // What the desktop composer holds after typing the sentence and pressing
    // its bold and link buttons.
    let desktop = Document {
        blocks: vec![Block::Paragraph(vec![
            Inline::Strong(vec![Inline::Text("bold".into())]),
            Inline::Text(" and a ".into()),
            Inline::Link {
                href: Href::parse("https://example.com").unwrap(),
                inlines: vec![Inline::Text("link".into())],
            },
        ])],
    };
    let terminal = to_document("**bold** and a [link](https://example.com)");
    assert_eq!(render(&terminal).1, render(&desktop).1);
}
