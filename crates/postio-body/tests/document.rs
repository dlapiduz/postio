//! The document type's two round trips, and the invariants underneath them.
//!
//! ADR 0004 pins four properties as tests rather than comments, because each
//! one is load-bearing for "Postio cannot send a tracking pixel" and a
//! comment cannot fail.

use postio_body::document::{Block, ContentId, Document, HeadingLevel, Href, Inline};
use postio_body::parse;

/// Every shape the document can take, in one value.
///
/// Deliberately exhaustive: the round-trip properties are only as good as the
/// documents they are checked over, and a corpus that omits a variant is a
/// corpus that cannot catch that variant's serialiser being wrong.
fn every_shape() -> Document {
    Document {
        blocks: vec![
            Block::Paragraph(vec![
                Inline::Text("plain, with ".to_owned()),
                Inline::Strong(vec![Inline::Text("bold".to_owned())]),
                Inline::Text(" and ".to_owned()),
                Inline::Emphasis(vec![Inline::Text("italic".to_owned())]),
                Inline::Break,
                Inline::Code("let x = 1;".to_owned()),
                Inline::Link {
                    href: Href::parse("https://example.com/a?b=c&d=e").unwrap(),
                    inlines: vec![Inline::Text("a link".to_owned())],
                },
                Inline::Image {
                    content_id: ContentId::parse("part1@example.com").unwrap(),
                    alt: "a picture".to_owned(),
                },
            ]),
            Block::Heading {
                level: HeadingLevel::One,
                inlines: vec![Inline::Text("one".to_owned())],
            },
            Block::Heading {
                level: HeadingLevel::Two,
                inlines: vec![Inline::Text("two".to_owned())],
            },
            Block::Heading {
                level: HeadingLevel::Three,
                inlines: vec![Inline::Text("three".to_owned())],
            },
            Block::List {
                ordered: false,
                items: vec![
                    vec![Block::Paragraph(vec![Inline::Text("first".to_owned())])],
                    vec![
                        Block::Paragraph(vec![Inline::Text("second".to_owned())]),
                        Block::List {
                            ordered: true,
                            items: vec![vec![Block::Paragraph(vec![Inline::Text(
                                "nested".to_owned(),
                            )])]],
                        },
                    ],
                ],
            },
            Block::Quote(vec![
                Block::Paragraph(vec![Inline::Text("they wrote this".to_owned())]),
                Block::Quote(vec![Block::Paragraph(vec![Inline::Text(
                    "and quoted this".to_owned(),
                )])]),
            ]),
            Block::Pre("  indented\n    further\n".to_owned()),
            Block::Rule,
        ],
    }
}

#[test]
fn structure_survives_the_round_trip() {
    // Issue #30's third acceptance criterion.
    let document = every_shape();
    let round_tripped = parse(&document.to_html());
    assert_eq!(
        round_tripped, document,
        "document -> HTML -> document lost or changed structure"
    );
}

#[test]
fn the_serialiser_is_the_normal_form() {
    // `to_html(parse(h)) == h` for anything already in the subset. This is
    // what makes re-saving a draft a no-op rather than a slow rewrite of the
    // user's own markup — and what stops two frontends drifting into two
    // different spellings of the same document.
    let html = every_shape().to_html();
    assert_eq!(parse(&html).to_html(), html, "to_html is not idempotent");
}

#[test]
fn a_script_has_no_representation() {
    let document = parse(
        "<p>before</p><script>alert('pwned')</script><p>after</p>\
         <p onclick=\"steal()\">handler</p>",
    );

    let html = document.to_html();
    assert!(!html.contains("script"), "{html}");
    assert!(!html.contains("alert"), "{html}");
    assert!(!html.contains("onclick"), "{html}");
    // The surrounding text is still the author's, and is kept.
    assert!(html.contains("before") && html.contains("after") && html.contains("handler"));
}

#[test]
fn a_remote_image_has_no_representation() {
    // The single most load-bearing line in the type: `Inline::Image` holds a
    // `ContentId`, so there is no variant that can carry a tracking pixel.
    // It is not stripped — it cannot be built.
    let document = parse(
        "<p><img src=\"https://tracker.example.com/pixel.gif\" width=\"1\">\
         <img src=\"http://tracker.example.net/p.png\">\
         <img src=\"data:image/gif;base64,R0lGOD\">\
         <img src=\"cid:real@example.com\" alt=\"kept\"></p>",
    );

    let images: Vec<_> = match &document.blocks[0] {
        Block::Paragraph(inlines) => inlines
            .iter()
            .filter(|i| matches!(i, Inline::Image { .. }))
            .collect(),
        other => panic!("expected a paragraph, got {other:?}"),
    };
    assert_eq!(images.len(), 1, "only the cid: image survives: {images:?}");

    let html = document.to_html();
    assert!(!html.contains("tracker.example"), "{html}");
    assert!(!html.contains("data:"), "{html}");
    assert!(html.contains("cid:real@example.com"), "{html}");
}

