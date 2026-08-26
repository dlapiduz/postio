//! The documents a reply and a forward start from: issue #340.
//!
//! ADR 0003 hardening requirement 6, in the direction that matters most: a
//! reply re-emits quoted markup into the world, so the quote is built from
//! the parsed [`Document`] — where a script or a tracking pixel has no
//! representation — never from the source's markup. Both directions are
//! checked against the hostile corpus fixture, through the real MIME parser,
//! exactly as `outgoing.rs` does for the hand-assembled shape.

use postio_body::document::{Block, Document, Inline};
use postio_body::{apply_signature, forwarded, parse, quoted_reply};

/// See `outgoing.rs`: every remote-reference trick at once.
const HOSTILE: &str = "html-tracking-pixel-remote-images.eml";

/// The fixture's HTML body through the real MIME parser, as `outgoing.rs`
/// reads it — the path a real message takes.
fn hostile_document() -> Document {
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
    parse(&html)
}

fn text(text: &str) -> Inline {
    Inline::Text(text.to_owned())
}

// ---------------------------------------------------------------------------
// Shape
// ---------------------------------------------------------------------------

#[test]
fn a_quoted_reply_is_a_caret_line_an_attribution_and_the_source_as_a_quote() {
    let source = Document {
        blocks: vec![Block::Paragraph(vec![
            text("The lamp "),
            Inline::Strong(vec![text("has shipped")]),
        ])],
    };
    let reply = quoted_reply(&source, "On 2026-08-26, Ada Lovelace wrote:");
    assert_eq!(
        reply.blocks,
        vec![
            // A blank line, not an empty paragraph: the caret needs a place
            // to sit above the quote, and parse narrows a truly empty
            // paragraph away on the first round trip.
            Block::Paragraph(vec![Inline::Break]),
            Block::Paragraph(vec![text("On 2026-08-26, Ada Lovelace wrote:")]),
            Block::Quote(source.blocks),
        ]
    );
}

#[test]
fn the_quoted_replys_text_form_is_the_familiar_angle_bracket_shape() {
    let source = Document::from_text("Short note\nto cover");
    let reply = quoted_reply(&source, "On 2026-08-26, Ada wrote:");
    let rendered = reply.to_text();
    assert!(rendered.contains("On 2026-08-26, Ada wrote:"), "{rendered}");
    assert!(rendered.contains("> Short note"), "{rendered}");
    assert!(rendered.contains("> to cover"), "{rendered}");
}

#[test]
fn quoting_nothing_still_leaves_the_attribution_but_no_empty_quote() {
    let reply = quoted_reply(&Document::new(), "On 2026-08-26, Ada wrote:");
    assert!(
        !reply
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Quote(_))),
        "an empty quote block says something was quoted when nothing was"
    );
    assert!(reply.to_text().contains("wrote:"));
}

#[test]
fn a_forward_carries_the_header_block_and_the_source_unquoted() {
    let source = Document {
        blocks: vec![Block::Paragraph(vec![text("Original words.")])],
    };
    let header = [
        "---------- Forwarded message ----------".to_owned(),
        "From: Ada Lovelace <ada@example.com>".to_owned(),
        "Date: 2026-08-26 09:00".to_owned(),
        "Subject: The lamp".to_owned(),
        "To: grace@example.com".to_owned(),
    ];
    let forward = forwarded(&source, &header);

    // The source arrives as itself — a forward presents the whole message,
    // not an answer to a fragment of it — so no Quote block wraps it.
    assert!(
        !forward
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Quote(_))),
        "a forward is not a quote"
    );
    let rendered = forward.to_text();
    for line in &header {
        assert!(rendered.contains(line.as_str()), "{rendered}");
    }
    assert!(rendered.contains("Original words."), "{rendered}");
    // The header block is one paragraph of five lines, matching the text
    // convention every mail client emits.
    assert!(
        forward.blocks.iter().any(|block| matches!(
            block,
            Block::Paragraph(inlines)
                if inlines.iter().filter(|inline| matches!(inline, Inline::Break)).count() == 4
        )),
        "{forward:?}"
    );
}

// ---------------------------------------------------------------------------
// The hostile corpus, both directions
// ---------------------------------------------------------------------------

/// Everything that fires on render with no action from anybody — the same
/// list `outgoing.rs` pins for the hand-assembled reply.
const LOADS: [&str; 7] = [
    "cdn.tracker.example.org",
    "pixel.tracker.example.org",
    "images.tracker.example.org",
    "background-image",
    "o.gif",
    "logo.gif",
    "lamp-brass-441",
];

const EXECUTES: [&str; 6] = ["<script", "<style", "<iframe", "style=", "class=", "onload"];

