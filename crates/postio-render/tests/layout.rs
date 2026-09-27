//! The sender's layout, as they built it (spec 006 US2).

mod support;

use postio_render::Rgb;
use support::{LIGHT, render, request, request_for};

fn x_of(doc: &postio_render::RenderedDocument, text: &str) -> f64 {
    let range = doc.text.find(text)[0].clone();
    doc.text.rects(range)[0].x0
}

/// FR-008: `@media` queries see the pane's width. At 500px the two columns
/// stack; at 800px they sit side by side.
#[test]
fn media_queries_see_the_pane_width() {
    let at = |width: f64| {
        let mut request = request("html-responsive-media", LIGHT).expect("a body");
        request.viewport.width = width;
        render(&request)
    };
    let narrow = at(500.0);
    assert!(
        (x_of(&narrow, "Market hall") - x_of(&narrow, "Bus times")).abs() < 1.0,
        "the columns did not stack at 500px"
    );
    let wide = at(800.0);
    assert!(
        x_of(&wide, "Bus times") - x_of(&wide, "Market hall") > 250.0,
        "the columns did not sit side by side at 800px"
    );
}

/// FR-010: nothing in `<head>` is content.
#[test]
fn the_head_is_not_content() {
    let doc = render(&request_for(
        "<!DOCTYPE html><html><head><title>Secret title</title>\
         <style>p { color: #123456 }</style></head><body><p>Hello.</p></body></html>"
            .to_owned(),
        LIGHT,
    ));
    assert!(
        !doc.text.text.contains("Secret title"),
        "{:?}",
        doc.text.text
    );
    assert!(!doc.text.text.contains("color"), "{:?}", doc.text.text);
    assert!(doc.text.text.contains("Hello."));
}

/// #1545: a sender's classes reach the page. The `.cta` rule's background
/// is painted behind the link it styles.
#[test]
fn a_senders_class_rule_is_painted() {
    let doc = render(&request("html-class-styled", LIGHT).expect("a body"));
    let raster = postio_render::rasterize(&doc);
    let rect = doc.text.rects(doc.text.find("Enter a crew")[0].clone())[0];
    let (x, y) = (
        (rect.x0 - 8.0).round() as usize,
        rect.center().y.round() as usize,
    );
    let i = (y * raster.width as usize + x) * 4;
    let painted = Rgb::from_u8(raster.rgba[i], raster.rgba[i + 1], raster.rgba[i + 2]);
    assert_eq!(
        painted.to_u8(),
        [0xd9, 0x82, 0x2b],
        "the .cta background is not painted"
    );
}
