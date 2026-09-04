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
