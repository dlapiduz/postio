//! The privacy pane's read-receipt count, wired end to end (#970).
//!
//! Postio never sends a read receipt automatically (CLAUDE.md's privacy
//! section) — this proves the pane says how often one was asked, not that
//! anything acts on it: a message with `Disposition-Notification-To` lands
//! in the store, and opening settings shows the count in its text, no
//! switch anywhere near it.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe. Set before the app under test
// starts, which is the one moment it is sound; the library forbids `unsafe`.

use crate::settle;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_storage::repository::MessageRepository;
use postio_storage::{BlobStore, test_support};

const ASKED: &[u8] = b"From: Newsletter <news@example.org>\r\n\
To: Ada Lovelace <ada@example.com>\r\n\
Subject: Please confirm receipt\r\n\
Disposition-Notification-To: news@example.org\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Let us know you got this\r\n";

pub fn opening_settings_shows_how_many_messages_asked_for_a_receipt() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under scripts/test-headless.sh)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let repository = MessageRepository::new(&connection);
        let mut asked =
            postio_model::mime::parse(ASKED).into_message(account.id, inbox, chrono::Utc::now());
        repository.create(&mut asked).expect("a message");
    }

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs,
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    settle();
    let _wired = feed_the_window(&window, &wiring).expect("the store has an account");
    settle();

    window.act(postio_core::Command::Settings);
    while glib::MainContext::default().iteration(false) {}

    let label = find_label(
        &window.clone().upcast(),
        "postio-settings-read-receipt-count",
    )
    .expect("the privacy pane always draws the read-receipt count line");
    assert!(
        label.contains('1'),
        "one message asked for a receipt: {label}"
    );
    assert!(
        label.contains("none have been sent"),
        "the line states the fixed no-automatic-sending policy: {label}"
    );

    bridge.shutdown();
}

fn find_label(widget: &gtk::Widget, class: &str) -> Option<String> {
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && widget.has_css_class(class)
    {
        return Some(label.label().to_string());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find_label(&current, class) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}
