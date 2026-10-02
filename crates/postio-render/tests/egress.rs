//! The observed half of "cannot reach the network" (spec FR-001, contract
//! renderer-graph-checks § 3). The graph checks prove no network crate is
//! linked; this watches a loopback listener while every hostile fixture is
//! rendered with its remote URLs pointed at it, consented or not -- the
//! renderer must not fetch even then: remote bytes come only from the app.

use std::sync::Arc;
use std::time::Duration;

use postio_body::RemoteImages;
use postio_model::test_corpus::{self, Category};
use postio_render::{RenderRequest, Renderer, Resources, Theme, Viewport};
use postio_test_support::listener::Listener;
use postio_ui::reader::document::{self, Rendering};

fn fonts() -> &'static postio_render::fonts::FontSet {
    static FONTS: std::sync::OnceLock<postio_render::fonts::FontSet> = std::sync::OnceLock::new();
    FONTS.get_or_init(|| {
        postio_render::fonts::FontSet::new(postio_render::fonts::Bundled {
            faces: document::FACES.iter().map(|face| face.bytes).collect(),
            sans: "Barlow",
            mono: "IBM Plex Mono",
        })
    })
}

#[test]
fn rendering_every_hostile_fixture_opens_no_connection() {
    let listener = Listener::start();
    // Control first: a zero means nothing from a listener nobody could reach.
    listener.control();
    let baseline = listener.count();

    let renderer = Renderer::new(fonts());
    let mut generation = 0;
    let hostile = test_corpus::by_category(Category::Hostile);
    assert!(!hostile.is_empty());
    for fixture in hostile {
        for remote in [RemoteImages::Blocked, RemoteImages::Allowed] {
            let mut parsed = postio_model::mime::parse(fixture.bytes());
            parsed.body.html = parsed.body.html.map(|html| listener.rewrite(&html));
            parsed.body.text = parsed.body.text.map(|text| listener.rewrite(&text));
            let body = document::body_html_in(&parsed.body, remote, Rendering::Original, None);
            let html = document::document_for(
                &body.html,
                &body.styles,
                remote,
                document::sheet_for(Rendering::Original, false),
            );
            generation += 1;
            let request = RenderRequest {
                generation,
                document: listener.rewrite(&html),
                plain_text: parsed.body.text.clone().unwrap_or_default(),
                fallback: None,
                over_cap: body.over_cap,
                resources: Arc::new(Resources::new()),
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
            renderer
                .request(request)
                .recv_timeout(Duration::from_secs(30))
                .unwrap_or_else(|_| panic!("{} did not render", fixture.name()));
        }
    }
    // Anything the engine started would have landed by now.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        listener.count() - baseline,
        0,
        "rendering connected to the listener: {:?}",
        listener.paths()
    );
}
