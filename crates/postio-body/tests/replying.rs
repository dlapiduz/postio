//! The documents a reply and a forward start from: issue #340.
//!
//! ADR 0003 hardening requirement 6, in the direction that matters most: a
//! reply re-emits quoted markup into the world, so the quote is built from
//! the parsed [`Document`] — where a script or a tracking pixel has no
//! representation — never from the source's markup. Both directions are
//! checked against the hostile corpus fixture, through the real MIME parser,
//! exactly as `outgoing.rs` does for the hand-assembled shape.

use postio_body::document::{Block, Document, Inline};
use postio_body::{Placement, apply_signature, forwarded, parse, quoted_reply};
use postio_model::account::Signature;

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

    let signed = apply_signature(
        &reply,
        Some(&signature("Grace Hopper", None)),
        Placement::BelowQuote,
    );
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
    assert_eq!(
        apply_signature(
            &signed,
            Some(&signature("Grace Hopper", None)),
            Placement::BelowQuote
        ),
        signed
    );
    let swapped = apply_signature(
        &signed,
        Some(&signature("Ada Lovelace", None)),
        Placement::BelowQuote,
    );
    let rendered = swapped.to_text();
    assert!(rendered.contains("Ada Lovelace"), "{rendered}");
    assert!(!rendered.contains("Grace Hopper"), "{rendered}");

    // And taking the signature away leaves the written part alone.
    let bare = apply_signature(&swapped, None, Placement::BelowQuote);
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
    let signed = apply_signature(
        &reply,
        Some(&signature("My Sig", None)),
        Placement::BelowQuote,
    );
    let rendered = signed.to_text();
    assert!(
        rendered.contains("Their Signature"),
        "the quote was edited: {rendered}"
    );
    assert!(rendered.contains("My Sig"), "{rendered}");

    let unsigned = apply_signature(&signed, None, Placement::BelowQuote);
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
    let signed = apply_signature(
        &reply,
        Some(&signature("Grace", None)),
        Placement::BelowQuote,
    );
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

// ---------------------------------------------------------------------------
// #12: the rich variant, and where the signature sits
// ---------------------------------------------------------------------------

fn signature(text: &str, html: Option<&str>) -> Signature {
    Signature {
        text: text.to_owned(),
        html: html.map(str::to_owned),
    }
}

#[test]
fn a_rich_signature_keeps_its_structure_instead_of_being_flattened() {
    // The HTML variant has been in the model (and the schema) since the
    // beginning with nothing to put it in; the composer has a rich body now.
    // A signature with markup must arrive as markup, not as its text form.
    let reply = quoted_reply(
        &Document::from_text("their words"),
        "On 2026-08-26, Ada wrote:",
    );
    let signed = apply_signature(
        &reply,
        Some(&signature(
            "Grace Hopper\nRear Admiral",
            Some("<p><strong>Grace Hopper</strong><br>Rear Admiral</p>"),
        )),
        Placement::BelowQuote,
    );

    assert!(
        signed.blocks.iter().any(|block| matches!(
            block,
            Block::Paragraph(inlines)
                if inlines.iter().any(|inline| matches!(inline, Inline::Strong(_)))
        )),
        "the rich variant was flattened: {signed:?}"
    );
    // And the plain rendering of that same document is still the text form a
    // text-only recipient reads.
    let rendered = signed.to_text();
    assert!(rendered.contains("Grace Hopper"), "{rendered}");
    assert!(rendered.contains("-- "), "{rendered}");
}

#[test]
fn a_signature_with_no_rich_variant_falls_back_to_its_text() {
    let reply = quoted_reply(
        &Document::from_text("their words"),
        "On 2026-08-26, Ada wrote:",
    );
    let signed = apply_signature(
        &reply,
        Some(&signature("Grace Hopper", None)),
        Placement::BelowQuote,
    );
    assert!(signed.to_text().contains("Grace Hopper"));
}

