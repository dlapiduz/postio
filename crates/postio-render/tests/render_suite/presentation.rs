//! How each message is presented in each theme (spec FR-013, research R10).

use crate::support::{DARK, LIGHT, render, request, request_for};
use postio_render::Presentation;

fn presentation(name: &str, theme: postio_render::Theme) -> Presentation {
    let doc = render(&request(name, theme).expect("a body"));
    assert_eq!(doc.messages.len(), 1, "{name}: one message");
    doc.messages[0].presentation
}

#[test]
fn each_fixture_is_presented_as_the_rule_says() {
    for (name, expected) in [
        ("html-newsletter", Presentation::Paper),
        ("html-designed-three-column", Presentation::Paper),
        ("html-white-page-reply", Presentation::Adapted),
        ("html-dark-text-no-background", Presentation::Adapted),
        ("html-dark-aware", Presentation::SenderDark),
        ("plain-text-simple", Presentation::Adapted),
    ] {
        assert_eq!(presentation(name, DARK), expected, "{name} in dark");
        assert_eq!(
            presentation(name, LIGHT),
            Presentation::Styled,
            "{name} in light"
        );
    }
}

/// FR-013's line: a page at or above relative luminance 0.9 is a client's
/// default white, below it a design.
#[test]
fn the_near_white_boundary() {
    let page = |grey: &str| {
        let html = postio_ui::reader::document::document_for(
            &format!(
                "<div class=\"postio-canvas\" style=\"background-color:{grey}\"><p>Hello.</p></div>"
            ),
            "",
            postio_body::RemoteImages::Blocked,
            postio_ui::reader::document::Sheet::Senders,
        );
        render(&request_for(html, DARK)).messages[0].presentation
    };
    // #f5f5f5 is 0.913; #f2f2f2 is 0.888.
    assert_eq!(page("#f5f5f5"), Presentation::Adapted);
    assert_eq!(page("#f2f2f2"), Presentation::Paper);
}

fn luminance(raster: &postio_render::Raster, x: f64, y: f64) -> f64 {
    let i = ((y.round() as usize) * raster.width as usize + x.round() as usize) * 4;
    postio_render::theme::relative_luminance(postio_render::Rgb::from_u8(
        raster.rgba[i],
        raster.rgba[i + 1],
        raster.rgba[i + 2],
    ))
}

/// Designed mail in dark mode is a sheet of paper on the reader's ground.
#[test]
fn paper_is_a_light_card_on_the_dark_ground() {
    let doc = render(&request("html-newsletter", DARK).expect("a body"));
    assert_eq!(doc.messages[0].presentation, Presentation::Paper);
    let raster = postio_render::rasterize(&doc);
    let card = doc.messages[0].rect;
    assert!(
        luminance(&raster, card.x0 + 40.0, card.y0 + 4.0) > 0.7,
        "inside the card is paper"
    );
    assert!(card.x0 > 4.0, "the card is inset from the reader's edge");
    assert!(
        luminance(&raster, card.x0 - 3.0, card.center().y) < 0.1,
        "outside is the dark ground"
    );
}

/// FR-013a: darkened paper keeps its hues at the dark end, every run still
/// meets the floor, and undarkening gives back the paper exactly.
#[test]
fn darken_remaps_the_grounds_and_round_trips() {
    use postio_render::theme::{contrast, to_oklch};
    let paper_request = request("html-newsletter", DARK).expect("a body");
    let paper = render(&paper_request);
    let mut darkened_request = paper_request.clone();
    darkened_request.darkened = vec![String::new()];
    let darkened = render(&darkened_request);
    assert_eq!(darkened.messages[0].presentation, Presentation::Darkened);
    for cluster in &darkened.text.clusters {
        let l = to_oklch(cluster.painted_ground).l;
        assert!(
            (0.12 - 1e-3..=0.30 + 1e-3).contains(&l),
            "a ground at OKLab L {l}"
        );
        assert!(contrast(cluster.color, cluster.painted_ground) >= 4.5 - 1e-3);
        if let Some(before) = paper
            .text
            .clusters
            .iter()
            .find(|c| c.range == cluster.range)
        {
            let (a, b) = (
                to_oklch(before.painted_ground),
                to_oklch(cluster.painted_ground),
            );
            if a.c > 0.03 {
                let d = (a.h - b.h).rem_euclid(360.0);
                assert!(d.min(360.0 - d) <= 2.0, "hue moved {a:?} -> {b:?}");
            }
        }
    }
    let again = render(&paper_request);
    assert!(
        again.display_list == paper.display_list,
        "undarkening did not restore the paper"
    );
}

/// FR-015: an image keeps its intended canvas behind it. The transparent
/// logo is found in the raster only as it looks over the sender's white:
/// over the dark ground its transparent pixels would not match.
#[test]
fn a_transparent_logo_keeps_its_canvas() {
    let parsed =
        postio_model::mime::parse(postio_model::test_corpus::load("html-transparent-logo").bytes());
    let logo = parsed
        .parts
        .iter()
        .find(|p| p.attachment.content_id.is_some())
        .map(|p| {
            image::load_from_memory(&p.content)
                .expect("the logo decodes")
                .to_rgba8()
        })
        .expect("the logo part");
    let over_white = |x: u32, y: u32| {
        let p = logo.get_pixel(x, y).0;
        let a = f64::from(p[3]) / 255.0;
        [0, 1, 2].map(|c| (f64::from(p[c]) * a + 255.0 * (1.0 - a)).round() as i32)
    };
    let found = |request: postio_render::RenderRequest| {
        let raster = postio_render::rasterize(&render(&request));
        let (w, h) = (raster.width, raster.height.min(300));
        let samples: Vec<(u32, u32)> = (0..16)
            .map(|i| ((i * 13) % logo.width(), (i * 7) % logo.height()))
            .collect();
        (0..h.saturating_sub(logo.height())).any(|oy| {
            (0..w.saturating_sub(logo.width())).any(|ox| {
                samples.iter().all(|&(x, y)| {
                    let i = (((oy + y) * w + ox + x) * 4) as usize;
                    let want = over_white(x, y);
                    (0..3).all(|c| (i32::from(raster.rgba[i + c]) - want[c]).abs() <= 3)
                })
            })
        })
    };
    let mut darkened = request("html-transparent-logo", DARK).expect("a body");
    darkened.darkened = vec![String::new()];
    assert!(found(darkened), "Darkened: the logo is not over its canvas");
    let mut adapted = request("html-transparent-logo", DARK).expect("a body");
    adapted.document = adapted.document.replace("#f5f5f5", "transparent");
    assert_eq!(
        render(&adapted).messages[0].presentation,
        Presentation::Adapted
    );
    assert!(found(adapted), "Adapted: the logo is not over its canvas");
}
