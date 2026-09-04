//! Reader view against the corpus — the mail it exists for.
//!
//! The unit tests in `src/reader_view.rs` prove the rules on markup written
//! to exercise one rule each. This proves them on a real-shaped newsletter:
//! nested layout tables, inline CSS, a media query, `List-Unsubscribe` and
//! One-Click. That fixture is in the corpus precisely because contrived
//! markup does not reproduce what a campaign actually sends.

use postio_body::reader_view::{self, LINKS_KEPT};
use postio_body::sanitize::{self, RemoteImages};
use postio_model::test_corpus;

/// The HTML part of a corpus fixture, sanitized the way the reader gets it.
///
/// Through `sanitize_body` first, deliberately: reader view is a readability
/// pass over markup that has already been made safe, and testing it on raw
/// sender HTML would be proving something the reader never does.
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
    // The failure that would make this feature hated: a person's actual
    // correspondence opening reduced. `multipart-alternative` is an ordinary
    // message that happens to carry an HTML part.
    assert!(
        !reader_view::reads_as_bulk(&sanitized("multipart-alternative")),
        "an ordinary message must not be treated as a campaign"
    );
}

/// The plain part of a corpus fixture, as the reader gets it.
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
    let found = reader_view::facts(&plain("transactional-shipping-notice"));
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
    // The fixture's closing paragraph contains `Note: this notice`, mid
    // sentence and on its own -- exactly the shape that would produce a
    // fabricated one-row table.
    let body = plain("transactional-shipping-notice");
    assert!(
        body.contains("Note: this notice"),
        "the fixture still carries the sentence this test is about"
    );
    assert_eq!(
        reader_view::facts(&body).len(),
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
        reader_view::facts(&text).is_empty(),
        "a newsletter's plain part is prose, and prose has no facts block"
    );
}
