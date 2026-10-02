//! Zoom (spec 006 FR-021, SC-009): at every step, nothing is lost off the
//! side, and zoom narrows the effective width the way a browser's does.

use crate::support::{LIGHT, render, request};
use postio_model::test_corpus;

const STEPS: [f64; 13] = [
    0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0,
];

#[test]
fn every_step_keeps_every_cluster_inside_the_document() {
    let mut failures = Vec::new();
    for fixture in test_corpus::all() {
        let Some(base) = request(fixture.name(), LIGHT) else {
            continue;
        };
        for zoom in STEPS {
            let mut request = base.clone();
            request.viewport.zoom = zoom;
            let doc = render(&request);
            if let Some(off) = doc
                .text
                .clusters
                .iter()
                .find(|c| c.rect.x1 > doc.size.width + 1.0 || c.rect.x0 < -1.0)
            {
                failures.push(format!(
                    "{} at {:.0}%: {:?} at {:?}, document {} wide",
                    fixture.name(),
                    zoom * 100.0,
                    doc.text.slice(off.range.clone()),
                    off.rect,
                    doc.size.width
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// FR-021a: 200% in an 800px pane lays out 400 CSS px, so the responsive
/// columns stack as they do in a narrow window.
#[test]
fn zoom_narrows_the_effective_width() {
    let x_of = |doc: &postio_render::RenderedDocument, text: &str| {
        doc.text.rects(doc.text.find(text)[0].clone())[0].x0
    };
    let mut request = request("html-responsive-media", LIGHT).expect("a body");
    let flat = render(&request);
    assert!(
        x_of(&flat, "Bus times") - x_of(&flat, "Market hall") > 250.0,
        "side by side at 100%"
    );
    request.viewport.zoom = 2.0;
    let zoomed = render(&request);
    assert!(
        (x_of(&zoomed, "Market hall") - x_of(&zoomed, "Bus times")).abs() < 1.0,
        "the columns did not stack at 200%"
    );
    // The view is still the pane's width; the text is twice the size.
    assert!(
        (zoomed.size.width - 800.0).abs() < 1.0,
        "the zoomed document is {} wide",
        zoomed.size.width
    );
}
