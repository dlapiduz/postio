//! The reading renderer, warm: a second request on a live `Renderer` per designed corpus fixture,
//! and the first tile's rasterisation. Reported, never gated: counts gate, timings report.

use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use postio_body::RemoteImages;
use postio_model::test_corpus::{self, Category};
use postio_render::fonts::{Bundled, FontSet};
use postio_render::tile::{TileSpec, rasterize_tile, tile_width};
use postio_render::{RenderRequest, Renderer, Resources, Theme, Viewport};
use postio_ui::reader::document::{self, Rendering};

const TILE_HEIGHT: u32 = 512;

fn request(fixture: &test_corpus::Fixture) -> Option<RenderRequest> {
    let parsed = postio_model::mime::parse(fixture.bytes());
    if parsed.body.is_empty() {
        return None;
    }
    let resources = Resources::new();
    for face in document::FACES {
        resources.insert_font(face.name, face.bytes);
    }
    for part in &parsed.parts {
        if let Some(cid) = &part.attachment.content_id {
            resources.insert_part(None, cid, part.content.clone());
        }
    }
    let body = document::body_html_in(
        &parsed.body,
        RemoteImages::Blocked,
        Rendering::Original,
        None,
    );
    let html = document::document_for(
        &body.html,
        &body.styles,
        RemoteImages::Blocked,
        document::sheet_for(Rendering::Original, false),
    );
    Some(RenderRequest {
        generation: 1,
        document: html,
        plain_text: parsed.body.text.unwrap_or_default(),
        fallback: None,
        over_cap: body.over_cap,
        resources: Arc::new(resources),
        viewport: Viewport {
            width: 800.0,
            hidpi_scale: 1.0,
            zoom: 1.0,
        },
        theme: Theme {
            dark: true,
            high_contrast: false,
        },
        darkened: Vec::new(),
        toggled_folds: Vec::new(),
        reader_view: Vec::new(),
    })
}

fn reader_render(c: &mut Criterion) {
    let fonts = FontSet::new(Bundled {
        faces: document::FACES.iter().map(|face| face.bytes).collect(),
        sans: "Barlow",
        mono: "IBM Plex Mono",
    });
    let renderer = Renderer::new(&fonts);
    let mut generation = 0u64;
    let mut render = |request: &RenderRequest| {
        generation += 1;
        let mut request = request.clone();
        request.generation = generation;
        renderer
            .request(request)
            .recv()
            .expect("the render thread answers")
    };

    let mut group = c.benchmark_group("reader_render");
    for fixture in test_corpus::by_category(Category::Designed) {
        let Some(request) = request(fixture) else {
            continue;
        };
        let warm = render(&request);
        group.bench_function(format!("warm/{}", fixture.name()), |b| {
            b.iter(|| black_box(render(&request)));
        });
        let tile = TileSpec::nth(0, TILE_HEIGHT);
        let mut pixels = vec![0u8; (tile_width(&warm) * TILE_HEIGHT * 4) as usize];
        group.bench_function(format!("first_tile/{}", fixture.name()), |b| {
            b.iter(|| rasterize_tile(black_box(&warm), tile, &mut pixels));
        });
    }
    group.finish();
}

criterion_group!(benches, reader_render);
criterion_main!(benches);
