//! The reading surface on a real display (spec 006 research R8): a
//! snapshot arrives, the adjustment spans it, and the pixels on screen are
//! the message's own.

use std::sync::Arc;

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_body::RemoteImages;
use postio_gtk::body_view::{BodyView, Content, DEFAULT_RENDER_DEADLINE};
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

/// Run the main loop until `done`, painting frames; false on timeout.
fn until(done: impl Fn() -> bool) -> bool {
    let context = glib::MainContext::default();
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    while !done() && std::time::Instant::now() < deadline {
        context.iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    done()
}

/// The window's pixels, as the compositor would get them, with the stride
/// of a row in bytes; `None` before the window's first frame.
fn pixels(window: &gtk::Window) -> Option<(usize, Vec<u8>)> {
    let (width, height) = (window.width(), window.height());
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(width), f64::from(height));
    let node = snapshot.to_node()?;
    let renderer = window.native().and_then(|native| native.renderer())?;
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
    let view = BodyView::new(postio_test_support::scaled(DEFAULT_RENDER_DEADLINE));
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
    // From the view's own coordinates to the window's, past its header.
    let point = view
        .compute_point(
            &window,
            &gtk::graphene::Point::new((rect.x0 - 6.0) as f32, (rect.center().y - 100.0) as f32),
        )
        .expect("the view is inside the window");
    let (x, y) = (f64::from(point.x()), f64::from(point.y()));
    let at = |pixels: &[u8], stride: usize| {
        let offset = y.round() as usize * stride + x.round() as usize * 4;
        [pixels[offset], pixels[offset + 1], pixels[offset + 2]]
    };
    let seen = || pixels(&window).map(|(stride, pixels)| at(&pixels, stride));
    let drawn = until(|| seen() == Some([0xdc, 0xeb, 0xe3]));
    assert!(
        drawn,
        "at ({x}, {y}) the window shows {:?}, not the card",
        seen()
    );
    window.destroy();
}
