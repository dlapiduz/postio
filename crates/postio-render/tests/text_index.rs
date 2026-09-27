//! The text index (research R7): the one reading-order serialisation that
//! copy, find and the screen reader share, with the geometry to act on it.

use std::sync::Arc;

use postio_body::RemoteImages;
use postio_model::test_corpus;
use postio_render::{RenderRequest, RenderedDocument, Resources, TextIndex, Theme, Viewport};
use postio_ui::reader::document::{self, Rendering};

fn fonts() -> &'static postio_render::fonts::FontSet {
    static FONTS: std::sync::OnceLock<postio_render::fonts::FontSet> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        postio_render::fonts::FontSet::new(postio_render::fonts::Bundled {
            faces: document::FACES.iter().map(|face| face.bytes).collect(),
            sans: "Barlow",
            mono: "IBM Plex Mono",
        })
    })
}

fn render_html(html: String) -> RenderedDocument {
    let request = RenderRequest {
        generation: 1,
        document: html,
        plain_text: String::new(),
        over_cap: None,
        resources: Arc::new(Resources::new()),
        viewport: Viewport {
            width: 800.0,
            hidpi_scale: 1.0,
            zoom: 1.0,
        },
        theme: Theme::default(),
        darkened: Vec::new(),
        toggled_folds: Vec::new(),
        reader_view: Vec::new(),
    };
    postio_render::render(&request, fonts())
}

fn receipt() -> RenderedDocument {
    let parsed = postio_model::mime::parse(test_corpus::load("html-transactional-receipt").bytes());
    let rendered = document::body_html_in(
        &parsed.body,
        RemoteImages::Blocked,
        Rendering::Original,
        None,
    );
    let sheet = document::sheet_for(Rendering::Original, false);
    render_html(document::document_for(
        &rendered.html,
        &rendered.styles,
        RemoteImages::Blocked,
        sheet,
    ))
}

/// The char range of the first `needle` in the index's text.
fn range_of(index: &TextIndex, needle: &str) -> std::ops::Range<usize> {
    let byte = index
        .text
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} is not in {:?}", index.text));
    let start = index.text[..byte].chars().count();
    start..start + needle.chars().count()
}

#[test]
fn a_table_reads_as_tab_separated_cells_in_newline_separated_rows() {
    let doc = receipt();
    let text = &doc.text.text;
    assert!(text.contains("Item\tQty\tPrice\n"), "{text:?}");
    assert!(
        text.contains("The Salt Road, paperback\t1\t14.00\n"),
        "{text:?}"
    );
    assert!(
        text.contains("Field Guide to Lichens\t2\t38.00\n"),
        "{text:?}"
    );
    // An empty cell still holds its column.
    assert!(text.contains("Total\t\t54.50"), "{text:?}");
}

#[test]
fn a_hidden_preheader_is_not_read() {
    let doc = receipt();
    assert!(doc.text.text.contains("Order 4410"), "{:?}", doc.text.text);
    assert!(!doc.text.text.contains("Preheader"), "{:?}", doc.text.text);
}

#[test]
fn an_image_is_read_as_its_alt_text() {
    let doc = receipt();
    // Once as the logo's alt, once as the heading beside it.
    assert_eq!(
        doc.text.text.matches("Fernhill Books").count(),
        2,
        "{:?}",
        doc.text.text
    );
}

#[test]
fn hit_at_a_glyph_returns_its_offset() {
    let doc = receipt();
    let order = range_of(&doc.text, "Order 4410").start;
    let cluster = doc
        .text
        .clusters
        .iter()
        .find(|c| c.range.contains(&order))
        .expect("the O of Order is drawn");
    assert_eq!(
        doc.text.hit(cluster.rect.center()),
        Some(cluster.range.start)
    );
}

#[test]
fn word_and_line_ranges() {
    let doc = receipt();
    let lichens = range_of(&doc.text, "Lichens");
    assert_eq!(doc.text.word_at(lichens.start + 2), lichens);
    let sentence = range_of(&doc.text, "Order 4410, placed 21 September 2026.");
    assert_eq!(doc.text.line_at(sentence.start + 3), sentence);
}

#[test]
fn rects_of_a_range_lie_inside_its_line() {
    let doc = receipt();
    let range = range_of(&doc.text, "Order 4410");
    let rects = doc.text.rects(range.clone());
    assert!(!rects.is_empty());
    let line = doc.text.rects(doc.text.line_at(range.start));
    for rect in rects {
        assert!(
            line.iter().any(|l| l.union(rect) == *l),
            "{rect:?} is outside the line {line:?}"
        );
    }
}

#[test]
fn slice_is_the_text_of_the_range() {
    let doc = receipt();
    let range = range_of(&doc.text, "Gift wrap");
    assert_eq!(doc.text.slice(range), "Gift wrap");
}

#[test]
fn find_folds_case_and_diacritics() {
    let doc = render_html(
        "<!DOCTYPE html><html><body><p>Total</p><p>tötal</p><p>totally</p></body></html>"
            .to_owned(),
    );
    let found: Vec<&str> = doc
        .text
        .find("TOTAL")
        .into_iter()
        .map(|range| doc.text.slice(range))
        .collect();
    assert_eq!(found, ["Total", "tötal", "total"]);
}

/// The index's geometry is in the view's pixels whatever the device scale:
/// Blitz shapes text at the device scale, and a cluster's rect must not
/// carry it (a 2x display drew highlights twice the size and position).
#[test]
fn cluster_rects_do_not_carry_the_device_scale() {
    let at = |scale: f64| {
        let mut request = RenderRequest {
            generation: 1,
            document: "<!DOCTYPE html><html><body><p>Hello there, the quick fox.</p></body></html>"
                .to_owned(),
            plain_text: String::new(),
            over_cap: None,
            resources: Arc::new(Resources::new()),
            viewport: Viewport {
                width: 400.0,
                hidpi_scale: scale,
                zoom: 1.0,
            },
            theme: Theme::default(),
            darkened: Vec::new(),
            toggled_folds: Vec::new(),
            reader_view: Vec::new(),
        };
        request.viewport.hidpi_scale = scale;
        postio_render::render(&request, fonts())
    };
    let (one, two) = (at(1.0), at(2.0));
    let rect = |doc: &RenderedDocument| doc.text.rects(doc.text.find("quick")[0].clone())[0];
    let (a, b) = (rect(&one), rect(&two));
    assert!(
        (a.x0 - b.x0).abs() < 1.0 && (a.x1 - b.x1).abs() < 1.0,
        "{a:?} at 1x, {b:?} at 2x"
    );
}
