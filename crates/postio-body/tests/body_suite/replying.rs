use postio_body::document::{Block, Document, Inline};
use postio_body::{Placement, apply_signature, forwarded, quoted_reply};
use postio_model::account::Signature;

const HOSTILE: &str = "html-tracking-pixel-remote-images.eml";

fn hostile_quote() -> postio_body::Quoted {
    let fixture = postio_model::test_corpus::get(HOSTILE)
        .unwrap_or_else(|| panic!("{HOSTILE} is not in the corpus"));
    let message = fixture.parse();
    let html = message
        .body
        .html
        .expect("the fixture is a text/html message");
    assert!(
        html.contains("pixel.tracker.example.org"),
        "the fixture arrived without its beacon, so this test cannot fail"
    );
    postio_body::quote_of(
        Some(&html),
        message.body.text.as_deref().unwrap_or_default(),
        "q1",
    )
}

fn text(text: &str) -> Inline {
    Inline::Text(text.to_owned())
}

#[test]
fn a_quoted_reply_is_a_caret_line_an_attribution_and_the_source_as_a_quote() {
    let source = postio_body::quote_of(
        Some("<p>The lamp <strong>has shipped</strong></p>"),
        "The lamp has shipped",
        "q1",
    );
    let reply = quoted_reply(&source, "On 2026-08-26, Ada Lovelace wrote:");
    assert_eq!(
        reply.blocks,
        vec![
            Block::Paragraph(vec![Inline::Break]),
            Block::Paragraph(vec![text("On 2026-08-26, Ada Lovelace wrote:")]),
            Block::Quoted(source),
        ]
    );
}

#[test]
fn the_quoted_replys_text_form_is_the_familiar_angle_bracket_shape() {
    let source = postio_body::quote_of(None, "Short note\nto cover", "q1");
    let reply = quoted_reply(&source, "On 2026-08-26, Ada wrote:");
    let rendered = reply.to_text();
    assert!(rendered.contains("On 2026-08-26, Ada wrote:"), "{rendered}");
    assert!(rendered.contains("> Short note"), "{rendered}");
    assert!(rendered.contains("> to cover"), "{rendered}");
}

#[test]
fn quoting_nothing_still_leaves_the_attribution_but_no_empty_quote() {
    let reply = quoted_reply(
        &postio_body::quote_of(None, "", "q1"),
        "On 2026-08-26, Ada wrote:",
    );
    assert!(
        !reply
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Quote(_) | Block::Quoted(_))),
        "an empty quote block says something was quoted when nothing was"
    );
    assert!(reply.to_text().contains("wrote:"));
}

#[test]
fn a_forward_carries_the_header_block_and_the_source_unquoted() {
    let source = postio_body::quote_of(Some("<p>Original words.</p>"), "Original words.", "q1");
    let header = [
        "---------- Forwarded message ----------".to_owned(),
        "From: Ada Lovelace <ada@example.com>".to_owned(),
        "Date: 2026-08-26 09:00".to_owned(),
        "Subject: The lamp".to_owned(),
        "To: grace@example.com".to_owned(),
    ];
    let forward = forwarded(&source, &header);

    // Since #1483 a forward carries the source rather than flattening it, but is still not shown as
    // a quote: a plain `div`, never a `<blockquote>`.
    assert!(
        !forward
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Quote(_))),
        "a forward is not a quote"
    );
    assert!(
        !forward.to_html().contains("<blockquote"),
        "a forward was wrapped as a quotation: {}",
        forward.to_html()
    );
    let rendered = forward.to_text();
    for line in &header {
        assert!(rendered.contains(line.as_str()), "{rendered}");
    }
    assert!(rendered.contains("Original words."), "{rendered}");
    assert!(
        forward.blocks.iter().any(|block| matches!(
            block,
            Block::Paragraph(inlines)
                if inlines.iter().filter(|inline| matches!(inline, Inline::Break)).count() == 4
        )),
        "{forward:?}"
    );
}

