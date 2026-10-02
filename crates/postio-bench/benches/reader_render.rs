//! The reading renderer, warm (spec 006 SC-005, T149).
//!
//! For each designed corpus fixture -- the mail whose layout its sender
//! built, the case the renderer exists for -- this times a second request
//! on a live `Renderer`, meaning the fonts and the thread are already up,
//! and the rasterisation of the first tile the view would draw.
//!
//! **Reported, never gated** (Constitution V: timings report, counts
//! gate). `bench.yml` compiles this nightly and times nothing; what holds
//! SC-005 on every pull request is the counted cost -- `RenderCounts` in
//! `postio-render`'s snapshot tests and `reader_spawns_no_web_process`'s
//! one render and two style passes per navigation.

#![allow(missing_docs)]
// `criterion_group!` expands to a `pub fn`, and the workspace lint floor
// reaches bench targets. A bench is not public API.

use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};
use postio_body::RemoteImages;
use postio_model::test_corpus::{self, Category};
use postio_render::fonts::{Bundled, FontSet};
use postio_render::tile::{TileSpec, rasterize_tile, tile_width};
use postio_render::{RenderRequest, Renderer, Resources, Theme, Viewport};
use postio_ui::reader::document::{self, Rendering};

/// The height of a tile the view draws, in device pixels.
const TILE_HEIGHT: u32 = 512;

/// `fixture` composed the way the reader composes it, with its parts.
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
        // The first request warms the thread and the fonts: not what is
        // being measured.
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
