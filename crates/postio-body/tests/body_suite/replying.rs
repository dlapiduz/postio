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
        id: Default::default(),
        name: String::new(),
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

// ---------------------------------------------------------------------------
// FR-044: the quote is the original as the reader renders it
//
// ADR 0033 reverses ADR 0003 Q3's *representation* while keeping its argument.
// The old rule was that a quote is rebuilt from the closed `Document`, so a
// script has no representation rather than being stripped on the way out. The
// new rule is that a quote carries the reader's own sanitised rendering — and
// the type is still the gate, because `Quoted` can only be made by a
// constructor that sanitises. What changed is what survives sanitising: a
// table, a colour, a class. What did not change is what does not.
// ---------------------------------------------------------------------------

/// A source with structure and styling the closed `Document` cannot hold.
///
/// Both carriers the reader actually admits: a `<style>` block, which is
/// parsed and scoped, and inline `style`, which is the one attribute
/// `add_generic_attributes` allows. Deliberately *not* `class` -- the
/// sanitiser drops it, so the reader drops it, so the quote must too. FR-045
/// says the quote is what the reader renders, which cuts both ways.
const RICH: &str = "\
<style>td { padding: 4px } p { color: #b00 }</style>\
<table><tr><td>Gate</td><td>Interlock</td></tr>\
<tr><td>North</td><td style=\"font-weight:bold\">Armed</td></tr></table>\
<p>Do not reset before the tide turns.</p>";

#[test]
fn an_html_originals_structure_and_styling_survive_into_the_quote() {
    // FR-044. Under the old rule every one of these assertions failed: a
    // `<table>` has no `Block`, and a class has nowhere to live at all.
    let quoted = postio_body::quote_of(Some(RICH), "Do not reset.", "q1");

    assert!(
        quoted.html().contains("<table"),
        "the table became something else: {}",
        quoted.html()
    );
    assert!(
        quoted.html().contains("Interlock") && quoted.html().contains("Armed"),
        "the table's cells did not survive"
    );
    assert!(
        quoted.html().contains("font-weight:bold"),
        "the inline declaration was dropped -- that is the attribute the \
         sanitiser admits and most HTML mail styles itself with: {}",
        quoted.html()
    );
    assert!(
        quoted.styles().contains("#b00"),
        "the sender's stylesheet did not survive: {}",
        quoted.styles()
    );
}

#[test]
fn a_quotes_styles_are_scoped_so_they_cannot_reach_the_users_own_text() {
    // FR-078, and the reason `styles.rs` exists: admitting one unscoped
    // sheet would let a message restyle Postio's chrome, the reply being
    // written above it, or an earlier quote nested inside it.
    let quoted = postio_body::quote_of(Some(RICH), "Do not reset.", "q1");

    for rule in quoted.styles().split('}').filter(|rule| rule.contains('{')) {
        let selector = rule.split('{').next().unwrap_or_default().trim();
        if selector.is_empty() {
            continue;
        }
        assert!(
            selector.contains("q1"),
            "a rule escaped the quote's scope and can match anything on the \
             page: {selector:?} in {}",
            quoted.styles()
        );
    }
}

#[test]
fn a_plain_text_only_original_falls_back_rather_than_quoting_nothing() {
    // FR-045. The failure this guards against is silent: a reply to a
    // text/plain message opening with an attribution and an empty box.
    let quoted = postio_body::quote_of(None, "Tide gate interlock is armed.", "q1");

    assert!(
        quoted.html().contains("Tide gate interlock is armed."),
        "a text-only original produced an empty quote: {:?}",
        quoted.html()
    );
    assert!(
        quoted.text().contains("Tide gate interlock is armed."),
        "the plain rendering lost the text too"
    );
}

#[test]
fn an_html_original_that_sanitises_to_nothing_still_falls_back_to_its_text() {
    // The case between the two above, and the one a naive `is_some` check
    // gets wrong: there *is* an HTML part, and nothing in it survives.
    let quoted = postio_body::quote_of(
        Some("<script>steal()</script>"),
        "Tide gate interlock is armed.",
        "q1",
    );

    assert!(
        quoted.html().contains("Tide gate interlock is armed."),
        "an HTML part that sanitised away left an empty quote instead of \
         falling back to the text alternative: {:?}",
        quoted.html()
    );
}

#[test]
fn a_quote_blocks_remote_images_even_where_the_reader_was_allowed_to_show_them() {
    // ADR 0033 Q2, and the one rule in this file that is about someone other
    // than the user. Allowing a sender's remote images is a decision the
    // reader makes on this machine, about this mailbox. Re-emitting them in a
    // reply would carry that decision to every recipient -- and hand the
    // sender a beacon that now fires in other people's clients.
    let with_beacon = "<p>Morning.</p><img src=\"https://pixel.tracker.example.org/x.gif\">";
    let quoted = postio_body::quote_of(Some(with_beacon), "Morning.", "q1");

    assert!(
        !quoted.html().contains("pixel.tracker.example.org"),
        "a remote reference was re-emitted into the reply: {}",
        quoted.html()
    );
    assert!(
        !quoted.html().contains("https://"),
        "some remote reference survived: {}",
        quoted.html()
    );
}

#[test]
fn no_quote_of_any_corpus_message_re_emits_a_script_or_a_remote_reference() {
    // FR-047 as a security test rather than a rendering nicety: rendering
    // happens on one machine, re-emission puts markup in front of everyone
    // who receives the reply. Corpus-wide, and it counts what it checked so
    // it cannot quietly pass over an empty set.
    let mut checked = 0;
    for fixture in postio_model::test_corpus::all() {
        let message = fixture.parse();
        let Some(html) = message.body.html.as_deref() else {
            continue;
        };
        let text = message.body.text.clone().unwrap_or_default();
        let quoted = postio_body::quote_of(Some(html), &text, "q1");
        let emitted = format!("{} {}", quoted.html(), quoted.styles());

        // Loading, not linking. An `<a href="https://...">` fetches nothing
        // until someone clicks it and the reader keeps it, so banning every
        // `https://` would ban ordinary correspondence. What may never be
        // re-emitted is anything the recipient's client would fetch on its
        // own -- which is what a tracking pixel *is*.
        for forbidden in [
            "<script",
            "<iframe",
            "<object",
            "<embed",
            "javascript:",
            "onerror=",
            "onload=",
            "src=\"http",
            "src='http",
            "url(http",
            "url(\"http",
            "url('http",
            "background=\"http",
        ] {
            assert!(
                !emitted.contains(forbidden),
                "{forbidden:?} was re-emitted from {}",
                fixture.name()
            );
        }
        checked += 1;
    }
    assert!(
        checked >= 5,
        "only {checked} HTML messages were checked; the corpus loader is not \
         finding them and this test is passing over nothing"
    );
}
