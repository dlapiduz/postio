use crate::support_reader::pump;
use std::rc::Rc;
use std::time::Instant;

use gtk::prelude::*;
use postio_model::message::MessageBody;
use postio_widgets::reader::{BlobSource, Reader, RemoteImageAllowList};

pub fn the_faces_are_the_readers_own_and_not_carried_by_the_document() {
    let (window, reader) = reader("allowlist");
    reader.render(&body("<p>The tide gate interlock is fixed.</p>"), None);
    wait_for_text(&reader, "tide gate");

    let fonts = postio_widgets::body_view::font_set();
    for family in ["Barlow", "Barlow Condensed", "IBM Plex Mono"] {
        assert!(
            fonts.has_family(family),
            "{family} is not in the reader's font set"
        );
    }
    assert_eq!(fonts.generic("sans-serif").as_deref(), Some("Barlow"));

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

/// Font discovery is the one slow thing a first render could wait on; warming starts it on the
/// first idle turn.
pub fn a_warmed_reader_draws_its_first_message_in_full() {
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
