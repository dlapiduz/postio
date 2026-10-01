//! The two treatments, drawn (specs/007-postio-focus T211, T212): app
//! colours holds every run to the contrast floor in the app's own ink, and
//! paper is drawn exactly as sent on a white sheet, dimmed to 92% in dark
//! and never recoloured. And a paper layout wider than its column is
//! zoomed to fit, down to 0.85.
//!
//! These read what the renderer drew -- each run's colour and the ground
//! painted behind it, and the raster's pixels -- not what the document
//! asked for.

mod support;

use postio_body::RemoteImages;
use postio_body::treatment::Treatment;
use postio_model::test_corpus;
use postio_render::theme::{contrast, relative_luminance};
use postio_render::{Presentation, RenderedDocument, Rgb, Theme};
use postio_ui::reader::document;
use support::{DARK, LIGHT, render, request_for};

/// A fixture drawn under `chosen` (or the rule's choice), in `theme`, in a
/// column `width` CSS pixels wide.
fn drawn(name: &str, chosen: Option<Treatment>, theme: Theme, width: f64) -> RenderedDocument {
    let body = postio_model::mime::parse(test_corpus::load(name).bytes()).body;
    let rendered = document::body_html_treated(&body, RemoteImages::Blocked, chosen, None);
    let treated = rendered.treated.expect("drawn under a treatment");
    let html = document::document_for_treated(
        &rendered.html,
        &rendered.styles,
        RemoteImages::Blocked,
        treated.shown,
    );
    // As the open message's column draws it: the body has no padding of
    // its own there, so the column is the view's width.
    let html = html.replacen("</style>", "body { padding: 0; }</style>", 1);
    let mut request = request_for(html, theme);
    request.viewport.width = width;
    render(&request)
}

fn text_of(doc: &RenderedDocument, cluster: &postio_render::Cluster) -> String {
    doc.text
        .text
        .chars()
        .skip(cluster.range.start)
        .take(cluster.range.len())
        .collect()
}

/// The first drawn character of `words`: clusters are per glyph.
fn cluster_of<'a>(doc: &'a RenderedDocument, words: &str) -> &'a postio_render::Cluster {
    let at = doc
        .text
        .text
        .find(words)
        .unwrap_or_else(|| panic!("`{words}` is not in the text"));
    let at = doc.text.text[..at].chars().count();
    doc.text
        .clusters
        .iter()
        .find(|c| c.range.start >= at)
        .unwrap_or_else(|| panic!("`{words}` is not drawn"))
}

fn pixel(doc: &RenderedDocument, x: f64, y: f64) -> [u8; 3] {
    let raster = postio_render::rasterize(doc);
    let i = ((y.round() as usize) * raster.width as usize + x.round() as usize) * 4;
    [raster.rgba[i], raster.rgba[i + 1], raster.rgba[i + 2]]
}

fn hex(value: &str) -> Rgb {
    let digit = |at: usize| u8::from_str_radix(&value[at..at + 2], 16).expect("hex");
    Rgb::from_u8(digit(1), digit(3), digit(5))
}

#[test]
fn app_colours_draws_office_mail_in_ink_and_every_run_reads() {
    for theme in [LIGHT, DARK] {
        let doc = drawn("html-work-black-text", None, theme, 480.0);
        assert!(!doc.text.clusters.is_empty());
        for cluster in &doc.text.clusters {
            let ratio = contrast(cluster.color, cluster.painted_ground);
            assert!(
                ratio >= 4.5 - 1e-3,
                "{:?} in dark={}: {ratio:.2} -- {:?} on {:?}",
                text_of(&doc, cluster),
                theme.dark,
                cluster.color,
                cluster.painted_ground
            );
        }
        // The ground behind the words is the reader's, never the sender's
        // tinted header cells.
        let grounds: std::collections::HashSet<[u8; 3]> = doc
            .text
            .clusters
            .iter()
            .map(|c| c.painted_ground.to_u8())
            .collect();
        assert_eq!(grounds.len(), 1, "more than one ground: {grounds:?}");
    }
}

#[test]
fn a_coloured_sentence_keeps_its_red_where_it_reads_and_is_ink_where_not() {
    let red = hex("#c00000");
    let colour_of = |theme: Theme| {
        let doc = drawn("html-work-black-text", None, theme, 480.0);
        let cluster = cluster_of(&doc, "Please confirm").clone();
        (cluster.color, cluster.painted_ground)
    };
    let (light, _) = colour_of(LIGHT);
    assert_eq!(light.to_u8(), red.to_u8(), "red reads on the light surface");
    let (dark, ground) = colour_of(DARK);
    assert_ne!(dark.to_u8(), red.to_u8(), "red does not read on dark");
    assert!(contrast(dark, ground) >= 4.5, "and what replaced it does");
    assert!(
        relative_luminance(dark) > 0.5,
        "it is the ink, not a lightened red: {dark:?}"
    );
}

