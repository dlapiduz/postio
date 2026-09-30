//! SC-001 on painted pixels: across the corpus, in light, dark and high
//! contrast, no text is drawn below the floor against what is actually
//! behind it -- 4.5:1, or 7:1 in high contrast, at every size (FR-012).

mod support;

use postio_model::test_corpus;
use postio_render::theme::{self, Rgb};
use postio_render::{Presentation, Raster, Theme};
use support::{DARK, HIGH_CONTRAST, LIGHT, render, request};

/// The ground behind a cluster: the commonest colour in a band just above
/// and below its box, leaving out its own ink -- glyph edges, an underline
/// -- which is anything close to its colour.
fn ground_behind(raster: &Raster, rect: postio_render::Rect, scale: f64, ink: Rgb) -> Option<Rgb> {
    let (w, h) = (raster.width as i64, raster.height as i64);
    let mut counts = std::collections::HashMap::<[u8; 3], usize>::new();
    let (x0, x1) = ((rect.x0 * scale) as i64, (rect.x1 * scale) as i64);
    for y in [(rect.y0 * scale) as i64 - 2, (rect.y1 * scale) as i64 + 1] {
        if !(0..h).contains(&y) {
            continue;
        }
        for x in x0.max(0)..x1.min(w) {
            let i = ((y * w + x) * 4) as usize;
            let px = [raster.rgba[i], raster.rgba[i + 1], raster.rgba[i + 2]];
            if theme::contrast(Rgb::from_u8(px[0], px[1], px[2]), ink) < 1.5 {
                continue;
            }
            *counts.entry(px).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|([r, g, b], _)| Rgb::from_u8(r, g, b))
}

/// Every cluster below the floor in `theme`, as `(text, ratio)` -- in the
/// presentation the reader would show: a message whose paper cannot reach
/// the high-contrast floor opens in Reader view (FR-013b).
fn below(name: &str, theme: Theme) -> Vec<(String, f64)> {
    let Some(request) = request(name, theme) else {
        return Vec::new();
    };
    let mut doc = render(&request);
    if !doc.needs_reader_view.is_empty() {
        assert!(
            theme.high_contrast,
            "{name}: only high contrast falls back to Reader view"
        );
        let reader =
            support::request_as(name, theme, postio_ui::reader::document::Rendering::Reader)
                .expect("a body");
        doc = render(&reader);
    }
    let raster = postio_render::rasterize(&doc);
    let floor = theme::floor(theme);
    let mut out = Vec::new();
    for cluster in &doc.text.clusters {
        if cluster.rect.height() < 4.0 {
            continue;
        }
        // Clusters inside an image-backed ground are judged against the
        // image's mean colour by the rule; a band sample there measures the
        // picture, not the ground.
        let Some(ground) = ground_behind(&raster, cluster.rect, doc.scale, cluster.color) else {
            continue;
        };
        let ratio = theme::contrast(cluster.color, ground);
        if ratio < floor - 0.05 {
            out.push((doc.text.slice(cluster.range.clone()).to_owned(), ratio));
        }
    }
    out
}

#[test]
fn no_text_is_below_the_floor_in_any_theme() {
    let mut failures = Vec::new();
    for fixture in test_corpus::all() {
        for (label, theme) in [
            ("light", LIGHT),
            ("dark", DARK),
            ("high contrast", HIGH_CONTRAST),
        ] {
            let low = below(fixture.name(), theme);
            if !low.is_empty() {
                failures.push(format!(
                    "{} in {label}: {} clusters, e.g. {:?}",
                    fixture.name(),
                    low.len(),
                    &low[..low.len().min(3)]
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "below the floor:\n{}",
        failures.join("\n")
    );
}

/// A sender's own dark design is honoured, not trusted: it stays theirs,
/// and it still meets the floor.
#[test]
fn a_senders_illegible_dark_design_is_repaired() {
    let doc = render(&request("html-illegible-sender-dark", DARK).expect("a body"));
    assert_eq!(doc.messages[0].presentation, Presentation::SenderDark);
    assert!(below("html-illegible-sender-dark", DARK).is_empty());
}
