//! The render thread (research R6): a panic is contained and falls back,
//! stale results are never delivered, and an abandoned render does not
//! hold up the next one.

use std::sync::Arc;
use std::time::Duration;

use postio_render::{FallbackReason, Outcome, RenderRequest, Renderer, Resources, Theme, Viewport};

/// Long enough that only a broken renderer waits it out.
const PATIENCE: Duration = Duration::from_secs(20);

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

/// A request for a document that looks up one part, so a hook on the
/// resource table runs inside the render.
fn request(generation: u64, resources: Resources) -> RenderRequest {
    RenderRequest {
        generation,
        document: "<!DOCTYPE html><html><body><p>hello</p>\
                   <img src=\"postio-cid:part%40example.com\"></body></html>"
            .to_owned(),
        plain_text: String::new(),
        fallback: None,
        over_cap: None,
        resources: Arc::new(resources),
        viewport: Viewport {
            width: 400.0,
            hidpi_scale: 1.0,
            zoom: 1.0,
        },
        theme: Theme::default(),
        darkened: Vec::new(),
        toggled_folds: Vec::new(),
        reader_view: Vec::new(),
    }
}

#[test]
fn a_panic_falls_back_and_the_next_request_renders() {
    let renderer = Renderer::new(fonts());
    let poisoned = Resources::new();
    poisoned.panic_on_lookup();
    let fell_back = renderer
        .request(request(1, poisoned))
        .recv_timeout(PATIENCE)
        .expect("a panicking render still answers");
    assert_eq!(
        fell_back.outcome,
        Outcome::FellBack(FallbackReason::Panicked)
    );
    assert_eq!(fell_back.generation, 1);

    let next = renderer
        .request(request(2, Resources::new()))
        .recv_timeout(PATIENCE)
        .expect("the next request is served");
    assert_eq!(next.outcome, Outcome::Rendered);
}

#[test]
fn a_stale_result_is_never_delivered() {
    let renderer = Renderer::new(fonts());
    let held = Resources::new();
    let release = held.hold_lookup();
    let first = renderer.request(request(1, held));
    let second = renderer.request(request(2, Resources::new()));
    release.release();
    let latest = second.recv_timeout(PATIENCE).expect("the latest is served");
    assert_eq!(latest.generation, 2);
    assert!(
        first.recv_timeout(PATIENCE).is_err(),
        "generation 1 was delivered after generation 2 was asked for"
    );
}

#[test]
fn after_abandon_the_next_request_goes_to_a_fresh_thread() {
    let renderer = Renderer::new(fonts());
    let held = Resources::new();
    let release = held.hold_lookup();
    let abandoned = renderer.request(request(1, held));
    release.wait_until_held(PATIENCE);
    renderer.abandon(1);
    // Generation 1 is still inside its render; one thread could not answer.
    let next = renderer
        .request(request(2, Resources::new()))
        .recv_timeout(PATIENCE)
        .expect("a fresh thread serves the next request");
    assert_eq!(next.generation, 2);
    release.release();
    assert!(
        abandoned.recv_timeout(PATIENCE).is_err(),
        "an abandoned render was delivered"
    );
}

/// A newer request does not queue behind a render already superseded
/// (specs/007-postio-focus T218). The reader's deadline runs from the
/// moment it asks, so a request that waited for a stale render to finish
/// spent its budget on a layout nobody would see: Focus asks twice as a
/// message opens -- once at the column it had, again at the column the
/// body's treatment gives it -- and stepping with `j` asks once per key,
/// so an ordinary newsletter fell back to plain text on a busy machine.
#[test]
fn a_newer_request_does_not_wait_for_a_superseded_render() {
    let renderer = Renderer::new(fonts());
    let held = Resources::new();
    let release = held.hold_lookup();
    let stale = renderer.request(request(1, held));
    release.wait_until_held(PATIENCE);
    // Generation 1 is inside its render, and stays there: the next request
    // is answered while it is.
    let next = renderer
        .request(request(2, Resources::new()))
        .recv_timeout(PATIENCE / 4)
        .expect("the newer request waited for the superseded render");
    assert_eq!(next.generation, 2);
    release.release();
    assert!(
        stale.recv_timeout(PATIENCE).is_err(),
        "a superseded render was delivered"
    );
}

/// A message the sanitizer found over an input cap falls back without the
/// engine ever parsing it: its lookup would panic, and does not run.
#[test]
fn a_message_over_a_cap_falls_back_without_reaching_the_engine() {
    let renderer = Renderer::new(fonts());
    let poisoned = Resources::new();
    poisoned.panic_on_lookup();
    let mut over = request(1, poisoned);
    over.over_cap = Some(postio_body::Cap::Depth);
    over.plain_text = "the plain alternative".to_owned();
    let fell_back = renderer
        .request(over)
        .recv_timeout(PATIENCE)
        .expect("an over-cap message still answers");
    assert_eq!(
        fell_back.outcome,
        Outcome::FellBack(FallbackReason::OverCap(postio_body::Cap::Depth))
    );
}
