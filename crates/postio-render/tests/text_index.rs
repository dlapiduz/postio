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

/// A translucent ground is part of what is behind the text: a search
/// match's tint over a white page is a tint, not white.
#[test]
fn a_translucent_ground_is_composited_over_the_page() {
    let doc = render_html(
        "<!DOCTYPE html><html><body style=\"background:#fff\">\
         <p>plain <span style=\"background:rgba(0,0,255,0.5)\">tinted</span></p>\
         </body></html>"
            .to_owned(),
    );
    let ground_of = |needle: &str| {
        let range = range_of(&doc.text, needle);
        doc.text
            .clusters
            .iter()
            .find(|c| c.range.contains(&range.start))
            .map(|c| c.painted_ground)
            .expect("a cluster")
    };
    let tinted = ground_of("tinted");
    assert!((tinted.r - 0.5).abs() < 0.01, "{tinted:?}");
    assert!((tinted.b - 1.0).abs() < 0.01, "{tinted:?}");
    let plain = ground_of("plain");
    assert!((plain.r - 1.0).abs() < 0.01, "{plain:?}");
}

/// One row of the locator's table (spec 007 T066, research R2): a message,
/// the sentence a detector stored from it, and what the reader must
/// highlight.
struct Row {
    /// What the row is about.
    case: &'static str,
    /// The message's HTML part, if it has one.
    html: Option<&'static str>,
    /// Its plain part, if it has one.
    text: Option<&'static str>,
    /// The text the detector read, when it is not what the reader draws.
    read: Option<&'static str>,
    /// The sentence, as the detector stored it.
    excerpt: &'static str,
    /// Which occurrence of it, in what was read, the detector stored.
    nth: usize,
    /// What the highlight reads, and which occurrence of that in the drawn
    /// text it must be. `None` when the reader does not draw the sentence.
    drawn: Option<(&'static str, usize)>,
}

const FILLER: &str = "The venue moved to the north hall, and parking is behind the \
                      library this time, so leave a few minutes more than usual.";

fn rows() -> Vec<Row> {
    let row = |case, html, excerpt, drawn| Row {
        case,
        html: Some(html),
        text: None,
        read: None,
        excerpt,
        nth: 0,
        drawn,
    };
    vec![
        row(
            "a sentence drawn once",
            "<p>Thanks for the notes. Could you send the signed lease by Friday?</p>",
            "Could you send the signed lease by Friday?",
            Some(("Could you send the signed lease by Friday?", 0)),
        ),
        Row {
            nth: 0,
            ..row(
                "the first of two, told apart by where it sat",
                said_twice(),
                "Can you call me back?",
                Some(("Can you call me back?", 0)),
            )
        },
        Row {
            nth: 1,
            ..row(
                "the second of two, told apart by where it sat",
                said_twice(),
                "Can you call me back?",
                Some(("Can you call me back?", 1)),
            )
        },
        Row {
            nth: 1,
            ..row(
                "the middle of three",
                "<p>Please confirm.</p><p>The first draft is attached.</p>\
                 <p>Please confirm.</p><p>The second draft follows on Monday.</p>\
                 <p>Please confirm.</p>",
                "Please confirm.",
                Some(("Please confirm.", 1)),
            )
        },
        Row {
            text: Some("can you book the cafe in zurich for tuesday?"),
            read: Some("can you book the cafe in zurich for tuesday?"),
            ..row(
                "case and diacritics other than drawn: read from a plain part \
                 written without them",
                "<p>Can you book the Café in Zürich for Tuesday?</p>",
                "can you book the cafe in zurich for tuesday?",
                Some(("Can you book the Café in Zürich for Tuesday?", 0)),
            )
        },
        row(
            "accented text before it does not move the range",
            "<p>Réunion à Genève, déjà prévue, café compris.</p>\
             <p>Would you forward the agenda to Zoë?</p>",
            "Would you forward the agenda to Zoë?",
            Some(("Would you forward the agenda to Zoë?", 0)),
        ),
        Row {
            html: None,
            text: Some(
                "Hi Ines,\n\nCould you look over the draft\nbefore the call on Monday?\n\nThanks",
            ),
            ..row(
                "a hard-wrapped plain-text line, drawn as one",
                "",
                "Could you look over the draft\nbefore the call on Monday?",
                Some((
                    "Could you look over the draft before the call on Monday?",
                    0,
                )),
            )
        },
        row(
            "a sentence across a table-cell boundary",
            "<table><tr><td>Please sign</td><td>the attached lease today.</td></tr></table>",
            "Please sign the attached lease today.",
            Some(("Please sign\tthe attached lease today.", 0)),
        ),
        Row {
            read: Some("Could you send the slides tonight?"),
            ..row(
                "a sentence the message does not draw",
                "<p>See you on Thursday.</p>",
                "Could you send the slides tonight?",
                None,
            )
        },
        row(
            "a sentence in a hidden preheader, never drawn",
            "<div style=\"display:none\">Could you reply by noon today, please?</div>\
             <p>Your order has shipped.</p>",
            "Could you reply by noon today, please?",
            None,
        ),
        Row {
            read: Some(" \n\t "),
            ..row(
                "an excerpt of nothing but whitespace",
                "<p>Anything at all.</p>",
                " \n\t ",
                None,
            )
        },
    ]
}

/// A body that asks the same question twice, with prose between and after.
fn said_twice() -> &'static str {
    static HTML: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    HTML.get_or_init(|| {
        format!(
            "<p>Can you call me back?</p><p>{FILLER}</p><p>{FILLER}</p>\
             <p>Can you call me back?</p><p>{FILLER}</p>"
        )
    })
}

