//! What Postio's patches to Blitz (`patches/blitz/`) change, drawn and
//! counted: each case is a document upstream paints wrongly, reduced to the
//! smallest one that shows it, so a new upstream version that drops or
//! breaks a patch fails here rather than in somebody's inbox.

use anyrender::ImageRenderer;
use blitz_dom::{BaseDocument, DocumentConfig};
use blitz_traits::shell::{ColorScheme, Viewport};

const WIDTH: u32 = 200;

fn paint(html: &str) -> (Vec<[u8; 4]>, u32, u32) {
    let config = DocumentConfig {
        viewport: Some(Viewport::new(WIDTH, 200, 1.0, ColorScheme::Light)),
        base_url: Some("postio-message://message/".to_owned()),
        ..Default::default()
    };
    let mut doc: BaseDocument = blitz_html::HtmlDocument::from_html(html, config).into_inner();
    doc.resolve(0.0);
    let height = (doc.root_element().final_layout().size.height.ceil() as u32).max(1);
    let mut renderer = anyrender_vello_cpu::VelloCpuImageRenderer::new(WIDTH, height);
    let mut buffer = vec![0u8; (WIDTH * height * 4) as usize];
    renderer.render(
        |scene| blitz_paint::paint_scene(scene, &mut doc, 1.0, WIDTH, height, 0, 0),
        &mut buffer,
    );
    let pixels = buffer.as_chunks::<4>().0.to_vec();
    (pixels, WIDTH, height)
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
