use postio_body::reader_view::{self, LINKS_KEPT};
use postio_body::sanitize::{self, RemoteImages};
use postio_model::test_corpus;

/// Through `sanitize_body` first: reader view runs over markup already made safe.
fn sanitized(name: &str) -> String {
    let fixture = test_corpus::load(name);
    let parsed = postio_model::mime::parse(fixture.bytes());
    let html = parsed
        .body
        .html
        .as_deref()
        .unwrap_or_else(|| panic!("`{name}` has no HTML part to reduce"));
    sanitize::sanitize_body(html, RemoteImages::Blocked).html
}

#[test]
fn a_real_newsletter_reduces_to_its_words_and_one_link() {
    let reduced = reader_view::reduce(&sanitized("html-newsletter"));

    assert!(
        !reduced.html.contains("<table") && !reduced.html.contains("<td"),
        "the layout tables survived reduction: {}",
        reduced.html
    );
    for attribute in ["bgcolor=", "width=", "align=", "cellpadding="] {
        assert!(
            !reduced.html.contains(attribute),
            "{attribute} survived: {}",
            reduced.html
        );
    }
    assert!(
        reduced.links_kept <= LINKS_KEPT,
        "a campaign keeps its primary call to action and no more, kept {}",
        reduced.links_kept
    );
    assert!(
        reduced.links_total() > reduced.links_kept,
        "the fixture is wrong if a newsletter has only one link in it"
    );
    assert!(
        !reduced.html.trim().is_empty(),
        "reducing a newsletter to nothing is not reading it"
    );
}

#[test]
fn a_real_newsletter_is_recognised_as_bulk() {
    assert!(
        reader_view::reads_as_bulk(&sanitized("html-newsletter")),
        "the fixture is a campaign and the heuristic should say so"
    );
}

#[test]
fn ordinary_mail_is_not_dragged_into_reader_view() {
    assert!(
        !reader_view::reads_as_bulk(&sanitized("multipart-alternative")),
        "an ordinary message must not be treated as a campaign"
    );
}

fn plain(name: &str) -> String {
    let fixture = test_corpus::load(name);
    let parsed = postio_model::mime::parse(fixture.bytes());
    parsed
        .body
        .text
        .unwrap_or_else(|| panic!("`{name}` has no plain part"))
}

#[test]
fn a_shipping_notice_yields_its_tracking_number_item_and_destination() {
    let found = reader_view::lift(&plain("transactional-shipping-notice")).rows;
    let rows: Vec<(&str, &str)> = found
        .iter()
        .map(|fact| (fact.label.as_str(), fact.value.as_str()))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("tracking", "EXTEST0042199317"),
            ("item", "Type-C Upgrade Small Board Replacement x 1"),
            ("ship to", "1 Example Way, Springfield"),
        ],
        "the three facts the canvas draws, in the order the sender wrote them"
    );
}

#[test]
fn the_prose_around_the_block_is_not_dragged_into_it() {
    let body = plain("transactional-shipping-notice");
    assert!(
        body.contains("Note: this notice"),
        "the fixture still carries the sentence this test is about"
    );
    assert_eq!(
        reader_view::lift(&body).rows.len(),
        3,
        "only the block, not the sentence with a colon in it"
    );
}

#[test]
fn a_newsletter_with_no_block_in_its_plain_part_yields_nothing() {
    let fixture = test_corpus::load("html-newsletter");
    let parsed = postio_model::mime::parse(fixture.bytes());
    let Some(text) = parsed.body.text else {
        return;
    };
    assert!(
        reader_view::lift(&text).rows.is_empty(),
        "a newsletter's plain part is prose, and prose has no facts block"
    );
}

#[test]
fn a_real_newsletters_columns_survive_sanitizing() {
    let clean = sanitized("html-newsletter");

    for kept in ["width=\"600\"", "width=\"70\"", "cellpadding", "valign"] {
        assert!(
            clean.contains(kept),
            "the newsletter lost {kept}, so it does not arrive in the columns \
             its sender built: {clean}"
        );
    }

    assert!(
        clean.matches("width=\"70\"").count() >= 3,
        "fewer than three columns kept their width, which is the case the \
         requirement names: {clean}"
    );

    // The layout attributes admitted in #1396 name no URL, and the deprecated `background`, which
    // loads an image, is not among them.
    assert!(
        !clean.contains("background="),
        "an attribute that fetches an image survived: {clean}"
    );
}

/// ADR 0032 puts several senders in one document, so a style that escapes its block reaches
/// somebody else's mail, not just Postio's chrome (#1410).
/// The survivors are checked too: a sanitizer that flattened everything would pass the refusal
/// half.
#[test]
fn a_message_cannot_style_its_way_out_of_its_own_block() {
    let clean = sanitized("html-escaping-styles");

    for refused in ["position:", "z-index", "vw", "vh"] {
        assert!(
            !clean.contains(refused),
            "`{refused}` survived, so a message can act outside its own block \
             -- in a conversation that is somebody else's mail: {clean}"
        );
    }

    for kept in [
        "color:#2b6cb0",
        "font-weight:700",
        "width=\"50%\"",
        "bgcolor",
    ] {
        assert!(
            clean.contains(kept),
            "containment took {kept} with it. FR-019b refuses a stated list \
             for a stated reason; flattening the sender's styling is not that: \
             {clean}"
        );
    }
}

/// Pins `reads_as_bulk` (#1412): a table-free campaign with six links is not bulk (a mild blind
/// spot), twelve links is; a reply quoting one table and a heavily styled personal note are not.
/// A style signal would risk the last case: correspondence is styled too, and reducing somebody's
/// letter because they chose Georgia is the worse error.
#[test]
fn styled_correspondence_is_not_mistaken_for_a_campaign() {
    let letter = sanitize::sanitize_body(
        "<p style=\"font-family:Georgia;color:#333;line-height:1.6;margin:24px\">Dear Ada, \
         the plans arrived and the survey is booked.</p>\
         <p style=\"color:#666\">Best, Grace</p>",
        RemoteImages::Blocked,
    )
    .html;
    assert!(
        !reader_view::reads_as_bulk(&letter),
        "a styled letter read as a campaign. Reducing somebody's
         correspondence to prose because they chose a serif is worse than
         letting a campaign through: {letter}"
    );

    let quoting = sanitize::sanitize_body(
        "<p>Sounds good.</p><blockquote><table><tr><td>agenda</td></tr></table></blockquote>",
        RemoteImages::Blocked,
    )
    .html;
    assert!(
        !reader_view::reads_as_bulk(&quoting),
        "a reply quoting one table read as a campaign -- the signal is \
         *nested* tables, because a template nests them and a quote does \
         not: {quoting}"
    );

    assert!(
        reader_view::reads_as_bulk(&sanitized("html-newsletter")),
        "the corpus newsletter stopped reading as bulk"
    );
}
