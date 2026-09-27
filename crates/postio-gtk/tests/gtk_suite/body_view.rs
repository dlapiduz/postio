//! The reading surface on a real display (spec 006 research R8): a
//! snapshot arrives, the adjustment spans it, and the pixels on screen are
//! the message's own.

use std::sync::Arc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_body::RemoteImages;
use postio_gtk::body_view::{BodyView, Content};
use postio_model::test_corpus;
use postio_render::Resources;
use postio_ui::reader::document::{self, Rendering};

/// The fixture as the reader composes it, with its parts and faces.
fn content(name: &str) -> Content {
    let parsed = postio_model::mime::parse(test_corpus::load(name).bytes());
    let resources = Resources::new();
    for part in &parsed.parts {
        if let Some(cid) = &part.attachment.content_id {
            resources.insert_part(None, cid, part.content.clone());
        }
    }
    for face in document::FACES {
        resources.insert_font(face.name, face.bytes);
    }
    let body = document::body_html_in(
        &parsed.body,
        RemoteImages::Blocked,
        Rendering::Original,
        None,
    );
    Content {
        document: document::document_for(
            &body.html,
            &body.styles,
            RemoteImages::Blocked,
            document::sheet_for(Rendering::Original, false),
        ),
        resources: Arc::new(resources),
        plain_text: parsed.body.text.unwrap_or_default(),
        over_cap: body.over_cap,
    }
}

/// One turn of the main loop, blocking until something happens. The
/// heartbeat bounds the block: the frame clock paints on the compositor's
/// frame callbacks, which only a blocking iteration waits for.
fn pump() {
    let heartbeat = glib::timeout_add_local(std::time::Duration::from_millis(10), || {
        glib::ControlFlow::Continue
    });
    glib::MainContext::default().iteration(true);
    heartbeat.remove();
}

/// Run the main loop until `done`, painting frames; false on timeout.
fn until(done: impl Fn() -> bool) -> bool {
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while !done() && std::time::Instant::now() < deadline {
        pump();
    }
    done()
}

/// What `widget` draws, in its own coordinates, with the stride of a row in
/// bytes; `None` before it has drawn anything.
fn pixels(widget: &impl IsA<gtk::Widget>) -> Option<(usize, Vec<u8>)> {
    let widget = widget.as_ref();
    let (width, height) = (widget.width(), widget.height());
    let paintable = gtk::WidgetPaintable::new(Some(widget));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
    let node = snapshot.to_node()?;
    let renderer = widget.native().and_then(|native| native.renderer())?;
    let bounds = gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
    let texture = renderer.render_texture(&node, Some(&bounds));
    let mut downloader = gdk::TextureDownloader::new(&texture);
    downloader.set_format(gdk::MemoryFormat::R8g8b8a8);
    let (bytes, stride) = downloader.download_bytes();
    Some((stride, bytes.to_vec()))
}

pub fn a_snapshot_fills_the_view_and_scrolls_with_it() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let view = BodyView::new(crate::reader_deadline());
    let scroller = gtk::ScrolledWindow::builder().child(&view).build();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&scroller)
        .build();
    window.present();
    view.set_content(content("html-designed-three-column"));
    assert!(until(|| view.document().is_some()), "no snapshot arrived");
    let doc = view.document().expect("a snapshot");
    let adjustment = scroller.vadjustment();
    assert_eq!(
        adjustment.upper(),
        doc.size.height,
        "the adjustment does not span the document"
    );

    // Just inside the Trail runner card, left of its heading: the card's
    // own ground, #dcebe3. Scrolled 100px, it is 100px higher.
    let heading = doc.text.find("Trail runner")[0].clone();
    let rect = doc.text.rects(heading)[0];
    adjustment.set_value(100.0);
    // In the view's own coordinates: 100px higher once scrolled.
    let (x, y) = (rect.x0 - 6.0, rect.center().y - 100.0);
    let at = |pixels: &[u8], stride: usize| {
        let offset = y.round() as usize * stride + x.round() as usize * 4;
        [pixels[offset], pixels[offset + 1], pixels[offset + 2]]
    };
    let seen = || pixels(&view).map(|(stride, pixels)| at(&pixels, stride));
    let drawn = until(|| seen() == Some([0xdc, 0xeb, 0xe3]));
    assert!(
        drawn,
        "at ({x}, {y}) the view shows {:?}, not the card",
        seen()
    );
    window.destroy();
}

/// Near enough: the low-resolution copy is a quarter scale, upscaled.
fn near(a: Option<[u8; 3]>, b: [u8; 3]) -> bool {
    a.is_some_and(|a| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 6))
}

