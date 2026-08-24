//! Can a person actually open or "Open with…" an attachment from the
//! running application?
//!
//! `postio-gtk`'s own test (`gtk_parts.rs`) proves the panel's `Ret` and `x`
//! keys ask -- through `connect_open` and `connect_external` -- and stop
//! there, because the panel itself must never be able to fetch. Nothing in
//! that test proves anything is listening. `postio-m2ex` is exactly the shape
//! of bug `postio-bl2` names: a capability fully built, fully unit-tested,
//! and reachable from nowhere, because nothing in `postio-app` ever called
//! `connect_open` or `connect_external`.
//!
//! So this starts where the application starts: a real store with a real
//! multipart message, a real `Window`, and `feed_the_window` -- the same call
//! `run` makes. Then it opens the parts panel the way a person does, presses
//! the real keys, and reads the file back off disk. What it does not assert
//! is that the desktop's own launcher opens anything: there is no portal in
//! a headless test, and the seam this proves ends where GTK's own,
//! independently-shipped launcher begins.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::{Wiring, feed_the_window};
use postio_core::bridge::{Bridge, event_channel, handler_fn};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::Message;
use postio_storage::repository::MessageRepository;
use postio_storage::{BlobStore, test_support};

/// A text body and a named, non-previewable attachment -- so both `Ret` (not
/// previewable, forces the chooser) and `x` (always forces it) take the same
/// path through `PartOpener`.
const RAW: &[u8] = b"From: Ada Lovelace <ada@example.com>\r\n\
To: Grace Hopper <grace@example.net>\r\n\
Subject: Quarterly figures\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"edge\"\r\n\
\r\n\
--edge\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
See the attached figures.\r\n\
--edge\r\n\
Content-Type: text/csv\r\n\
Content-Disposition: attachment; filename=\"figures.csv\"\r\n\
\r\n\
one,two\r\n\
--edge--\r\n";

fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

#[test]
fn opening_and_open_with_ing_a_part_reach_the_desktop() {
    let state_dir = std::env::temp_dir().join(format!("postio-parts-open-{}", std::process::id()));
    let export_dir = state_dir.join("export");
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statements of a single-threaded test.
    unsafe {
        std::env::set_var("XDG_STATE_HOME", &state_dir);
        std::env::set_var("POSTIO_EXPORT_DIR", &export_dir);
    }

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under scripts/test-headless.sh)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // ── a store with one account, one folder, and a real attached message ──
    let database = test_support::memory();
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.keep()).expect("a blob store");

    {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let parsed = postio_model::mime::parse(RAW);
        let mut message = Message::new(account.id, inbox, chrono::Utc::now());
        message.subject = Some("Quarterly figures".into());
        // Already downloaded: this proves the wiring reaches the desktop, not
        // that a fetch happens first -- `postio_app::reading::part_bytes`
        // covers the fetch-first case on its own, without a display.
        message.raw_blob_id = Some(blobs.put(RAW).expect("a blob"));
        message.attachments = parsed
            .parts
            .iter()
            .map(|part| part.attachment.clone())
            .collect();
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("a message");
    }

    let (bridge, _replies) = Bridge::new(handler_fn(|_, _| async {})).expect("a runtime");
    let (sink, _events) = event_channel();
    let wiring = Wiring::new(database, blobs, bridge.handle(), sink, bridge.commands());

    let window = Window::default();
    window.present();
    while glib::MainContext::default().iteration(false) {}

    // ── the same call `run` makes ───────────────────────────────────────
    let _wired = feed_the_window(&window, &wiring).expect("the store has an account");
    while glib::MainContext::default().iteration(false) {}

    // ── open the message, exactly as a double click or `Enter` does ─────
    activate_first_row(&window);
    assert!(
        settle_until(|| window.reading()),
        "the message was never opened"
    );

    // ── the chip is the only way into the panel from a running window ───
    let chip = settle_for_chip(&window).unwrap_or_else(|| {
        panic!("no attachment chip appeared, so the panel can never be reached")
    });
    chip.emit_clicked();
    while glib::MainContext::default().iteration(false) {}
    assert!(
        window.parts().is_visible(),
        "the chip should open the panel"
    );

    // ── walk to the attachment; the panel starts on the first part ──────
    let panel = window.parts();
    while panel.cursor().map(|node| node.mime) != Some("text/csv".to_owned()) {
        assert!(
            panel.press(gdk::Key::j),
            "walked off the end of the tree before finding the attachment"
        );
    }

    let saved = export_dir.join("figures.csv");

    // ── `Ret`: connect_open ──────────────────────────────────────────────
    assert!(!saved.exists(), "nothing should be there before Ret");
    assert!(panel.press(gdk::Key::Return));
    assert!(
        settle_until(|| saved.exists()),
        "pressing Ret in the parts panel never produced a file. Every layer \
         under this one is unit-tested and passes -- check whether anything \
         calls PartsPanel::connect_open."
    );
    assert_eq!(
        std::fs::read(&saved).expect("the file"),
        b"one,two",
        "the file opened is not the bytes the sender attached"
    );

    // ── `x`: connect_external, independently of `Ret` ────────────────────
    std::fs::remove_file(&saved).expect("the fixture cleans up its own file");
    assert!(panel.press(gdk::Key::x));
    assert!(
        settle_until(|| saved.exists()),
        "pressing x (\"Open with…\") in the parts panel never produced a \
         file -- check whether anything calls PartsPanel::connect_external."
    );

    bridge.shutdown();
    let _ = std::fs::remove_dir_all(&state_dir);
}

/// The one chip a message with a named attachment gets, once it appears.
fn settle_for_chip(window: &Window) -> Option<gtk::Button> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if let Some(chip) = chips(window).into_iter().next() {
            return Some(chip);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}

fn chips(window: &Window) -> Vec<gtk::Button> {
    let mut found = Vec::new();
    walk(window.upcast_ref::<gtk::Widget>(), &mut |widget| {
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && button.has_css_class("postio-attachment")
        {
            found.push(button.clone());
        }
    });
    found
}

fn walk(widget: &gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(widget);
    let mut child = widget.first_child();
    while let Some(node) = child {
        walk(&node, visit);
        child = node.next_sibling();
    }
}

fn activate_first_row(window: &Window) {
    let view = find_list_view(window.upcast_ref::<gtk::Widget>())
        .expect("the message list is built on a GtkListView");
    view.emit_by_name::<()>("activate", &[&0u32]);
}

fn find_list_view(widget: &gtk::Widget) -> Option<gtk::ListView> {
    if let Some(view) = widget.downcast_ref::<gtk::ListView>() {
        return Some(view.clone());
    }
    let mut child = widget.first_child();
    while let Some(node) = child {
        if let Some(found) = find_list_view(&node) {
            return Some(found);
        }
        child = node.next_sibling();
    }
    None
}