#[test]
fn only_three_schemes_construct_an_href() {
    for good in [
        "https://example.com",
        "http://example.com/x",
        "mailto:ada@example.com",
        "HTTPS://EXAMPLE.COM",
    ] {
        assert!(Href::parse(good).is_some(), "{good} should be a link");
    }
    for bad in [
        "javascript:alert(1)",
        "JaVaScRiPt:alert(1)",
        "data:text/html;base64,PHNjcmlwdD4=",
        "vbscript:msgbox",
        "file:///etc/passwd",
        "/relative/path",
        "#anchor",
        "",
        "https:",
        // The classic bypass: an entity-decoded control character inside the
        // scheme. Refusing controls outright means the check cannot be walked
        // around by choosing where the decode happens.
        "java\tscript:alert(1)",
        "java\nscript:alert(1)",
    ] {
        assert!(Href::parse(bad).is_none(), "{bad:?} must not be a link");
    }
}

#[test]
fn a_javascript_link_loses_the_href_and_keeps_the_words() {
    let document = parse("<p><a href=\"javascript:alert(1)\">click me</a></p>");
    let html = document.to_html();

    assert!(!html.contains("javascript"), "{html}");
    assert!(
        !html.contains("<a "),
        "a refused href must not emit a link: {html}"
    );
    assert!(
        html.contains("click me"),
        "the author's words are not the attack: {html}"
    );
}

#[test]
fn a_content_id_cannot_be_a_url() {
    for bad in [
        "https://evil.example/p.gif",
        "//evil.example",
        "a b",
        "",
        "<>",
        "with\tcontrol",
    ] {
        assert!(ContentId::parse(bad).is_none(), "{bad:?}");
    }
    assert_eq!(
        ContentId::parse("<part1@example.com>").unwrap().as_str(),
        "part1@example.com",
        "MIME writes the brackets; the id is what is inside them"
    );
}

#[test]
fn styling_does_not_survive() {
    let document = parse(
        "<p style=\"color:red\" class=\"x\"><font color=\"red\">red</font> \
         <span style=\"display:none\">hidden</span></p>",
    );
    let html = document.to_html();

    for banned in ["style", "class", "color", "font", "span"] {
        assert!(!html.contains(banned), "{banned} survived: {html}");
    }
    assert!(html.contains("red") && html.contains("hidden"));
}

#[test]
fn a_plain_text_document_is_just_paragraphs() {
    // ADR 0004 Q6: v1 ships a plain-text editor over the neutral model. A
    // document of Paragraph/Text *is* a plain-text document, so the model
    // needs no restricting — the editor does.
    let document = Document::from_text("first line\nsame paragraph\n\nsecond paragraph\n");

    assert_eq!(
        document,
        Document {
            blocks: vec![
                Block::Paragraph(vec![
                    Inline::Text("first line".to_owned()),
                    Inline::Break,
                    Inline::Text("same paragraph".to_owned()),
                ]),
                Block::Paragraph(vec![Inline::Text("second paragraph".to_owned())]),
            ],
        }
    );
    assert_eq!(
        document.to_text(),
        "first line\nsame paragraph\n\nsecond paragraph"
    );
    assert_eq!(
        document.to_html(),
        "<p>first line<br>same paragraph</p><p>second paragraph</p>"
    );
}

#[test]
fn to_text_is_readable_rather_than_a_tag_soup() {
    // The reason the set is closed: `to_text` over arbitrary HTML is what
    // makes most mail's text/plain part unreadable.
    let text = every_shape().to_text();

    assert!(text.contains("plain, with bold and italic"));
    assert!(
        text.contains("a link <https://example.com/a?b=c&d=e>"),
        "a link's address is the point of it in plain text: {text}"
    );
    assert!(text.contains("[image: a picture]"));
    assert!(text.contains("- first"), "{text}");
    assert!(text.contains("1. nested"), "{text}");
    assert!(
        text.contains("> they wrote this"),
        "a quote is quoted the way every mail client writes one: {text}"
    );
}

#[test]
fn text_escaping_survives_a_round_trip() {
    let document = Document {
        blocks: vec![Block::Paragraph(vec![Inline::Text(
            "5 < 6 && \"quoted\" & <b>not bold</b>".to_owned(),
        )])],
    };

    let html = document.to_html();
    assert!(!html.contains("<b>"), "text became markup: {html}");
    assert_eq!(parse(&html), document);
}

#[test]
fn parsing_never_panics_on_anything() {
    // Total, because the quoting path's input is a message somebody else
    // wrote. A parse that could fail would be a hostile message you cannot
    // reply to.
    for hostile in [
        "",
        "<",
        "<<<<<<",
        "<p><p><p><p>",
        "</p></div></html>",
        "<a href=",
        "<!-- unterminated",
        "<![CDATA[x]]>",
        "&#x0;&#xFFFF;&notanentity;",
        "<table><tr><td>cell</td></tr></table>",
        "<p>\u{0}\u{feff}text</p>",
    ] {
        let _ = parse(hostile).to_html();
        let _ = parse(hostile).to_text();
    }
}