#[test]
fn paper_is_drawn_as_sent_and_dimmed_to_92_percent_in_dark() {
    let page = [0xf6, 0xf1, 0xe7];
    let corner = |theme: Theme| {
        let doc = drawn("html-newsletter-own-page", None, theme, 640.0);
        assert_eq!(doc.messages.len(), 1);
        let sheet = doc.messages[0].rect;
        (pixel(&doc, sheet.x0 + 10.0, sheet.y0 + 10.0), doc)
    };
    let (light, light_doc) = corner(LIGHT);
    assert_eq!(light, page, "the sender's page, exactly, in light");
    assert_eq!(light_doc.counts.repaired_runs, 0, "nothing was recoloured");
    let (dark, dark_doc) = corner(DARK);
    for (channel, (got, sent)) in dark.iter().zip(page).enumerate() {
        let want = f64::from(sent) * 0.92;
        assert!(
            (f64::from(*got) - want).abs() <= 1.5,
            "channel {channel}: {got}, wanted {want:.1} (92% of {sent}): never inverted"
        );
    }
    assert_eq!(dark_doc.counts.repaired_runs, 0, "nothing was recoloured");
    assert_eq!(dark_doc.messages[0].presentation, Presentation::Paper);
    // The words are the sender's, unchanged, in both.
    let colours = |doc: &RenderedDocument| {
        doc.text
            .clusters
            .iter()
            .map(|c| c.color.to_u8())
            .collect::<Vec<_>>()
    };
    assert_eq!(colours(&light_doc), colours(&dark_doc));
}

#[test]
fn paper_with_no_page_of_its_own_is_a_white_sheet() {
    for (theme, want) in [(LIGHT, 255.0), (DARK, 255.0 * 0.92)] {
        let doc = drawn("html-receipt-fixed-width", None, theme, 640.0);
        let sheet = doc.messages[0].rect;
        let [r, g, b] = pixel(&doc, sheet.x0 + 6.0, sheet.y0 + 6.0);
        for got in [r, g, b] {
            assert!(
                (f64::from(got) - want).abs() <= 1.5,
                "dark={}: {got}, wanted {want}",
                theme.dark
            );
        }
    }
}

#[test]
fn correspondence_switched_to_paper_is_the_senders_black_on_white() {
    let doc = drawn("html-work-black-text", Some(Treatment::Paper), DARK, 640.0);
    let cluster = cluster_of(&doc, "Hi everyone");
    assert_eq!(
        cluster.color.to_u8(),
        [0, 0, 0],
        "the sender's black, not the app's ink"
    );
    let sheet = doc.messages[0].rect;
    assert!(
        cluster.rect.x0 >= sheet.x0 + 23.0,
        "the letter is inset from the sheet's edge: {:?} in {sheet:?}",
        cluster.rect
    );
}

#[test]
fn a_paper_layout_wider_than_its_column_is_zoomed_to_fit() {
    // The newsletter is a 640px table: in a 608px column (a 1024px window)
    // it is drawn at 95%, whole; in 700 it fits; in 480 it would need 75%,
    // under the floor, so it stays at 85% and the view scrolls the rest.
    let at = |width: f64| drawn("html-newsletter-own-page", None, LIGHT, width);
    let fitted = at(608.0);
    assert!(
        (fitted.fit - 608.0 / 640.0).abs() < 0.01,
        "fit {}",
        fitted.fit
    );
    assert!(
        fitted.size.width <= 608.0 + 0.5,
        "still wider than the column: {}",
        fitted.size.width
    );
    assert_eq!(at(700.0).fit, 1.0);
    let floor = at(480.0);
    assert!((floor.fit - postio_render::render::PAPER_FIT_FLOOR).abs() < 1e-9);
    assert!(
        floor.size.width > 480.0,
        "below the floor it scrolls sideways"
    );
    // App colours is never zoomed: it reflows.
    let app = drawn("html-work-black-text", None, LIGHT, 400.0);
    assert_eq!(app.fit, 1.0);
}

#[test]
fn the_fit_is_the_columns_share_clamped_to_the_floor() {
    use postio_render::render::{PAPER_FIT_FLOOR, fit_scale};
    assert_eq!(fit_scale(600.0, 640.0), 1.0);
    assert_eq!(fit_scale(640.3, 640.0), 1.0, "half a pixel is not a reason");
    assert!((fit_scale(640.0, 608.0) - 0.95).abs() < 1e-9);
    assert_eq!(fit_scale(1000.0, 480.0), PAPER_FIT_FLOOR);
}
