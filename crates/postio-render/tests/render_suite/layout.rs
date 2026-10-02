//! The sender's layout, as they built it (spec 006 US2).

use crate::support::{LIGHT, render, request, request_for};
use postio_render::Rgb;

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

/// FR-026: an image arriving moves nothing. The same message with its
/// remote images missing and then present has the same message box and
/// the same place for every line of text.
#[test]
fn remote_images_arriving_move_nothing() {
    use postio_body::RemoteImages;
    use postio_ui::reader::document::{self, Rendering};
    let parsed = postio_model::mime::parse(
        postio_model::test_corpus::load("html-tracking-pixel-remote-images").bytes(),
    );
    let body = document::body_html_in(
        &parsed.body,
        RemoteImages::Allowed,
        Rendering::Original,
        None,
    );
    let html = document::document_for(
        &body.html,
        &body.styles,
        RemoteImages::Allowed,
        document::sheet_for(Rendering::Original, false),
    );
    let missing = render(&request_for(html.clone(), LIGHT));
    let present_request = request_for(html.clone(), LIGHT);
    let urls: Vec<String> = html
        .split(['"', '\'', '(', ')'])
        .filter(|s| s.starts_with("https://") && !s.contains(' '))
        .map(str::to_owned)
        .collect();
    assert!(
        !urls.is_empty(),
        "the fixture's remote images survived consent"
    );
    let mut picture = Vec::new();
    image::RgbImage::from_pixel(1, 1, image::Rgb([200, 30, 30]))
        .write_to(
            &mut std::io::Cursor::new(&mut picture),
            image::ImageFormat::Png,
        )
        .expect("a PNG");
    for url in &urls {
        present_request
            .resources
            .insert_remote(url, picture.clone());
    }
    let present = render(&present_request);
    assert!(
        present.counts.resources_resolved > missing.counts.resources_resolved,
        "no image arrived"
    );
    assert_eq!(
        missing.messages[0].rect, present.messages[0].rect,
        "the message box moved"
    );
    let lines = |doc: &postio_render::RenderedDocument| -> Vec<(String, f64)> {
        doc.text
            .clusters
            .iter()
            .map(|c| (doc.text.slice(c.range.clone()).to_owned(), c.rect.y0))
            .collect()
    };
    assert_eq!(
        lines(&missing),
        lines(&present),
        "text moved when the images arrived"
    );
}
