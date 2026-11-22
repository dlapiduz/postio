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
    "~~struck~~",
    "- [ ] open\n- [x] done",
    "7. seventh",
    "```rust\nfn x() {}\n```",
    "| a | b |\n|:--|--:|\n| 1 | 2 |",
    "![chart](https://example.com/c.png)",
    "#### Four\n\n##### Five\n\n###### Six",
    "see www.example.com",
];

#[test]
fn the_golden_markdown_reads_into_the_golden_document() {
    // spec 014 contracts/wire.md: the document is written there as the
    // Markdown that reads into it.
    let golden = postio_body::golden::golden();
    assert_eq!(to_document(postio_body::golden::GOLDEN_MARKDOWN), golden);
    assert_eq!(
        to_document(&postio_body::markdown::from_document(&golden)),
        golden,
        "the golden document does not survive being written as Markdown"
    );
}

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

#[test]
fn a_heading_made_in_either_composer_is_the_same_first_level_heading() {
    use postio_body::{Block, HeadingLevel};
    // #1485: the desktop's `heading_1` is WebKit's `formatBlock` to `<h1>`,
    // and the terminal's is `# ` -- the same command, so the same message.
    // Level one on both sides: a `# ` that sent `<h2>` would be a second
    // dialect of the one verb.
    let desktop = parse("<h1>Agenda</h1><p>First, the gate.</p>");
    let terminal = to_document("# Agenda\n\nFirst, the gate.");
    assert!(
        matches!(
            desktop.blocks.first(),
            Some(Block::Heading {
                level: HeadingLevel::One,
                ..
            })
        ),
        "the desktop's heading is not a first-level one: {desktop:?}"
    );
    assert_eq!(terminal, desktop);
    assert_eq!(render(&terminal).1, render(&desktop).1);
}
