//! What Postio's patches to Blitz (`patches/blitz/`) change, drawn and
//! counted: each case is a document upstream paints wrongly, reduced to the
//! smallest one that shows it, so a new upstream version that drops or
//! breaks a patch fails here rather than in somebody's inbox.

mod support;

use anyrender::ImageRenderer;
use blitz_dom::{BaseDocument, DocumentConfig};
use blitz_traits::shell::{ColorScheme, Viewport};

const WIDTH: u32 = 200;

fn paint(html: &str) -> (Vec<[u8; 4]>, u32, u32) {
    let (pixels, height, _) = paint_at(html, WIDTH);
    (pixels, WIDTH, height)
}

/// `html` laid out and drawn in a viewport `width` CSS pixels wide: its
/// pixels, its height, and the document, to read its layout.
fn paint_at(html: &str, width: u32) -> (Vec<[u8; 4]>, u32, BaseDocument) {
    let config = DocumentConfig {
        viewport: Some(Viewport::new(width, 200, 1.0, ColorScheme::Light)),
        base_url: Some("postio-message://message/".to_owned()),
        ..Default::default()
    };
    let mut doc: BaseDocument = blitz_html::HtmlDocument::from_html(html, config).into_inner();
    doc.resolve(0.0);
    let height = (doc.root_element().final_layout().size.height.ceil() as u32).max(1);
    let mut renderer = anyrender_vello_cpu::VelloCpuImageRenderer::new(width, height);
    let mut buffer = vec![0u8; (width * height * 4) as usize];
    renderer.render(
        |scene| blitz_paint::paint_scene(scene, &mut doc, 1.0, width, height, 0, 0),
        &mut buffer,
    );
    let pixels = buffer.as_chunks::<4>().0.to_vec();
    (pixels, height, doc)
}

fn count(pixels: &[[u8; 4]], want: impl Fn(&[u8; 4]) -> bool) -> usize {
    pixels.iter().filter(|p| want(p)).count()
}

fn page(table_css: &str, cell_css: &str) -> String {
    format!(
        "<!DOCTYPE html><html><head><style>\
         html,body{{margin:0;background:#fff}}\
         table{{border-collapse:collapse;{table_css}}}\
         td{{width:40px;height:40px;padding:0;{cell_css}}}\
         </style></head><body><table>\
         <tr><td></td><td></td></tr><tr><td></td><td></td></tr>\
         </table></body></html>"
    )
}

/// Upstream #504: a borderless cell's computed width is the initial
/// `medium`, and the collapsed model painted it as a 3px grid in the text
/// colour -- every layout table in every newsletter wore one.
#[test]
fn a_collapsed_table_with_no_borders_paints_no_grid() {
    let (pixels, ..) = paint(&page("", ""));
    let marked = count(&pixels, |p| p[0] < 250 || p[1] < 250 || p[2] < 250);
    assert_eq!(
        marked, 0,
        "a borderless collapsed table drew {marked} pixels"
    );
}

/// The same defect in layout: the phantom width was also a gutter, so the
/// cells stood 3px apart and the table grew.
#[test]
fn a_collapsed_table_with_no_borders_is_exactly_its_cells() {
    let html = page("background:#000", "background:#fff");
    let (pixels, ..) = paint(&html);
    let black = count(&pixels, |p| p[0] < 5 && p[1] < 5 && p[2] < 5);
    assert_eq!(black, 0, "{black} pixels of table showed between the cells");
}

/// The painter drew every edge in `border-top-color`. With the vertical
/// edges blue and the horizontal ones red, the gutter between the columns
/// has to be blue.
#[test]
fn a_collapsed_grid_paints_each_edge_in_its_own_colour() {
    let (pixels, ..) = paint(&page("", "border:4px solid;border-color:#f00 #00f"));
    let red = count(&pixels, |p| p[0] > 200 && p[1] < 50 && p[2] < 50);
    let blue = count(&pixels, |p| p[0] < 50 && p[1] < 50 && p[2] > 200);
    assert!(red > 0, "the horizontal edges are red");
    assert!(blue > 0, "the vertical edges are blue, but none were drawn");
}

/// What the patch must not lose: a table that asks for a grid gets one.
#[test]
fn a_collapsed_table_with_borders_still_paints_its_grid() {
    let (pixels, ..) = paint(&page("", "border:1px solid #000"));
    let black = count(&pixels, |p| p[0] < 5 && p[1] < 5 && p[2] < 5);
    // Three horizontal and three vertical one-pixel lines about 81px long.
    assert!(black > 400, "only {black} pixels of grid were drawn");
}

