//! Images reach the page: decoded from the message's own parts, never
//! fetched (spec FR-003; research R4, R5).

use std::collections::HashSet;
use std::sync::Arc;

use postio_body::RemoteImages;
use postio_model::test_corpus;
use postio_render::{RenderRequest, Resources, Theme, Viewport};
use postio_ui::reader::document::{self, Rendering};

/// The composed document of one fixture, and its parts as a resource table.
fn request(fixture: &str) -> (RenderRequest, Vec<(String, Vec<u8>)>) {
    let parsed = postio_model::mime::parse(test_corpus::load(fixture).bytes());
    let parts: Vec<(String, Vec<u8>)> = parsed
        .parts
        .iter()
        .filter_map(|part| Some((part.attachment.content_id.clone()?, part.content.clone())))
        .collect();
    let resources = Resources::new();
    for (cid, bytes) in &parts {
        resources.insert_part(None, cid, bytes.clone());
    }
    for face in document::FACES {
        resources.insert_font(face.name, face.bytes);
    }
    let rendered = document::body_html_in(
        &parsed.body,
        RemoteImages::Blocked,
        Rendering::Original,
        None,
    );
    let sheet = document::sheet_for(Rendering::Original, false);
    let html = document::document_for(
        &rendered.html,
        &rendered.styles,
        RemoteImages::Blocked,
        sheet,
    );
    let request = RenderRequest {
        generation: 1,
        document: html,
        resources: Arc::new(resources),
        viewport: Viewport {
            width: 800.0,
            hidpi_scale: 1.0,
            zoom: 1.0,
        },
        theme: Theme::default(),
        darkened: Vec::new(),
        toggled_folds: Vec::new(),
        reader_view: Vec::new(),
    };
    (request, parts)
}

/// The spike never enabled a decoder format, so every image drew as its
/// placeholder and nobody saw it. Each inline part must paint its own
/// pixels: most of a part's distinct colours appear in the raster.
#[test]
fn an_inline_cid_image_paints_its_own_pixels() {
    let (request, parts) = request("inline-image-cid");
    let doc = postio_render::render(&request, fonts());
    let raster = postio_render::rasterize(&doc);
    let painted: HashSet<[u8; 3]> = raster
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| [p[0], p[1], p[2]])
        .collect();
    for (cid, bytes) in parts {
        let image = image::load_from_memory(&bytes).expect("the fixture's part decodes");
        let own: HashSet<[u8; 3]> = image.to_rgb8().pixels().map(|p| p.0).collect();
        let found = own.intersection(&painted).count();
        assert!(
            found * 2 > own.len(),
            "{cid}: {found} of its {} colours were painted",
            own.len()
        );
    }
    assert_eq!(
        doc.counts.resources_unresolved, 1,
        "the dangling cid: is counted"
    );
}

/// A relative URL resolves against the renderer's base, to nothing: without
/// a base Blitz resolved against a `data:` URL and panicked (research R1;
/// the evaluation's `POSTIO_EVAL_NO_BASE` shows it).
#[test]
fn a_relative_image_url_resolves_to_nothing_without_a_panic() {
    let (mut request, _) = request("inline-image-cid");
    request.document = request.document.replacen(
        "</body>",
        "<img src=\"x\" width=\"10\" height=\"10\"></body>",
        1,
    );
    let doc = postio_render::render(&request, fonts());
    assert_eq!(doc.outcome, postio_render::Outcome::Rendered);
    assert_eq!(
        doc.counts.resources_unresolved, 2,
        "the dangling cid: and the relative src"
    );
}

/// An SVG image paints its own shapes, and never a local file its
/// `<image href>` names: usvg's default resolver reads any path it is
/// given, and Blitz used the default (research R5).
#[test]
fn an_svg_image_paints_its_shapes_and_no_local_file() {
    let probe = tempfile::Builder::new()
        .suffix(".png")
        .tempfile()
        .expect("a temporary file");
    let magenta = image::RgbImage::from_pixel(40, 40, image::Rgb([0xff, 0x00, 0xff]));
    magenta
        .save_with_format(probe.path(), image::ImageFormat::Png)
        .expect("the probe PNG is written");

    let (request, parts) = request("html-svg-local-file");
    let (cid, svg) = parts.into_iter().next().expect("the fixture's SVG part");
    let svg = String::from_utf8(svg)
        .expect("the SVG is text")
        // Both forms: the fixture's `file://` URL, and a bare absolute
        // path, which is what usvg's default resolver hands to fs::read.
        .replacen(
            "file:///tmp/postio-svg-local-file-probe.png",
            &probe.path().to_string_lossy(),
            1,
        )
        .replace(
            "/tmp/postio-svg-local-file-probe.png",
            &probe.path().to_string_lossy(),
        );
    request.resources.insert_part(None, &cid, svg.into_bytes());

    let raster = postio_render::rasterize(&postio_render::render(&request, fonts()));
    let pixels = raster.rgba.as_chunks::<4>().0;
    let near = |p: &[u8; 4], [r, g, b]: [u8; 3]| {
        p[0].abs_diff(r) < 8 && p[1].abs_diff(g) < 8 && p[2].abs_diff(b) < 8
    };
    let green = pixels
        .iter()
        .filter(|p| near(p, [0x2e, 0x7d, 0x32]))
        .count();
    let leaked = pixels
        .iter()
        .filter(|p| near(p, [0xff, 0x00, 0xff]))
        .count();
    assert_eq!(leaked, 0, "{leaked} pixels of a local file were painted");
    assert!(green > 1_000, "the SVG's own rect painted {green} pixels");
}

/// The process's font set: bundled faces plus discovery, built once.
fn fonts() -> &'static postio_render::fonts::FontSet {
    static FONTS: std::sync::OnceLock<postio_render::fonts::FontSet> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        postio_render::fonts::FontSet::new(postio_render::fonts::Bundled {
            faces: postio_ui::reader::document::FACES
                .iter()
                .map(|face| face.bytes)
                .collect(),
            sans: "Barlow",
            mono: "IBM Plex Mono",
        })
    })
}