/// A body drawn the way the single-message reader draws it.
fn drawn(html: Option<&str>, text: Option<&str>) -> RenderedDocument {
    let body = postio_model::MessageBody {
        html: html.map(str::to_owned),
        text: text.map(str::to_owned),
    };
    let rendered = document::body_html_in(&body, RemoteImages::Blocked, Rendering::Original, None);
    render_html(document::document_for(
        &rendered.html,
        &rendered.styles,
        RemoteImages::Blocked,
        document::sheet_for(Rendering::Original, false),
    ))
}

/// What a detector reads (spike S5's "what is drawn"): the HTML flattened
/// when there is HTML, else the plain part.
fn read_from(html: Option<&str>, text: Option<&str>) -> String {
    html.map(|html| postio_body::parse(html).to_search_text())
        .or(text.map(str::to_owned))
        .unwrap_or_default()
        .replace("\r\n", "\n")
}

/// The char offset of the `nth` occurrence of `needle` in `haystack`.
fn nth_offset(haystack: &str, needle: &str, nth: usize) -> usize {
    let byte = haystack
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("{needle:?} is not in {haystack:?} {} times", nth + 1))
        .0;
    haystack[..byte].chars().count()
}

#[test]
fn an_excerpt_is_located_where_the_reader_draws_it() {
    let mut failures = Vec::new();
    for row in rows() {
        let html = row.html.filter(|html| !html.is_empty());
        let document = drawn(html, row.text);
        let read = row
            .read
            .map(str::to_owned)
            .unwrap_or_else(|| read_from(html, row.text));
        let excerpt = postio_render::Excerpt {
            text: row.excerpt,
            offset: nth_offset(&read, row.excerpt, row.nth),
            source_len: read.chars().count(),
        };
        let found = document.text.locate(excerpt);
        let want = row.drawn.map(|(words, nth)| {
            let start = nth_offset(&document.text.text, words, nth);
            start..start + words.chars().count()
        });
        if found != want {
            failures.push(format!(
                "{}: located {:?} ({:?}), wanted {want:?} ({:?}), in {:?}",
                row.case,
                found,
                found.clone().map(|range| document.text.slice(range)),
                row.drawn.map(|(words, _)| words),
                document.text.text,
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