/// And a cell that says `none` on one side only has no line there: here the
/// cells keep their top and bottom but not their sides.
#[test]
fn a_collapsed_edge_styled_none_is_not_drawn() {
    let (pixels, ..) = paint(&page(
        "",
        "border-top:2px solid #000;border-bottom:2px solid #000;border-left:none;border-right:none",
    ));
    let (width, height) = (WIDTH as usize, pixels.len() / WIDTH as usize);
    // Down the middle of the first column no vertical line can cross; along
    // a row between the horizontal lines, nothing dark at all.
    let row = height / 4;
    let dark_in_row = (0..width)
        .filter(|&x| {
            let p = pixels[row * width + x];
            p[0] < 128 && p[1] < 128 && p[2] < 128
        })
        .count();
    assert_eq!(dark_in_row, 0, "a vertical line crossed row {row}");
}

/// Responsive newsletters stack their columns on a narrow screen with
/// `@media (max-width:600px) { .stack { display:block } }` on the cells,
/// and the app-colours column is 480px wide. A `display:block` child of a
/// row is not a cell, and the release dropped it -- text, images and all.
/// CSS 2.1 §17.2.1 wraps such children in one anonymous cell, so they are
/// drawn and stack inside it, each as wide as the table. Here every cell is
/// 30px of its own colour holding a word, blockified by the media
/// query in the first row and by its own style in the second.
#[test]
fn a_cell_made_display_block_is_drawn_and_stacks() {
    const NARROW: u32 = 480;
    let html = "<!DOCTYPE html><html><head><style>\
         html,body{margin:0;background:#fff;color:#000}\
         td{padding:0;height:30px;line-height:30px}\
         @media (max-width:600px){.stack{display:block !important;width:100% !important}}\
         </style></head><body>\
         <table width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\">\
         <tr><td id=\"a\" class=\"stack\" style=\"background:#f00\">Alpha</td>\
         <td id=\"b\" class=\"stack\" style=\"background:#00f\">Bravo</td></tr>\
         <tr><td id=\"c\" style=\"display:block;background:#0f0\">Charlie</td>\
         <td id=\"d\" style=\"display:block;background:#ff0\">Delta</td></tr>\
         </table></body></html>";
    let (pixels, height, doc) = paint_at(html, NARROW);
    let width = NARROW as usize;

    for (id, colour) in [
        ("a", [255, 0, 0]),
        ("b", [0, 0, 255]),
        ("c", [0, 255, 0]),
        ("d", [255, 255, 0]),
    ] {
        let node = doc.get_element_by_id(id).expect("the cell is in the DOM");
        let size = doc.get_node(node).unwrap().final_layout().size;
        assert!(
            size.width > 0.0 && size.height > 0.0,
            "cell {id} was not laid out: {size:?}"
        );

        // The rows of the raster this cell's colour fills most of.
        let is = |p: &[u8; 4]| (0..3).all(|i| (i32::from(p[i]) - colour[i]).abs() < 8);
        let rows = (0..height as usize)
            .filter(|&y| {
                pixels[y * width..(y + 1) * width]
                    .iter()
                    .filter(|p| is(p))
                    .count()
                    > width / 2
            })
            .count();
        assert!(
            rows >= 20,
            "cell {id} filled {rows} rows of the {NARROW}px column: it was not drawn full width"
        );
    }

    // Blitz alone has no fonts here, so the words are read from the text
    // the renderer drew: each one starts a drawn cluster with a size.
    let mut request = support::request_for(html.to_owned(), support::LIGHT);
    request.viewport.width = f64::from(NARROW);
    let drawn = support::render(&request);
    for word in ["Alpha", "Bravo", "Charlie", "Delta"] {
        let at = drawn
            .text
            .text
            .find(word)
            .unwrap_or_else(|| panic!("`{word}` is not in the drawn text"));
        let at = drawn.text.text[..at].chars().count();
        let cluster = drawn
            .text
            .clusters
            .iter()
            .find(|c| c.range.start == at)
            .unwrap_or_else(|| panic!("`{word}` was not drawn"));
        assert!(
            cluster.rect.width() > 0.0 && cluster.rect.height() > 0.0,
            "`{word}` was drawn with no size"
        );
    }
}
