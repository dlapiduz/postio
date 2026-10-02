//! The two treatments, drawn (specs/007-postio-focus T211, T212): app
//! colours holds every run to the contrast floor in the app's own ink, and
//! paper is drawn exactly as sent on a white sheet, dimmed to 92% in dark
//! and never recoloured. And a paper layout wider than its column is
//! zoomed to fit, down to 0.85.
//!
//! These read what the renderer drew -- each run's colour and the ground
//! painted behind it, and the raster's pixels -- not what the document
//! asked for.

use crate::support::{DARK, LIGHT, render, request_for};
use postio_body::RemoteImages;
use postio_body::treatment::Treatment;
use postio_model::test_corpus;
use postio_render::theme::{contrast, relative_luminance};
use postio_render::{Presentation, RenderedDocument, Rgb, Theme};
use postio_ui::reader::document;

/// A fixture drawn under `chosen` (or the rule's choice), in `theme`, in a
/// column `width` CSS pixels wide.
fn drawn(name: &str, chosen: Option<Treatment>, theme: Theme, width: f64) -> RenderedDocument {
    drawn_with(name, chosen, theme, width, "")
}

/// [`drawn`], with `css` added after the reader's own sheet, as the column
/// adds its palette.
fn drawn_with(
    name: &str,
    chosen: Option<Treatment>,
    theme: Theme,
    width: f64,
    css: &str,
) -> RenderedDocument {
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
    let html = html.replacen(
        "</style>",
        &format!("body {{ padding: 0; }}{css}</style>"),
        1,
    );
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

/// An ordinary newsletter in the app's colours lays out once, in dark as in
/// light (T218). The dark-mode white behind an image is the treatment's
/// stylesheet's, not a mark per `<img>` for a second layout: that doubled
/// every newsletter's layout in dark, and with the render's other costs
/// took a long one past the reader's deadline into the plain-text
/// fallback.
#[test]
fn an_app_colours_newsletter_with_images_lays_out_once() {
    for theme in [LIGHT, DARK] {
        // The column supplies its own accent, the system's, which reads on
        // its ground (`FLOW_PALETTE`); the generated palette's light accent
        // does not reach 4.5:1 on its ground, and every link would be
        // repaired -- a second layout this test is not about.
        let accent = if theme.dark { "#78aeed" } else { "#0461be" };
        let doc = drawn_with(
            "html-newsletter-many-tables",
            None,
            theme,
            480.0,
            &format!(":root {{ --r-accent: {accent}; }}"),
        );
        assert_eq!(
            doc.messages[0].presentation,
            if theme.dark {
                Presentation::Adapted
            } else {
                Presentation::Styled
            },
            "not drawn in app colours"
        );
        assert_eq!(
            doc.counts.style_passes, 1,
            "dark: {}: laid out more than once",
            theme.dark
        );
    }
}

/// Where `words` start drawing, with the cluster at exactly that character:
/// [`cluster_of`] tolerates a word missing from the drawing by taking the
/// next one drawn, which is the very failure this asks about.
fn drawn_at(doc: &RenderedDocument, words: &str) -> kurbo::Rect {
    let at = doc
        .text
        .text
        .find(words)
        .unwrap_or_else(|| panic!("`{words}` is not in the text"));
    let at = doc.text.text[..at].chars().count();
    let cluster = doc
        .text
        .clusters
        .iter()
        .find(|c| c.range.start == at)
        .unwrap_or_else(|| panic!("`{words}` is in the text but was not drawn"));
    assert!(
        cluster.rect.width() > 0.0 && cluster.rect.height() > 0.0,
        "`{words}` was drawn with no size: {:?}",
        cluster.rect
    );
    cluster.rect
}

/// T222: a responsive newsletter stacks its columns by making its cells
/// `display:block` under `@media (max-width: 600px)`, which the 480px
/// app-colours column satisfies. The engine dropped every such cell, so
/// all but the header and footer vanished. Every column is drawn, stacked
/// under its neighbour at 480px and beside it at 700px, where the query
/// does not apply.
#[test]
fn app_colours_draws_the_stacked_cells_of_a_responsive_newsletter() {
    let name = "html-responsive-stacked-cells";
    let sections = [
        (
            "Orchard walk",
            "Saplings planted",
            "Cider press",
            "The press is mended",
        ),
        (
            "Library hours",
            "The reading room",
            "Map archive",
            "Sixty survey sheets",
        ),
        (
            "Ferry notice",
            "The morning crossing",
            "Harbour lights",
            "New lamps",
        ),
    ];
    let narrow = drawn(name, Some(Treatment::AppColours), LIGHT, 480.0);
    let wide = drawn(name, Some(Treatment::AppColours), LIGHT, 700.0);
    for (left, left_text, right, right_text) in sections {
        let [l, lt, r, rt] = [left, left_text, right, right_text].map(|w| drawn_at(&narrow, w));
        assert!(
            lt.y0 > l.y0 && r.y0 > lt.y0 && rt.y0 > r.y0,
            "{left}: not stacked at 480px"
        );
        assert!(
            (r.x0 - l.x0).abs() < 1.0,
            "{right} starts at {} but {left} at {}: stacked cells share the column's left edge",
            r.x0,
            l.x0
        );
        let [l, r] = [left, right].map(|w| drawn_at(&wide, w));
        assert!(
            (r.y0 - l.y0).abs() < 1.0 && r.x0 > l.x0 + 200.0,
            "{right} is not beside {left} at 700px"
        );
    }
    // Each stacked column has the whole width: its paragraph runs past the
    // middle of the column, where a half-width cell would have wrapped it.
    let paragraph = "The press is mended and the first pressing is booked for the twelfth.";
    let at = narrow
        .text
        .text
        .find(paragraph)
        .expect("the paragraph is in the text");
    let at = narrow.text.text[..at].chars().count();
    let reach = narrow
        .text
        .clusters
        .iter()
        .filter(|c| (at..at + paragraph.chars().count()).contains(&c.range.start))
        .map(|c| c.rect.x1)
        .fold(0.0, f64::max);
    assert!(
        reach > 300.0,
        "the stacked paragraph reaches only {reach}px"
    );
    // Stacked, the page is longer: about 594px at 480 against 408 side by
    // side. Dropping the stacked cells left the header and footer alone.
    let (tall, short) = (narrow.size.height, wide.size.height);
    assert!(
        (520.0..680.0).contains(&tall) && tall > short,
        "{tall}px at 480px and {short}px at 700px"
    );
}
