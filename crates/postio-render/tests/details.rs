//! `<details>` folds, which the reader uses for thread messages and quoted
//! text, toggle by id with no script (research R15, risk R15-a).

use std::sync::Arc;

use postio_render::{RenderRequest, Resources, Theme, Viewport};

const DOCUMENT: &str = "<!DOCTYPE html><html><body style=\"margin:0;background:#fff\">\
    <details data-postio-fold=\"q1\"><summary style=\"height:20px\">quoted text</summary>\
    <div style=\"height:100px;background:#0000ff\"></div></details>\
    <div style=\"height:40px;background:#00ff00\"></div></body></html>";

fn render(toggled: &[&str]) -> (postio_render::RenderedDocument, postio_render::Raster) {
    let request = RenderRequest {
        generation: 1,
        document: DOCUMENT.to_owned(),
        plain_text: String::new(),
        over_cap: None,
        resources: Arc::new(Resources::new()),
        viewport: Viewport {
            width: 200.0,
            hidpi_scale: 1.0,
            zoom: 1.0,
        },
        theme: Theme::default(),
        darkened: Vec::new(),
        toggled_folds: toggled.iter().map(|id| (*id).to_owned()).collect(),
        reader_view: Vec::new(),
    };
    let doc = postio_render::render(&request, fonts());
    let raster = postio_render::rasterize(&doc);
    (doc, raster)
}

fn blue(raster: &postio_render::Raster) -> usize {
    raster
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] < 20 && p[1] < 20 && p[2] > 230)
        .count()
}

/// The first row, from the top, holding any pixel of `rgb`.
fn first_row_of(raster: &postio_render::Raster, want: impl Fn(&[u8; 4]) -> bool) -> Option<u32> {
    let pixels = raster.rgba.as_chunks::<4>().0;
    (0..raster.height)
        .find(|y| (0..raster.width).any(|x| want(&pixels[(y * raster.width + x) as usize])))
}

#[test]
fn a_closed_fold_paints_only_its_summary() {
    let (_, raster) = render(&[]);
    assert_eq!(blue(&raster), 0, "the closed fold's body was painted");
}

#[test]
fn toggling_a_fold_by_id_paints_its_body() {
    let (_, raster) = render(&["q1"]);
    assert!(
        blue(&raster) >= 100 * 190,
        "the opened fold's body was not painted"
    );
}

/// Opening a fold keeps its summary where it was -- the body starts right
/// below the 20px summary -- and moves only what follows, by the body's
/// height. The summary's own pixels may differ: its disclosure marker
/// turns.
#[test]
fn opening_a_fold_moves_only_what_is_below_its_summary() {
    let green = |p: &[u8; 4]| p[0] < 20 && p[1] > 230 && p[2] < 20;
    let (closed_doc, closed) = render(&[]);
    let (open_doc, open) = render(&["q1"]);
    let blue = |p: &[u8; 4]| p[0] < 20 && p[1] < 20 && p[2] > 230;
    assert_eq!(
        first_row_of(&open, blue),
        Some(20),
        "the summary's height changed"
    );
    let (closed_at, open_at) = (
        first_row_of(&closed, green).expect("the block after the fold"),
        first_row_of(&open, green).expect("the block after the fold"),
    );
    assert_eq!(
        open_at - closed_at,
        100,
        "what follows moved by the body's height"
    );
    assert!(open_doc.size.height - closed_doc.size.height >= 100.0);
}

/// The process's font set: bundled faces plus discovery, built once.
fn fonts() -> &'static postio_render::fonts::FontSet {
    static FONTS: std::sync::OnceLock<postio_render::fonts::FontSet> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        postio_render::fonts::FontSet::new(postio_render::fonts::Bundled {
            faces: postio_ui::reader::document::FACES
                .iter()
                .map(|face| face.bytes)
                .collect(),
            sans: "Barlow",
            mono: "IBM Plex Mono",
        })
    })
}

/// Spec 007 T067 (FR-034): a single message's quoted history reaches the
/// snapshot as a fold with an id, drawn closed under its count, and opens
/// by that id -- what activating it asks of the renderer. Before the ids,
/// a single message's quote had no id to be opened by, and the snapshot
/// did not list it.
#[test]
fn a_single_messages_quote_opens_by_its_id() {
    use postio_body::RemoteImages;
    use postio_ui::reader::document::{self, Rendering, Sheet};

    let body = postio_model::MessageBody {
        text: None,
        html: Some(
            "<p>Agreed, see you then.</p>\
             <blockquote><p>Shall we meet at the quay office?</p>\
             <p>Thursday suits me.</p></blockquote>"
                .to_owned(),
        ),
    };
    let rendered = document::body_html(&body, RemoteImages::Blocked, Rendering::Original);
    let html = document::document_for(
        &rendered.html,
        &rendered.styles,
        RemoteImages::Blocked,
        Sheet::Theme,
    );
    let drawn = |toggled: &[&str]| {
        postio_render::render(
            &RenderRequest {
                generation: 1,
                document: html.clone(),
                plain_text: String::new(),
                over_cap: None,
                resources: Arc::new(Resources::new()),
                viewport: Viewport {
                    width: 600.0,
                    hidpi_scale: 1.0,
                    zoom: 1.0,
                },
                theme: Theme::default(),
                darkened: Vec::new(),
                toggled_folds: toggled.iter().map(|id| (*id).to_owned()).collect(),
                reader_view: Vec::new(),
            },
            fonts(),
        )
    };

    let closed = drawn(&[]);
    let ids: Vec<(&str, bool)> = closed
        .folds
        .iter()
        .map(|fold| (fold.id.as_str(), fold.open))
        .collect();
    assert_eq!(ids, [("q0", false)], "the quote's fold, by id, closed");
    assert!(
        closed.text.text.contains("2 quoted lines"),
        "{:?}",
        closed.text.text
    );
    assert!(
        !closed.text.text.contains("Thursday suits me."),
        "{:?}",
        closed.text.text
    );

    let opened = drawn(&["q0"]);
    assert!(
        opened.folds.iter().all(|fold| fold.open),
        "{:?}",
        opened.folds
    );
    assert!(
        opened.text.text.contains("Thursday suits me."),
        "the opened quote is not drawn: {:?}",
        opened.text.text
    );
}