#[test]
fn a_reply_built_by_the_production_path_carries_no_load_and_no_script() {
    let reply = quoted_reply(&hostile_document(), "On 2026-08-26, a sender wrote:");
    let (rendered_text, rendered_html) = postio_body::render(&reply);

    for leak in LOADS {
        assert!(!rendered_html.contains(leak), "{leak}:\n{rendered_html}");
        assert!(!rendered_text.contains(leak), "{leak}:\n{rendered_text}");
    }
    for leak in EXECUTES {
        assert!(!rendered_html.contains(leak), "{leak}:\n{rendered_html}");
    }
    assert!(!rendered_html.contains("<img"), "{rendered_html}");
    // Still a quote of the message the human read.
    assert!(rendered_html.contains("has shipped"), "{rendered_html}");
    assert!(rendered_html.contains("<blockquote>"), "{rendered_html}");
}

#[test]
fn a_forward_built_by_the_production_path_carries_no_load_and_no_script() {
    let header = ["---------- Forwarded message ----------".to_owned()];
    let forward = forwarded(&hostile_document(), &header);
    let (rendered_text, rendered_html) = postio_body::render(&forward);

    for leak in LOADS {
        assert!(!rendered_html.contains(leak), "{leak}:\n{rendered_html}");
        assert!(!rendered_text.contains(leak), "{leak}:\n{rendered_text}");
    }
    for leak in EXECUTES {
        assert!(!rendered_html.contains(leak), "{leak}:\n{rendered_html}");
    }
    assert!(!rendered_html.contains("<img"), "{rendered_html}");
    assert!(rendered_html.contains("has shipped"), "{rendered_html}");
}

// ---------------------------------------------------------------------------
// Signatures at the document level
// ---------------------------------------------------------------------------

#[test]
fn a_signature_lands_after_the_quote_and_swaps_idempotently() {
    let reply = quoted_reply(
        &Document::from_text("original words"),
        "On 2026-08-26, Ada wrote:",
    );

    let signed = apply_signature(&reply, Some("Grace Hopper"));
    let rendered = signed.to_text();
    assert!(rendered.contains("-- "), "{rendered}");
    assert!(rendered.contains("Grace Hopper"), "{rendered}");
    let quote_at = rendered.find("> original words").expect("the quote stays");
    let sig_at = rendered.find("Grace Hopper").unwrap();
    assert!(
        sig_at > quote_at,
        "the signature goes at the end: {rendered}"
    );

    // Applying again changes nothing; switching identities swaps cleanly.
    assert_eq!(apply_signature(&signed, Some("Grace Hopper")), signed);
    let swapped = apply_signature(&signed, Some("Ada Lovelace"));
    let rendered = swapped.to_text();
    assert!(rendered.contains("Ada Lovelace"), "{rendered}");
    assert!(!rendered.contains("Grace Hopper"), "{rendered}");

    // And taking the signature away leaves the written part alone.
    let bare = apply_signature(&swapped, None);
    assert!(!bare.to_text().contains("Ada Lovelace"));
    assert!(bare.to_text().contains("> original words"));
}

#[test]
fn a_separator_inside_the_quote_is_not_this_drafts_signature() {
    // The quoted message's own signature arrives inside the Quote block, so
    // replacing "the signature" must never reach into it — mirroring
    // `postio_model::signature::split`, where a separator followed by quoted
    // lines is somebody else's.
    let source = Document::from_text("their words\n\n-- \nTheir Signature");
    let reply = quoted_reply(&source, "On 2026-08-26, Ada wrote:");
    let signed = apply_signature(&reply, Some("My Sig"));
    let rendered = signed.to_text();
    assert!(
        rendered.contains("Their Signature"),
        "the quote was edited: {rendered}"
    );
    assert!(rendered.contains("My Sig"), "{rendered}");

    let unsigned = apply_signature(&signed, None);
    assert!(
        unsigned.to_text().contains("Their Signature"),
        "removing my signature removed theirs: {}",
        unsigned.to_text()
    );
}

#[test]
fn rich_structure_survives_a_signature_swap() {
    // The regression this API exists to prevent: the composer used to apply
    // signatures by flattening the whole body to text and reloading it,
    // which would have turned a rich quote into angle-bracket lines.
    let source = Document {
        blocks: vec![Block::Paragraph(vec![Inline::Strong(vec![text("bold")])])],
    };
    let reply = quoted_reply(&source, "On 2026-08-26, Ada wrote:");
    let signed = apply_signature(&reply, Some("Grace"));
    assert!(
        signed.blocks.iter().any(|block| matches!(
            block,
            Block::Quote(blocks)
                if blocks.iter().any(|inner| matches!(
                    inner,
                    Block::Paragraph(inlines)
                        if inlines.iter().any(|inline| matches!(inline, Inline::Strong(_)))
                ))
        )),
        "{signed:?}"
    );
}
