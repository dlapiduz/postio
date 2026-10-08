//! A render past its deadline shows the message's own plain text (spec 006
//! FR-023; specs/007-postio-focus research R1).
//!
//! The view has always drawn whatever plain text it was handed, under a
//! notice that says the original could not be shown. The reader never handed
//! it any: the fallback was the notice alone, over an empty page, for every
//! message -- including the ones that came with a text part saying exactly
//! what the HTML said.
//!
//! The render is held on the render thread past the reader's own deadline,
//! and what is asserted is the snapshot on screen: what a person would read.
//!
//! Skips without a display. Nothing here touches the network.

use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_model::message::MessageBody;
use postio_render::{FallbackReason, Outcome};
use postio_widgets::reader::{Reader, RemoteImageAllowList};

fn reader_in_a_window() -> Option<(gtk::Window, Reader, tempfile::TempDir)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    crate::support_reader::prepare(&display);
    let dir = tempfile::tempdir().expect("a scratch directory");
    let reader = Reader::with_allowlist(
        Rc::new(|_content_id: &str| None),
        RemoteImageAllowList::default(),
        dir.path().join("allow.json"),
    );
    let window = gtk::Window::new();
    window.set_default_size(900, 700);
    window.set_child(Some(&reader.widget()));
    window.present();
    Some((window, reader, dir))
}

/// The snapshot's text once the render has fallen back at its deadline.
fn fallen_back(reader: &Reader) -> String {
    let view = reader.view();
    crate::support_reader::settle_until("the held render to fall back at its deadline", || {
        view.document()
            .is_some_and(|d| d.outcome == Outcome::FellBack(FallbackReason::Deadline))
    });
    view.document().expect("the fallback").text.text.clone()
}

pub fn a_message_past_its_deadline_shows_its_own_text() {
    let Some((window, reader, _dir)) = reader_in_a_window() else {
        return;
    };
    reader.view().hold_renders();
    reader.render(
        &MessageBody {
            text: Some("The survey starts Monday at the north gate.".to_owned()),
            html: Some(
                "<table style=\"background:#0b3d2e\"><tr><td style=\"color:#f5f0e1\">\
                 The survey starts <b>Monday</b> at the north gate.</td></tr></table>"
                    .to_owned(),
            ),
        },
        Some("ada@example.com"),
    );
    let shown = fallen_back(&reader);
    reader.view().release_renders();
    assert!(
        shown.contains("took too long"),
        "the fallback does not say why the original is missing: {shown:?}"
    );
    assert!(
        shown.contains("The survey starts Monday at the north gate."),
        "the message came with a text part, and its fallback shows none of it: {shown:?}"
    );
    window.destroy();
}
