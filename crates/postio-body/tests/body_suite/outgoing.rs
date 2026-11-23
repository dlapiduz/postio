use postio_body::document::{
    Alignment, Block, ContentId, Document, HeadingLevel, Href, Inline, Language, ListItem,
    ListKind, Table, Task, WebHref,
};
use postio_body::{harden, parse};

const HOSTILE: &str = "html-tracking-pixel-remote-images.eml";

/// Through `postio_model::test_corpus`, the path a real message takes, not a hand-rolled decoder.
fn hostile_html() -> String {
    let fixture = postio_model::test_corpus::get(HOSTILE)
        .unwrap_or_else(|| panic!("{HOSTILE} is not in the corpus"));
    let html = fixture
        .parse()
        .body
        .html
        .expect("the fixture is a text/html message");
    assert!(
        html.contains("pixel.tracker.example.org"),
        "the fixture arrived without its beacon, so this test cannot fail"
    );
    html
}

fn every_shape() -> Document {
    Document {
        blocks: vec![
            Block::Paragraph(vec![
                Inline::Text("text with < & > and \"quotes\"".to_owned()),
                Inline::Strong(vec![Inline::Text("bold".to_owned())]),
                Inline::Emphasis(vec![Inline::Text("italic".to_owned())]),
                Inline::Code("a < b && c".to_owned()),
                Inline::Break,
                Inline::Link {
                    href: Href::parse("https://example.com/path?a=1&b=2").unwrap(),
                    inlines: vec![Inline::Text("link".to_owned())],
                },
                Inline::Link {
                    href: Href::parse("mailto:ada@example.com").unwrap(),
                    inlines: vec![Inline::Text("write".to_owned())],
                },
                Inline::Image {
                    content_id: ContentId::parse("part1@example.com").unwrap(),
                    alt: "alt \"text\" & more".to_owned(),
                },
            ]),
            Block::Heading {
                level: HeadingLevel::Two,
                inlines: vec![Inline::Text("heading".to_owned())],
            },
            Block::List {
                kind: ListKind::NUMBERED,
                items: vec![ListItem::new(vec![Block::Paragraph(vec![Inline::Text(
                    "one".to_owned(),
                )])])],
            },
            Block::Quote(vec![Block::Paragraph(vec![Inline::Text(
                "quoted".to_owned(),
            )])]),
            Block::Code {
                language: None,
                text: "  raw < text\n".to_owned(),
            },
            Block::Rule,
            Block::Heading {
                level: HeadingLevel::Six,
                inlines: vec![Inline::Strikethrough(vec![Inline::Text("gone".to_owned())])],
            },
            Block::List {
                kind: ListKind::Ordered { start: 7 },
                items: vec![ListItem::new(vec![Block::Paragraph(vec![Inline::Text(
                    "seventh".to_owned(),
                )])])],
            },
            Block::List {
                kind: ListKind::Bullet,
                items: vec![
                    ListItem::task(
                        Task::Open,
                        vec![Block::Paragraph(vec![Inline::Text("open".to_owned())])],
                    ),
                    ListItem::task(
                        Task::Done,
                        vec![Block::Paragraph(vec![Inline::Text("done".to_owned())])],
                    ),
                ],
            },
            Block::Code {
                language: Language::parse("c++"),
                text: "int main() {}\n".to_owned(),
            },
            Block::Table(Table::new(
                vec![
                    Alignment::Left,
                    Alignment::Center,
                    Alignment::Right,
                    Alignment::None,
                ],
                vec![
                    vec![Inline::Text("a".to_owned())],
                    vec![Inline::Text("b".to_owned())],
                    vec![Inline::Text("c".to_owned())],
                    vec![Inline::Text("d".to_owned())],
                ],
                vec![vec![vec![
                    Inline::Text("1".to_owned()),
                    Inline::Break,
                    Inline::Text("2".to_owned()),
                ]]],
            )),
            Block::Paragraph(vec![Inline::RemoteImage {
                href: WebHref::parse("https://example.com/x.png?a=1&b=2").unwrap(),
                alt: "a \"chart\"".to_owned(),
            }]),
        ],
    }
}

#[test]
fn the_backstop_never_fires() {
    // If this fails the serialiser has a bug; fix that (ADR 0004 Q4).
    let html = every_shape().to_html();
    assert_eq!(
        harden(&html),
        html,
        "the outgoing sanitiser changed the serialiser's output, which means \
         the serialiser emitted something outside the subset"
    );
}

#[test]
fn the_backstop_never_fires_on_anything_parsed_from_the_hostile_corpus() {
    let html = parse(&hostile_html()).to_html();
    assert_eq!(harden(&html), html, "{html}");
}

#[test]
fn a_reply_quoting_a_hostile_message_carries_none_of_it() {
    let quoted = parse(&hostile_html());
    let reply = Document {
        blocks: [
            vec![Block::Paragraph(vec![Inline::Text(
                "Thanks, got it.".to_owned(),
            )])],
            vec![Block::Quote(quoted.blocks)],
        ]
        .concat(),
    };

    let (text, html) = postio_body::render(&reply);

    // Nothing that loads: each fires on render and tells the sender a human read the message. `<a
    // href>` does not load.
    for leak in [
        "cdn.tracker.example.org",
        "pixel.tracker.example.org",
        "images.tracker.example.org",
        "background-image",
        "o.gif",
        "logo.gif",
        "lamp-brass-441",
    ] {
        assert!(
            !html.contains(leak),
            "outgoing HTML carries {leak}:\n{html}"
        );
        assert!(
            !text.contains(leak),
            "outgoing text carries {leak}:\n{text}"
        );
    }
    for leak in ["<script", "<style", "<iframe", "style=", "class=", "onload"] {
        assert!(
            !html.contains(leak),
            "outgoing HTML carries {leak}:\n{html}"
        );
    }
    assert!(!html.contains("<img"), "{html}");

    assert!(html.contains("Thanks, got it."), "{html}");
    assert!(
        html.contains("has shipped"),
        "the quoted text is the point of quoting: {html}"
    );
    assert!(
        html.contains("<blockquote>"),
        "the quote has to read as a quote: {html}"
    );
}

#[test]
fn a_quoted_link_keeps_its_href_because_a_link_is_not_a_load() {
    // The tracker link survives quoting: an `<a href>` does nothing until clicked, and stripping
    // hrefs would mangle every message.
    let document = parse(&hostile_html());
    let html = document.to_html();
    let text = document.to_text();

    assert!(
        html.contains("click.tracker.example.org"),
        "the link was dropped, which is a behaviour change nobody asked for: {html}"
    );
    assert!(
        text.contains("Track your parcel"),
        "the author's words are not the attack: {text}"
    );
}

#[test]
fn an_inline_image_from_a_real_message_does_survive() {
    // A `cid:` image is part of the message and kept: "no images" is about remote references.
    let fixture = postio_model::test_corpus::get("inline-image-cid.eml")
        .expect("the inline-image fixture is in the corpus");
    assert!(
        fixture.text_lossy().contains("cid:"),
        "the fixture no longer carries an inline reference"
    );

    let document = parse("<p><img src=\"cid:logo@example.com\" alt=\"logo\"></p>");
    let html = document.to_html();
    assert!(html.contains("cid:logo@example.com"), "{html}");
    assert_eq!(
        harden(&html),
        html,
        "the backstop must not strip cid: {html}"
    );
}
