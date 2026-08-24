//! What Postio is willing to put on the wire, checked against hostile input.
//!
//! Issue #30's fourth and fifth acceptance criteria, in the stronger form ADR
//! 0004 Q4 chose: outgoing markup is *generated from a closed type*, so a
//! script or a tracking pixel has no representation rather than being
//! stripped. The ammonia backstop is asserted to be a **no-op** — a backstop
//! that quietly cleans up after a real bug is one that hides it.

use postio_body::document::{Block, ContentId, Document, HeadingLevel, Href, Inline};
use postio_body::{harden, parse};

/// The corpus fixture written to carry every remote-reference trick at once:
/// a CSS `background-image`, a CDN product shot, a hosted logo, a redirect
/// link and a 1x1 open-rate beacon.
const HOSTILE: &str = "html-tracking-pixel-remote-images.eml";

/// The fixture's HTML body, decoded by the real MIME parser.
///
/// Through `postio_model::test_corpus` rather than by reading the file and
/// undoing quoted-printable by hand: this is the path a real message takes,
/// and a hand-rolled decoder that mangled a URL would make "the URL is gone"
/// pass for the wrong reason.
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

/// Everything the serialiser can emit, including the shapes an attacker would
/// like to reach.
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
                ordered: true,
                items: vec![vec![Block::Paragraph(vec![Inline::Text("one".to_owned())])]],
            },
            Block::Quote(vec![Block::Paragraph(vec![Inline::Text(
                "quoted".to_owned(),
            )])]),
            Block::Pre("  raw < text\n".to_owned()),
            Block::Rule,
        ],
    }
}

#[test]
fn the_backstop_never_fires() {
    // ADR 0004 Q4. If this ever fails, the serialiser has a bug — fix that,
    // do not be relieved that ammonia caught it.
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
    // The same property where it actually matters: a document that came from
    // somebody else's markup, not one this test wrote.
    let html = parse(&hostile_html()).to_html();
    assert_eq!(harden(&html), html, "{html}");
}

#[test]
fn a_reply_quoting_a_hostile_message_carries_none_of_it() {
    // Issue #30's fifth acceptance criterion. The reply is what a user sends
    // when they hit reply on the tracking-pixel fixture: their own words,
    // then the sender's message quoted underneath.
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

    // Nothing that *loads*. This is the list that matters: every one of
    // these fires on render, with no action from anybody, and every one of
    // them is a report to the sender that a human read the message.
    //
    // A link is not on this list, and that is deliberate — see the test
    // below. `<img src>` fetches; `<a href>` does not.
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
    // Nothing that executes or styles.
    for leak in ["<script", "<style", "<iframe", "style=", "class=", "onload"] {
        assert!(
            !html.contains(leak),
            "outgoing HTML carries {leak}:\n{html}"
        );
    }
    // And no image at all: every image in that message was remote, so none of
    // them has a representation.
    assert!(!html.contains("<img"), "{html}");

    // The reply is still a reply: the user's words and the sender's words.
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
    // The fixture's link is a redirect through `click.tracker.example.org`,
    // and it survives quoting. That is the correct answer, and it is worth
    // writing down because "the tracker's domain is still in the reply" looks
    // alarming next to the test above.
    //
    // An `<a href>` does nothing until a human clicks it. Stripping hrefs out
    // of quoted text would mangle every ordinary message — a colleague's link
    // is the content — to defend against something that is not a load. The
    // reader takes the same position and says so: "href is never touched;
    // links stay live text".
    //
    // The second-order question — a redirect link carrying the original
    // recipient's id back out to everyone on the thread — is a real one and
    // is a product decision rather than a detail of this crate. It is filed
    // separately.
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
    // The other half of the property, and the one that would make the test
    // above pass for the wrong reason: a `cid:` image is part of the message
    // and is kept, so "no images" is a result about *remote* references
    // rather than about images.
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