const LOADS: [&str; 7] = [
    "cdn.tracker.example.org",
    "pixel.tracker.example.org",
    "images.tracker.example.org",
    "background-image",
    "o.gif",
    "logo.gif",
    "lamp-brass-441",
];

/// `style=` is deliberately off the list (ADR 0033): inline declarations carry a quote's styling.
/// `<style` stays, since what is emitted is CSS Postio parsed and rewrote in `Quoted::styles`.
const EXECUTES: [&str; 4] = ["<script", "<style", "<iframe", "onload"];

#[test]
fn a_reply_built_by_the_production_path_carries_no_load_and_no_script() {
    let reply = quoted_reply(&hostile_quote(), "On 2026-08-26, a sender wrote:");
    let (rendered_text, rendered_html) = postio_body::render(&reply);

    for leak in LOADS {
        assert!(!rendered_html.contains(leak), "{leak}:\n{rendered_html}");
        assert!(!rendered_text.contains(leak), "{leak}:\n{rendered_text}");
    }
    for leak in EXECUTES {
        assert!(!rendered_html.contains(leak), "{leak}:\n{rendered_html}");
    }
    // #1484: no image element at all; a remote `<img>` kept its element and lost its `src`, leaving
    // the recipient a broken icon. It is the sender's `alt` now, or nothing.
    assert!(
        !rendered_html.contains("<img"),
        "a broken image element reached the reply: {rendered_html}"
    );
    assert!(rendered_html.contains("has shipped"), "{rendered_html}");
    assert!(
        rendered_html.contains("<blockquote"),
        "the quote stopped being a quote: {rendered_html}"
    );
}

