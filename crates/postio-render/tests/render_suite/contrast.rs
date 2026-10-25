//! SC-001 on painted pixels: across the corpus, in light, dark and high
//! contrast, no text is drawn below the floor against what is actually
//! behind it -- 4.5:1, or 7:1 in high contrast, at every size (FR-012).

use crate::support;

use crate::support::{DARK, HIGH_CONTRAST, LIGHT, render, request};
use postio_model::test_corpus;
use postio_render::theme::{self, Rgb};
use postio_render::{Presentation, Raster, Theme};

/// The commonest colour in one row of pixels across `x0..x1`, leaving out
/// anything close to `ink`.
fn commonest(raster: &Raster, y: i64, x0: i64, x1: i64, ink: Rgb) -> Option<([u8; 3], usize)> {
    let (w, h) = (raster.width as i64, raster.height as i64);
    if !(0..h).contains(&y) {
        return None;
    }
    let mut counts = std::collections::HashMap::<[u8; 3], usize>::new();
    for x in x0.max(0)..x1.min(w) {
        let i = ((y * w + x) * 4) as usize;
        let px = [raster.rgba[i], raster.rgba[i + 1], raster.rgba[i + 2]];
        if theme::contrast(Rgb::from_u8(px[0], px[1], px[2]), ink) < 1.5 {
            continue;
        }
        *counts.entry(px).or_default() += 1;
    }
    counts.into_iter().max_by_key(|(px, n)| (*n, *px))
}

/// Whether `px` is `ink` anti-aliased over `ground`: on the line between
/// them, give or take rounding.
fn is_fringe(px: [u8; 3], ink: Rgb, ground: [u8; 3]) -> bool {
    let ink = [ink.r, ink.g, ink.b].map(|c| (c * 255.0).round());
    let (px, ground) = (px.map(f64::from), ground.map(f64::from));
    let axis = (0..3)
        .max_by(|&a, &b| {
            (ink[a] - ground[a])
                .abs()
                .total_cmp(&(ink[b] - ground[b]).abs())
        })
        .unwrap_or(0);
    if (ink[axis] - ground[axis]).abs() < 1.0 {
        return false;
    }
    let t = (px[axis] - ground[axis]) / (ink[axis] - ground[axis]);
    (0.0..=1.0).contains(&t)
        && (0..3).all(|c| (px[c] - (ground[c] + t * (ink[c] - ground[c]))).abs() <= 6.0)
}

/// The ground behind a cluster: the commonest colour in a band just above
/// and below its box, leaving out its own ink -- glyph edges, an underline
/// -- which is anything close to its colour. A row below the box that is
/// only the anti-aliased fringe of an underline over the row above is the
/// underline, not a second ground: where a link's underline falls against
/// the box moves with the font's metrics, and the ground must not.
fn ground_behind(raster: &Raster, rect: postio_render::Rect, scale: f64, ink: Rgb) -> Option<Rgb> {
    let (x0, x1) = ((rect.x0 * scale) as i64, (rect.x1 * scale) as i64);
    let above = commonest(raster, (rect.y0 * scale) as i64 - 2, x0, x1, ink);
    let below = commonest(raster, (rect.y1 * scale) as i64 + 1, x0, x1, ink);
    let [r, g, b] = match (above, below) {
        (Some((a, _)), Some((b, _))) if is_fringe(b, ink, a) => a,
        (Some(a), Some(b)) => std::cmp::max_by_key(a, b, |(px, n)| (*n, *px)).0,
        (Some((px, _)), None) | (None, Some((px, _))) => px,
        (None, None) => return None,
    };
    Some(Rgb::from_u8(r, g, b))
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
