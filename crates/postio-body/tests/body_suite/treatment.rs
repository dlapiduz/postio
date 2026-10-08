//! The two treatments against the corpus (specs/007-postio-focus T210,
//! T211): the mail the design handoff draws, and the hostile mail that must
//! reach neither treatment with anything live in it.
//!
//! The unit tests in `src/treatment.rs` prove each trigger on markup written
//! for it. These prove the rule on real-shaped mail, where a trigger has to
//! be found among everything else a sender's client wrote.

use postio_body::sanitize::{self, RemoteImages, Sanitized};
use postio_body::treatment::{self, Treatment, Trigger};
use postio_model::message::MessageBody;
use postio_model::test_corpus;

/// A fixture's body, as the reader gets it.
fn body(name: &str) -> MessageBody {
    postio_model::mime::parse(test_corpus::load(name).bytes()).body
}

/// A fixture's HTML part, sanitised the way the reader gets it.
fn sanitized(name: &str) -> Sanitized {
    let body = body(name);
    let html = body
        .html
        .as_deref()
        .unwrap_or_else(|| panic!("`{name}` has no HTML part"));
    sanitize::sanitize_body(html, RemoteImages::Blocked)
}

/// Plain text is app colours by the reader's rule, which never classifies
/// it (`postio_ui::reader::document::body_html_treated`); the fixture is
/// plain text only, which is what makes it the case.
#[test]
fn plain_text_has_no_html_to_classify() {
    let body = body("plain-text-simple");
    assert!(body.html.is_none(), "the fixture grew an HTML part");
    assert!(body.text.is_some_and(|text| !text.trim().is_empty()));
}

#[test]
fn office_work_mail_with_black_text_is_app_colours() {
    assert_eq!(
        treatment::paper_trigger(&sanitized("html-work-black-text")),
        None,
        "black text and a tinted header row are not a page of its own"
    );
}

#[test]
fn a_webmail_reply_chain_is_app_colours() {
    assert_eq!(
        treatment::classify(&sanitized("html-gmail-reply-chain")),
        Treatment::AppColours
    );
}

#[test]
fn a_newsletter_that_paints_its_page_is_paper() {
    assert_eq!(
        treatment::paper_trigger(&sanitized("html-newsletter-own-page")),
        Some(Trigger::PageBackground)
    );
}

#[test]
fn a_receipt_is_paper_by_its_layout_alone() {
    assert_eq!(
        treatment::paper_trigger(&sanitized("html-receipt-fixed-width")),
        Some(Trigger::WideTable)
    );
}

#[test]
fn the_corpus_rendering_fixtures_land_where_their_shape_says() {
    // Correspondence: a white page, dark text on nothing.
    for name in ["html-white-page-reply", "html-dark-text-no-background"] {
        assert_eq!(
            treatment::classify(&sanitized(name)),
            Treatment::AppColours,
            "{name}"
        );
    }
    // Designed mail: coloured cards and a dark footer in layout tables.
    for name in ["html-designed-three-column", "html-newsletter"] {
        assert_eq!(
            treatment::classify(&sanitized(name)),
            Treatment::Paper,
            "{name}"
        );
    }
}

#[test]
fn app_colours_takes_the_office_clients_ink_and_type_and_keeps_its_red() {
    let sanitized = sanitized("html-work-black-text");
    let html = treatment::app_colours(&sanitized.html);
    let css = treatment::app_colours_css(&sanitized.styles);
    for gone in ["color: black", "color:black", "#595959", "D9E2F3", "10.0pt"] {
        assert!(!html.contains(gone), "{gone} survived the markup: {html}");
    }
    for gone in ["Calibri", "black", "#0563C1"] {
        assert!(!css.contains(gone), "{gone} survived the stylesheet: {css}");
    }
    assert!(
        html.contains("#C00000"),
        "the sentence the sender coloured to say something lost its colour: {html}"
    );
    assert!(
        html.contains(treatment::GRID_CLASS),
        "the bordered table is not drawn as a grid: {html}"
    );
    assert!(
        !html.contains("&nbsp;</span></p>"),
        "spacer paragraphs survived: {html}"
    );
    for kept in [
        "Entrance",
        "Garage level P1",
        "floor plan",
        "Dana Whitfield",
    ] {
        assert!(html.contains(kept), "{kept} was lost: {html}");
    }
}

/// Neither treatment is handed anything that runs or reaches the network:
/// both start from the sanitiser's output, and this holds it to that on the
/// corpus's hostile mail (T210's "sanitise every HTML body").
#[test]
fn the_sanitiser_leaves_neither_treatment_anything_live() {
    for name in [
        "html-script-forms",
        "html-tracking-pixel-remote-images",
        "html-every-url-vector",
    ] {
        let sanitized = sanitized(name);
        let treated = [
            sanitized.html.clone(),
            treatment::app_colours(&sanitized.html),
        ];
        for html in treated {
            let lower = html.to_ascii_lowercase();
            for live in [
                "<script",
                "<form",
                "<input",
                "<button",
                "<iframe",
                "<object",
                "<embed",
                "javascript:",
                " onload=",
                " onerror=",
                " onclick=",
                "src=\"http",
                "url(http",
            ] {
                assert!(!lower.contains(live), "{name}: `{live}` survived: {html}");
            }
        }
        let css = sanitized.styles.to_ascii_lowercase();
        assert!(!css.contains("url(http"), "{name}: a remote url() in {css}");
        assert!(!css.contains("@import"), "{name}: an @import in {css}");
    }
}
