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
    let doc = postio_render::render(&request);
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