#[test]
fn a_forward_built_by_the_production_path_carries_no_load_and_no_script() {
    let header = ["---------- Forwarded message ----------".to_owned()];
    let forward = forwarded(&hostile_quote(), &header);
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

#[test]
fn a_signature_lands_after_the_quote_and_swaps_idempotently() {
    let reply = quoted_reply(
        &postio_body::quote_of(None, "original words", "q1"),
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

    let bare = apply_signature(&swapped, None, Placement::BelowQuote);
    assert!(!bare.to_text().contains("Ada Lovelace"));
    assert!(bare.to_text().contains("> original words"));
}

#[test]
fn a_separator_inside_the_quote_is_not_this_drafts_signature() {
    let source = postio_body::quote_of(None, "their words\n\n-- \nTheir Signature", "q1");
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
    // The composer used to apply signatures by flattening the body to text and reloading it,
    // turning a rich quote into angle-bracket lines.
    let source = postio_body::quote_of(Some("<p><strong>bold</strong></p>"), "bold", "q1");
    let reply = quoted_reply(&source, "On 2026-08-26, Ada wrote:");
    let signed = apply_signature(
        &reply,
        Some(&signature("Grace", None)),
        Placement::BelowQuote,
    );
    let quoted = signed
        .blocks
        .iter()
        .find_map(|block| match block {
            Block::Quoted(quoted) => Some(quoted),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the signature swap ate the quote: {signed:?}"));
    assert!(
        quoted.html().contains("<strong>"),
        "the emphasis was flattened by the signature swap: {}",
        quoted.html()
    );
}

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
    let reply = quoted_reply(
        &postio_body::quote_of(None, "their words", "q1"),
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
    let rendered = signed.to_text();
    assert!(rendered.contains("Grace Hopper"), "{rendered}");
    assert!(rendered.contains("-- "), "{rendered}");
}

#[test]
fn a_signature_with_no_rich_variant_falls_back_to_its_text() {
    let reply = quoted_reply(
        &postio_body::quote_of(None, "their words", "q1"),
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
    let reply = quoted_reply(
        &postio_body::quote_of(None, "their words", "q1"),
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
    assert!(
        signed
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Quote(_) | Block::Quoted(_))),
        "{signed:?}"
    );
}

#[test]
fn swapping_placement_moves_the_signature_rather_than_adding_one() {
    let reply = quoted_reply(
        &postio_body::quote_of(None, "their words", "q1"),
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
    let back = apply_signature(&above, Some(&sig), Placement::BelowQuote);
    assert_eq!(back, below);
}

#[test]
fn a_signature_above_the_quote_is_still_replaced_not_stacked() {
    let reply = quoted_reply(
        &postio_body::quote_of(None, "their words", "q1"),
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
    // RFC 3676: other clients fold on `-- ` immediately before the signature; a separate paragraph
    // renders a blank line under it.
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
    let html = signed.to_html();
    assert!(html.contains("-- <br>Lena"), "{html}");
}

#[test]
fn a_paragraph_that_merely_starts_with_two_hyphens_is_not_a_separator() {
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
    assert_eq!(signed.to_text(), "\n\n-- \nLena");
}

/// Both carriers the reader admits: a `<style>` block (parsed and scoped) and inline `style`.
/// Deliberately not `class`, which the sanitiser drops, so the quote must too.
const RICH: &str = "\
<style>td { padding: 4px } p { color: #b00 }</style>\
<table><tr><td>Gate</td><td>Interlock</td></tr>\
<tr><td>North</td><td style=\"font-weight:bold\">Armed</td></tr></table>\
<p>Do not reset before the tide turns.</p>";

#[test]
fn an_html_originals_structure_and_styling_survive_into_the_quote() {
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
    // An unscoped sheet would let a message restyle Postio's chrome, the reply being written, or an
    // earlier quote.
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
    // The silent failure: a reply to a text/plain message opening with an attribution and an empty
    // box.
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
    // ADR 0033 Q2: allowing images is a local decision; re-emitting them would fire a beacon in
    // every recipient's client.
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
    // Re-emission puts markup in front of everyone who receives the reply. Counts what it checked
    // so it cannot pass over an empty set.
    let mut checked = 0;
    for fixture in postio_model::test_corpus::all() {
        let message = fixture.parse();
        let Some(html) = message.body.html.as_deref() else {
            continue;
        };
        let text = message.body.text.clone().unwrap_or_default();
        let quoted = postio_body::quote_of(Some(html), &text, "q1");
        let emitted = format!("{} {}", quoted.html(), quoted.styles());

        // Loading, not linking: an `<a href>` fetches nothing until clicked, so banning every
        // `https://` would ban ordinary correspondence.
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
            "<img",
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

#[test]
fn a_quote_survives_the_round_trip_through_the_editor_intact() {
    // What is in the editor is what is sent; every keystroke costs a `to_html` and a `parse`, so
    // the rich quote must survive that round trip.
    let quoted = postio_body::quote_of(Some(RICH), "Do not reset.", "q1");
    let document = postio_body::Document {
        blocks: vec![
            postio_body::document::Block::Paragraph(vec![text("Acknowledged.")]),
            postio_body::document::Block::Quoted(quoted),
        ],
    };

    let round_tripped = postio_body::parse(&document.to_html());

    let Some(postio_body::document::Block::Quoted(after)) = round_tripped
        .blocks
        .iter()
        .find(|block| matches!(block, postio_body::document::Block::Quoted(_)))
    else {
        panic!(
            "the quote came back as something other than a quote: {:?}",
            round_tripped.blocks
        );
    };
    assert!(
        after.html().contains("<table") && after.html().contains("Interlock"),
        "the table did not survive the round trip: {}",
        after.html()
    );
    assert!(
        after.html().contains("font-weight:bold"),
        "the inline declaration did not survive the round trip: {}",
        after.html()
    );
    let text = round_tripped.to_text();
    assert!(
        text.contains("> Do not reset before the tide turns."),
        "the plain rendering lost the quote markers: {text:?}"
    );
    assert!(
        text.lines().all(|line| line.is_empty()
            || line.starts_with('>')
            || line.starts_with("Acknowledged")),
        "a quoted line escaped its markers: {text:?}"
    );
    assert!(
        !text.contains("GateInterlock"),
        "the table's cells ran together in the plain half: {text:?}"
    );
}

#[test]
fn a_script_smuggled_into_the_editors_quote_does_not_come_back_out() {
    // `parse` rebuilds through `quote_of` instead of trusting what it finds: the content has been
    // through a `contenteditable` DOM (paste handlers, extensions, bugs) since it was sanitised.
    let smuggled = "<blockquote data-postio-quoted=\"1\">\
<p>Morning.</p><script>steal()</script>\
<img src=\"https://pixel.tracker.example.org/x.gif\"></blockquote>";

    let document = postio_body::parse(smuggled);
    let rendered = document.to_html();

    assert!(
        !rendered.contains("<script") && !rendered.contains("steal()"),
        "a script came back out of the editor: {rendered}"
    );
    assert!(
        !rendered.contains("pixel.tracker.example.org"),
        "a beacon came back out of the editor: {rendered}"
    );
    assert!(
        rendered.contains("Morning."),
        "sanitising took the content with it: {rendered}"
    );
}

#[test]
fn an_image_in_a_quote_becomes_what_the_sender_called_it() {
    // #1484: the reader blocks an image by stripping its `src`, which suits its banner; a sent
    // reply has none, so every quote image is broken by construction. The sender's `alt` is the one
    // thing worth keeping.
    let quoted = postio_body::quote_of(
        Some(
            "<p>Morning.</p>\
             <img src=\"https://shop.example.org/lamp.png\" alt=\"Brass reading lamp\">\
             <p>Best.</p>",
        ),
        "Morning. Best.",
        "q1",
    );

    assert!(
        !quoted.html().contains("<img"),
        "a broken image element reached the reply: {}",
        quoted.html()
    );
    assert!(
        quoted.html().contains("Brass reading lamp"),
        "the sender's own description went with it, so the recipient cannot \
         tell there was a picture at all: {}",
        quoted.html()
    );
    assert!(
        quoted.html().contains("Morning.") && quoted.html().contains("Best."),
        "the text around the image was disturbed: {}",
        quoted.html()
    );
}

#[test]
fn an_image_with_nothing_to_say_leaves_nothing_behind() {
    // A 1x1 tracking pixel with no `alt` has nothing to contribute and goes; a caption of `[]`
    // would be worse than the broken icon.
    let quoted = postio_body::quote_of(
        Some("<p>Morning.</p><img src=\"https://pixel.tracker.example.org/x.gif\" alt=\"\">"),
        "Morning.",
        "q1",
    );

    assert!(!quoted.html().contains("<img"), "{}", quoted.html());
    assert!(
        !quoted.html().contains("[]"),
        "an image with no description left an empty caption: {}",
        quoted.html()
    );
    assert!(quoted.html().contains("Morning."), "{}", quoted.html());
}

#[test]
fn a_forward_carries_the_originals_structure_the_way_a_reply_does() {
    // #1483: a forward used to flatten while a reply preserved. `Block::Quoted` stays editable
    // because `parse` rebuilds it from the DOM, so trimming half of a forward survives.
    let source = postio_body::quote_of(Some(RICH), "Do not reset.", "q1");
    let forwarded = postio_body::forwarded(
        &source,
        &["---------- Forwarded message ----------".to_owned()],
    );

    let carried = forwarded
        .blocks
        .iter()
        .find_map(|block| match block {
            Block::Quoted(quoted) => Some(quoted),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the forward carries nothing: {forwarded:?}"));

    assert!(
        carried.html().contains("<table"),
        "the forward flattened the table the reply keeps: {}",
        carried.html()
    );
    assert!(
        carried.html().contains("font-weight:bold"),
        "the inline declaration was dropped: {}",
        carried.html()
    );
    assert!(
        forwarded
            .to_text()
            .contains("---------- Forwarded message ----------"),
        "the forwarding header block went missing: {:?}",
        forwarded.to_text()
    );
}

/// An attribution above an empty quote reads as a quote that failed to load; the composer says the
/// original is still downloading.
#[test]
fn a_reply_to_a_message_with_no_body_yet_has_no_attribution() {
    let fixture = postio_model::test_corpus::get(HOSTILE)
        .unwrap_or_else(|| panic!("{HOSTILE} is not in the corpus"));
    let mut source = fixture.parse();
    assert!(
        postio_body::replying::quoted_body(&source, false)
            .text
            .unwrap_or_default()
            .contains("wrote:"),
        "a message with a body does get its attribution, so this test can fail"
    );
    source.body = postio_model::MessageBody::default();
    for forward in [false, true] {
        let body = postio_body::replying::quoted_body(&source, forward);
        assert!(
            body.text.as_deref().unwrap_or_default().trim().is_empty()
                && body
                    .html
                    .as_deref()
                    .is_none_or(|html| !html.contains("wrote:")),
            "a source with nothing to quote left {body:?} (forward: {forward})"
        );
    }
}

mod requoting {
    use postio_body::{Block, Inline, quote_of, quoted_reply, requote};
    use postio_model::MessageBody;

    const FOUR: &str = "On 2026-03-04, Ann wrote:";
    const SIX: &str = "On 2026-03-06, Cy wrote:";

    fn six() -> postio_body::Quoted {
        quote_of(None, "six says hello\nand goodbye", "q6")
    }

    fn rich_reply_with(typed: &str) -> MessageBody {
        let four = quote_of(None, "four says hello", "q4");
        let mut document = quoted_reply(&four, FOUR);
        document.blocks[0] = Block::Paragraph(vec![Inline::Text(typed.to_owned())]);
        MessageBody {
            text: Some(document.to_text()),
            html: Some(document.to_html()),
        }
    }

    #[test]
    fn the_plain_half_keeps_the_typed_words_and_quotes_the_new_message() {
        let old = MessageBody {
            text: Some(format!(
                "typed first\n\nand second\n\n{FOUR}\n> four says hello\n"
            )),
            html: None,
        };
        let new = requote(&old, SIX, &six());
        assert_eq!(
            new.text.as_deref(),
            Some(
                format!("typed first\n\nand second\n\n{SIX}\n> six says hello\n> and goodbye")
                    .as_str()
            )
        );
        assert_eq!(new.html, None, "a plain reply stays plain");
    }

    #[test]
    fn the_rich_half_keeps_the_typed_words_and_quotes_the_new_message() {
        let new = requote(&rich_reply_with("my typed words"), SIX, &six());
        let html = new.html.expect("still rich");
        assert!(html.contains("my typed words"), "{html}");
        assert!(html.contains("six says hello"), "{html}");
        assert!(html.contains(SIX), "{html}");
        assert!(!html.contains("four says"), "the old quote is gone: {html}");
        assert!(!html.contains(FOUR), "so is its attribution: {html}");
        let text = new.text.expect("and its text half");
        assert!(text.contains("my typed words"), "{text}");
        assert!(text.contains("> six says hello"), "{text}");
        assert!(!text.contains("four says"), "{text}");
    }

    #[test]
    fn what_follows_the_quote_is_kept() {
        let old = MessageBody {
            text: Some(format!("{FOUR}\n> four says hello\n-- \nAda\n")),
            html: None,
        };
        let text = requote(&old, SIX, &six()).text.expect("text");
        assert!(text.ends_with("> and goodbye\n-- \nAda\n"), "{text:?}");
    }

    #[test]
    fn a_body_with_no_quote_gets_the_new_one_after_the_words() {
        let old = MessageBody {
            text: Some("only my words".to_owned()),
            html: None,
        };
        let text = requote(&old, SIX, &six()).text.expect("text");
        assert_eq!(
            text,
            format!("only my words\n\n{SIX}\n> six says hello\n> and goodbye")
        );
    }
}
