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
    // The fixture's closing paragraph contains `Note: this notice`, mid
    // sentence and on its own -- exactly the shape that would produce a
    // fabricated one-row table.
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

/// FR-019a on a real campaign, not on markup written to pass it (#1396).
///
/// `sanitize.rs`'s own test builds a three-column table by hand. This is the
/// corpus newsletter — nested layout tables, `width="600"` on the shell,
/// `width="70"` columns, `cellpadding`, `valign` — which is what a campaign
/// actually sends and what "a message that arranges itself in three columns
/// MUST appear in three columns" was written about.
///
/// The distinction matters here more than usual. Ammonia's per-tag defaults
/// dropped every one of these until #1396, and the hand-written test and this
/// one would have failed together — but only this one says the *shape of real
/// mail* survives, which is the claim FR-019a actually makes.
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

    // Its own layout, not a stray from somewhere: the shell table is 600px
    // wide and the columns inside it are 70, which is the arrangement the
    // fixture was written with.
    assert!(
        clean.matches("width=\"70\"").count() >= 3,
        "fewer than three columns kept their width, which is the case the \
         requirement names: {clean}"
    );

    // And nothing new reaches the network: the layout attributes admitted in
    // #1396 name no URL, and the deprecated `background` -- which does load an
    // image -- is deliberately not among them.
    assert!(
        !clean.contains("background="),
        "an attribute that fetches an image survived: {clean}"
    );
}

/// Containment, on a message rather than on a string (#1410).
///
/// The rules are unit-tested in `sanitize.rs` against markup written to
/// exercise one each. This is the same hostility inside real message bytes,
/// reached through `mime::parse` the way the reader reaches it — which is the
/// path a fixture exists to cover.
///
/// It matters more since ADR 0032 put several senders in one document. In a
/// pane holding one message, a style that escaped its block reached Postio's
/// own chrome; in a thread it reaches *somebody else's mail*.
///
/// FR-019b is the shape of the assertion: refusal is an enumerable list with
/// a stated reason, not a general suspicion of sender CSS. So the survivors
/// are checked too — a sanitizer that flattened everything would pass the
/// first half of this and fail the second.
#[test]
fn a_message_cannot_style_its_way_out_of_its_own_block() {
    let clean = sanitized("html-escaping-styles");

    // What `REFUSED` and `REFUSED_UNITS` name, and only that.
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

/// What `reads_as_bulk` must not start catching (#1412, T066).
///
/// The heuristic is two signals — a nested table, or ten links — and the
/// pressure on it is to add a third now that the sender's inline styling
/// survives (#1325, #1396). Measured before changing anything, it has one
/// real blind spot and two correct answers worth keeping:
///
/// | | reads as bulk |
/// |---|---|
/// | a table-free campaign with six links | **no** — the blind spot |
/// | the same with twelve | yes, on the link count |
/// | a reply quoting one table | no |
/// | a heavily styled personal note | no |
///
/// The blind spot is mild: a campaign that opens in its sender's own layout
/// is what FR-019a asks for anyway, and reader view exists for mail that is
/// *unreadable* raw. A style signal would close it and would risk the last
/// row — correspondence is styled too, and reducing somebody's letter to
/// prose because they chose Georgia is the worse error of the two.
///
/// So this pins the answers rather than changing them. Whoever adds a third
/// signal has to keep the personal note out, which is the constraint that
/// makes the change hard and is invisible without a test.
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

    // And the corpus newsletter still is one, so the guard rails above have
    // not been won by making the heuristic answer no to everything.
    assert!(
        reader_view::reads_as_bulk(&sanitized("html-newsletter")),
        "the corpus newsletter stopped reading as bulk"
    );
}
