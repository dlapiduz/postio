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
        open_folds: Vec::new(),
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
    let doc = postio_render::render(&request);
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
    let doc = postio_render::render(&request);
    assert_eq!(doc.outcome, postio_render::Outcome::Rendered);
    assert_eq!(
        doc.counts.resources_unresolved, 2,
        "the dangling cid: and the relative src"
    );
}
