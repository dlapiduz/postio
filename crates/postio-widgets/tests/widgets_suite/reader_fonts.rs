//! The reader's typefaces reach the renderer, and the document does not
//! carry them (#768, ADR 0023).
//!
//! Every document used to inline ~1.21 MB of base64 `@font-face` data, on
//! every render. The faces are Postio's own `FontSet` now, registered once
//! per process and handed to every render (spec 006 research R3); the
//! document names them and never carries them.
//!
//! What WebKit made worth a display -- that the engine *fetched* a face
//! over `postio-font:`, and that a warmed web process fetched none for the
//! first message -- has no equivalent: nothing is fetched. What replaces
//! it is the font set answering for Postio's families here, and the
//! renderer shaping with them in `postio-render`'s `fonts` suite.
//!
//! Skips without a display. Nothing here touches the network.

use crate::support_reader::pump;
use std::rc::Rc;
use std::time::Instant;

use gtk::gdk;
use gtk::prelude::*;
use postio_model::message::MessageBody;
use postio_widgets::reader::{BlobSource, Reader, RemoteImageAllowList};

pub fn the_faces_are_the_readers_own_and_not_carried_by_the_document() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, reader) = reader("allowlist");
    reader.render(&body("<p>The tide gate interlock is fixed.</p>"), None);
    wait_for_text(&reader, "tide gate");

    // The families the reader's sheet names resolve in the set every render
    // is given; a name nothing answers is a silent fall back to whatever
    // sans the machine has.
    let fonts = postio_widgets::body_view::font_set();
    for family in ["Barlow", "Barlow Condensed", "IBM Plex Mono"] {
        assert!(
            fonts.has_family(family),
            "{family} is not in the reader's font set"
        );
    }
    assert_eq!(fonts.generic("sans-serif").as_deref(), Some("Barlow"));

    // The whole of #768: what a message change costs is the message, not
    // the typeface catalogue.
    let document = postio_ui::reader::document::document_for(
        "<p>The tide gate interlock is fixed.</p>",
        "",
        postio_body::RemoteImages::Blocked,
        postio_ui::reader::document::Sheet::Theme,
    );
    assert!(
        !document.contains("data:font/"),
        "the faces are travelling with the document again"
    );
    assert!(
        document.len() < 64 * 1024,
        "the document is {} bytes for a one-line message",
        document.len()
    );
    window.close();
}

/// The first message is drawn, not the deadline's plain-text fallback.
///
/// Font discovery reads every installed face's tables, which is the one
/// slow thing a first render could wait on; warming the reader starts it
/// on the application's first idle turn, so the first message a person
/// opens is rendered in full.
pub fn a_warmed_reader_draws_its_first_message_in_full() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, reader) = reader("warm-allowlist");
    reader.warm();
    reader.render(
        &body("<p>Plain, <b>bold</b>, <i>italic</i> and <code>mono</code>.</p>"),
        None,
    );
    wait_for_text(&reader, "italic");
    let document = reader.view().document().expect("a snapshot");
    assert_eq!(
        document.outcome,
        postio_render::Outcome::Rendered,
        "the first message fell back instead of rendering"
    );
    window.close();
}

fn reader(name: &str) -> (gtk::Window, Reader) {
    let window = gtk::Window::new();
    window.set_default_size(600, 500);
    let source: Rc<dyn BlobSource> = Rc::new(|_: &str| None);
    let reader =
        Reader::with_allowlist(source, RemoteImageAllowList::default(), scratch_path(name));
    window.set_child(Some(&reader.widget()));
    window.present();
    pump();
    (window, reader)
}

fn wait_for_text(reader: &Reader, text: &str) {
    let deadline = Instant::now() + postio_test_support::patience();
    while Instant::now() < deadline {
        if reader
            .view()
            .document()
            .is_some_and(|d| d.text.text.contains(text))
        {
            return;
        }
        while glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the reader never drew {text:?}");
}

fn body(html: &str) -> MessageBody {
    MessageBody {
        text: None,
        html: Some(html.to_owned()),
    }
}

fn scratch_path(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("postio-widgets-fonts-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(format!("{name}.ini"))
}
