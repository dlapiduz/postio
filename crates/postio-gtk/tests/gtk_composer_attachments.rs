//! Attachments: added via the file chooser or drag-and-drop, both of which
//! converge on the same `add_file`; shown with name and size; removed before
//! send cleans the draft up.
//!
//! Its own file: GTK is single-threaded and initialised once, so one
//! `#[test]` per integration binary. See `gtk_composer.rs`.
//!
//! `GtkFileDialog` and GTK's own drag machinery cannot be driven headlessly,
//! so what is tested here is what both converge on:
//! `Composer::test_attach_path` calls exactly what a chosen or dropped
//! `gio::File` reaches.
//!
//! `connect_attach`'s handler answers through a callback rather than a
//! return value specifically so a real handler can hand a large file's bytes
//! to a background task and call back once they land in the blob store
//! (`postio-c16.6`) — the deferred-handler case below proves the composer
//! never assumes that callback runs before `add_file` returns.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::composer;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::MessageId;
use postio_model::{AccountId, Attachment, Draft};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

#[test]
fn attaching_shows_the_row_and_removing_cleans_it_up() {
    let state_dir = std::env::temp_dir().join(format!(
        "postio-composer-attachments-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.present();
    settle();

    let composer = composer::install(&window);
    composer.open(Draft::new(AccountId::UNASSIGNED));
    settle();

    let file_path = state_dir.join("report.pdf");
    std::fs::write(&file_path, b"not a real pdf").unwrap();

    // ── Nothing connected: the file goes nowhere, and says so ─────────────
    composer.test_attach_path(&file_path);
    settle();
    assert_eq!(composer.test_attachment_count(), 0);
    assert!(!composer.test_attachments_visible());
    assert!(
        composer.status().contains("not attached"),
        "status should name the missing handler, not stay silent: {}",
        composer.status()
    );

    // ── Connected, and answering synchronously: attaching shows the row ───
    composer.connect_attach(|path, then| {
        let mut attachment = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 2_048);
        attachment.filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        then(Some(attachment));
    });
    composer.test_attach_path(&file_path);
    settle();
    assert_eq!(composer.test_attachment_count(), 1);
    assert!(composer.test_attachments_visible());
    let draft = composer.draft();
    assert_eq!(draft.attachments.len(), 1);
    assert_eq!(draft.attachments[0].filename.as_deref(), Some("report.pdf"));
    assert_eq!(draft.attachments[0].size, 2_048);

    // ── A handler that answers later — the point of the callback shape ───
    // Stands in for a real handler spawning a slow blob-store write and
    // calling back once it finishes: nothing here blocks on `then` ever
    // being called, and the row still appears once it is.
    composer.connect_attach(|_, then| {
        glib::idle_add_local_once(move || {
            then(Some(Attachment::new(
                MessageId::UNASSIGNED,
                "application/zip",
                64 * 1024 * 1024,
            )));
        });
    });
    composer.test_attach_path(&file_path);
    settle();
    assert_eq!(
        composer.test_attachment_count(),
        2,
        "the row appears once the deferred handler calls back"
    );

    // ── Removing before send cleans up ────────────────────────────────────
    composer.test_remove_attachment(0);
    settle();
    assert_eq!(
        composer.test_attachment_count(),
        1,
        "removing index 0 should drop the first attachment, not the second"
    );
    assert_eq!(composer.draft().attachments[0].size, 64 * 1024 * 1024);

    composer.test_remove_attachment(0);
    settle();
    assert_eq!(composer.test_attachment_count(), 0);
    assert!(
        !composer.test_attachments_visible(),
        "the row hides again once nothing is left to show"
    );
}
