//! SC-002: designed mail looks the way its sender built it. Each designed
//! fixture with a reference render is drawn through the production pipeline
//! -- sanitize, compose, render -- at 800px in light, and compared with
//! the reference by `postio_test_support::fidelity`
//! (`contracts/fidelity-metric.md`).

use std::path::PathBuf;

use crate::support::{LIGHT, render, request};
use postio_model::test_corpus::{self, Category};
use postio_test_support::fidelity::{self, Image};

/// The references are the message alone, so the reader's own frame around
/// it -- its padding, border and margins -- is set aside for the comparison
/// (research R13; the evaluation's S1 did the same for both engines).
const NEUTRAL_CHROME: &str = "html, body { background: #ffffff !important; margin: 0 !important; padding: 0 !important; }\
 .postio-body { padding: 8px !important; border: 0 !important; border-radius: 0 !important; margin: 0 !important; max-width: none !important; min-height: 0 !important; }\
 .postio-body:has(> .postio-canvas) { padding: 0 !important; }\
 .postio-canvas { margin: 8px; padding: 0; border-radius: 0; }";

fn references() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../postio-test-support/data/reference")
}

#[test]
fn designed_mail_matches_its_references() {
    let mismatches =
        std::fs::read_to_string(references().join("MISMATCHES.md")).unwrap_or_default();
    let (mut judged, mut matched) = (0, 0);
    let mut failures = Vec::new();
    for fixture in test_corpus::by_category(Category::Designed) {
        let path = references().join(format!("{}.png", fixture.name()));
        if !path.exists() {
            continue;
        }
        let mut request = request(fixture.name(), LIGHT).expect("a body");
        request.document = request.document.replacen(
            "</head>",
            &format!("<style>{NEUTRAL_CHROME}</style></head>"),
            1,
        );
        let raster = postio_render::rasterize(&render(&request));
        let candidate =
            Image::from_rgba(raster.width as usize, raster.height as usize, raster.rgba);
        let comparison = fidelity::compare(&Image::load_png(&path), &candidate);
        judged += 1;
        if comparison.matches() {
            matched += 1;
        } else {
            let listed = mismatches
                .lines()
                .any(|line| line.contains(fixture.name()) && line.contains("cosmetic"));
            if !listed {
                failures.push(format!(
                    "{}: {:.1}% agreeing, lost block {}, height ok {}",
                    fixture.name(),
                    100.0 * comparison.agreeing as f64 / comparison.cells.max(1) as f64,
                    comparison.lost_block,
                    comparison.height_ok
                ));
            }
        }
    }
    assert!(judged >= 6, "only {judged} references were found");
    let rate = f64::from(matched) / f64::from(judged);
    assert!(
        rate >= 0.95 && failures.is_empty(),
        "{matched} of {judged} match; not listed as cosmetic in MISMATCHES.md:\n{}",
        failures.join("\n")
    );
}