/// No frame is blank (FR-029): a render in flight leaves the last frame up,
/// and a tile that is not ready draws from the low-resolution copy rather
/// than the bare ground.
pub fn no_frame_shows_only_the_ground() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let view = BodyView::new(crate::reader_deadline());
    let scroller = gtk::ScrolledWindow::builder().child(&view).build();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&scroller)
        .build();
    window.present();
    view.set_content(content("html-designed-three-column"));
    assert!(until(|| view.document().is_some()), "no snapshot arrived");
    let doc = view.document().expect("a snapshot");
    let rect = doc.text.rects(doc.text.find("Trail runner")[0].clone())[0];
    // Read through the scroller, which always paints: where the view drew
    // nothing, its ground shows, so a blank view is seen as one.
    let point = view
        .compute_point(
            &scroller,
            &gtk::graphene::Point::new((rect.x0 - 6.0) as f32, rect.center().y as f32),
        )
        .expect("the view is inside its scroller");
    let at = |pixels: &[u8], stride: usize| {
        let offset = point.y().round() as usize * stride + point.x().round() as usize * 4;
        [pixels[offset], pixels[offset + 1], pixels[offset + 2]]
    };
    let seen = || pixels(&scroller).map(|(stride, pixels)| at(&pixels, stride));
    let card = [0xdc, 0xeb, 0xe3];
    assert!(until(|| seen() == Some(card)), "the card never drew");

    // A second render, held inside its resource lookup: every frame
    // painted meanwhile is still the first snapshot. A read of `None` is a
    // frame not yet repainted, not a blank one: the paintable hands back
    // the last node the widget drew, and there is none while a redraw is
    // pending.
    let mut held = content("html-designed-three-column");
    let resources = Resources::new();
    let gate = resources.hold_lookup();
    held.resources = Arc::new(resources);
    let before = doc.generation;
    view.set_content(held);
    let mut painted = 0;
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(5));
    while painted < 5 && std::time::Instant::now() < deadline {
        pump();
        if let Some(frame) = seen() {
            assert_eq!(frame, card, "a render in flight changed the frame");
            painted += 1;
        }
    }
    assert_eq!(painted, 5, "too few frames were painted to judge");
    gate.release();
    assert!(
        until(|| view.document().is_some_and(|d| d.generation > before)),
        "the second snapshot never arrived"
    );
    assert!(
        until(|| seen() == Some(card)),
        "the second snapshot never drew"
    );

    // Every tile gone: the first frame painted after is the one that asks
    // for them again, so it cannot hold them -- it shows the low-resolution
    // copy, or it shows the ground.
    view.evict_tiles();
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(5));
    let frame = loop {
        pump();
        if let Some(frame) = seen() {
            break frame;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "nothing was painted after eviction"
        );
    };
    assert!(
        near(Some(frame), card),
        "an evicted tile drew {frame:?}, not the low-resolution card"
    );
    window.destroy();
}

/// FR-023: a render that outlives its deadline is abandoned, and the plain
/// text shows with a notice saying why and a way to the source. The render
/// is held by the test, so nothing here depends on a clock.
pub fn a_render_past_its_deadline_shows_the_plain_text() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let view = BodyView::new(std::time::Duration::from_millis(1));
    let scroller = gtk::ScrolledWindow::builder().child(&view).build();
    let window = gtk::Window::builder()
        .default_width(800)
        .default_height(600)
        .child(&scroller)
        .build();
    window.present();
    let mut held = content("html-designed-three-column");
    let resources = Resources::new();
    let gate = resources.hold_lookup();
    held.resources = Arc::new(resources);
    held.plain_text = "the words the sender also sent as text".to_owned();
    view.set_content(held);
    assert!(
        until(|| view
            .document()
            .is_some_and(|d| matches!(d.outcome, postio_render::Outcome::FellBack(_)))),
        "the render past its deadline did not fall back"
    );
    let fallback = view.document().expect("the fallback");
    assert_eq!(
        fallback.outcome,
        postio_render::Outcome::FellBack(postio_render::FallbackReason::Deadline)
    );
    assert!(
        fallback
            .text
            .text
            .contains("the words the sender also sent as text")
    );
    assert!(
        fallback.text.text.contains("took too long"),
        "no notice says why: {:?}",
        fallback.text.text
    );
    assert!(
        view.activate_action("body.view-source", None).is_ok(),
        "no View source action"
    );

    // The late snapshot arrives, and is not shown.
    gate.release();
    let generation = fallback.generation;
    for _ in 0..30 {
        pump();
    }
    let shown = view.document().expect("still a document");
    assert_eq!(
        shown.generation, generation,
        "the late snapshot replaced the fallback"
    );
    window.destroy();
}