#[test]
fn placement_puts_the_signature_above_the_quote_when_asked() {
    // Top-posting: the signature belongs under what was written and above the
    // quoted message, which is where every client that top-posts puts it.
    let reply = quoted_reply(
        &Document::from_text("their words"),
        "On 2026-08-26, Ada wrote:",
    );
    let signed = apply_signature(
        &reply,
        Some(&signature("Grace Hopper", None)),
        Placement::AboveQuote,
    );

    let rendered = signed.to_text();
    let sig_at = rendered.find("Grace Hopper").expect("the signature");
    let quote_at = rendered.find("> their words").expect("the quote");
    assert!(
        sig_at < quote_at,
        "the signature should sit above the quote:\n{rendered}"
    );
    // The quote is still whole, and still a quote.
    assert!(
        signed
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Quote(_))),
        "{signed:?}"
    );
}

#[test]
fn swapping_placement_moves_the_signature_rather_than_adding_one() {
    let reply = quoted_reply(
        &Document::from_text("their words"),
        "On 2026-08-26, Ada wrote:",
    );
    let sig = signature("Grace Hopper", None);
    let below = apply_signature(&reply, Some(&sig), Placement::BelowQuote);
    let above = apply_signature(&below, Some(&sig), Placement::AboveQuote);

    assert_eq!(
        above.to_text().matches("Grace Hopper").count(),
        1,
        "moving a signature must not leave the old one behind:\n{}",
        above.to_text()
    );
    // And back again, idempotently.
    let back = apply_signature(&above, Some(&sig), Placement::BelowQuote);
    assert_eq!(back, below);
}

#[test]
fn a_signature_above_the_quote_is_still_replaced_not_stacked() {
    let reply = quoted_reply(
        &Document::from_text("their words"),
        "On 2026-08-26, Ada wrote:",
    );
    let first = apply_signature(
        &reply,
        Some(&signature("Grace Hopper", None)),
        Placement::AboveQuote,
    );
    let second = apply_signature(
        &first,
        Some(&signature("Ada Lovelace", None)),
        Placement::AboveQuote,
    );
    let rendered = second.to_text();
    assert!(rendered.contains("Ada Lovelace"), "{rendered}");
    assert!(!rendered.contains("Grace Hopper"), "{rendered}");
    assert!(rendered.contains("> their words"), "{rendered}");
}

#[test]
fn the_separator_sits_on_the_line_directly_above_the_signature() {
    // RFC 3676: the line other clients fold on is exactly `-- ` immediately
    // before the signature. A separator in a paragraph of its own renders
    // with a blank line under it, and then nothing recognises it.
    let signed = apply_signature(
        &Document::from_text("Looking now."),
        Some(&signature("Lena", None)),
        Placement::BelowQuote,
    );
    let rendered = signed.to_text();
    assert!(
        rendered.contains("-- \nLena"),
        "the separator must be adjacent to the signature:\n{rendered:?}"
    );
    // And in the rich rendering the two are one paragraph, one line apart.
    let html = signed.to_html();
    assert!(html.contains("-- <br>Lena"), "{html}");
}

#[test]
fn a_paragraph_that_merely_starts_with_two_hyphens_is_not_a_separator() {
    // "--fast is the flag you want" opens with the same two characters and is
    // somebody's sentence, not the end of their message.
    let written = Document::from_text("--fast is the flag you want\n\nMore below.");
    let signed = apply_signature(
        &written,
        Some(&signature("Lena", None)),
        Placement::BelowQuote,
    );
    let rendered = signed.to_text();
    assert!(rendered.contains("--fast is the flag"), "{rendered}");
    assert!(rendered.contains("More below."), "{rendered}");
    assert!(rendered.contains("-- \nLena"), "{rendered}");
}

#[test]
fn a_signature_on_an_empty_draft_leaves_somewhere_to_type() {
    // Without a line above it the caret opens inside the separator paragraph,
    // and the first word typed lands in front of the `-- `.
    let signed = apply_signature(
        &Document::new(),
        Some(&signature("Lena", None)),
        Placement::BelowQuote,
    );
    assert!(
        matches!(signed.blocks.first(), Some(Block::Paragraph(inlines))
            if !matches!(inlines.first(), Some(Inline::Text(text)) if text.starts_with("--"))),
        "the draft should open on a line of its own: {signed:?}"
    );
    // One blank line then the separator — the spelling the plain-text
    // pipeline has always put on the wire.
    assert_eq!(signed.to_text(), "\n\n-- \nLena");
}
