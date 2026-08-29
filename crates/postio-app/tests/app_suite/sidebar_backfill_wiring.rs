//! ADR 0016, #350: the sidebar's own context menu, wired to a real store.
//!
//! `crates/postio-gtk/tests/gtk_suite/gtk_sidebar_backfill_exclusion.rs`
//! proves the menu's own seam reacts correctly. Neither that nor
//! `postio-storage`'s repository tests prove the two ever meet: that
//! `postio_app::sidebar_backfill::install`, run over a real seeded database,
//! actually persists the toggle and shows the sidebar the result without
//! waiting for a sync that will never come.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::mailbox::MailboxRole;
use postio_session::Wiring;
use postio_storage::repository::MailboxRepository;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

fn settle_until(done: impl Fn() -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        settle();
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}

/// Depth-first search of a widget tree for the first one carrying `class`.
fn find(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find(&current, class) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

pub fn the_menu_persists_and_the_sidebar_reflects_it_without_a_sync() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    let report = seed_small(&database, 61);
    let inbox = report.mailbox(MailboxRole::Inbox).expect("a seeded inbox");
    let inbox_id = inbox.id;
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

    let (bridge, _replies) =
        postio_core::bridge::Bridge::new(postio_core::bridge::handler_fn(|_, _| async {}))
            .expect("a runtime");
    let (sink, _events) = postio_core::bridge::event_channel();
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

    let _wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    let sidebar = window.sidebar();

    assert!(
        settle_until(|| !sidebar.mailboxes().is_empty()),
        "the seeded folders should have reached the sidebar"
    );
    assert!(
        !read_excluded(&database, inbox_id),
        "every selectable folder backfills by default (ADR 0016)"
    );

    // ── the context menu on Inbox's own row toggles it ─────────────────
    let inbox_row: gtk::ListBoxRow = find(sidebar.upcast_ref::<gtk::Widget>(), "postio-folder")
        .and_then(|w| w.downcast().ok())
        .expect("Inbox is the first special-use row");
    sidebar.test_open_special_folder_menu(&inbox_row);
    assert!(
        sidebar
            .activate_action("folder.toggle-backfill", None)
            .is_ok(),
        "the toggle entry should exist on Inbox's own context menu"
    );
    sidebar.test_close_folder_menu();

    // ── persisted, and shown without any sync ever running ─────────────
    assert!(
        settle_until(|| read_excluded(&database, inbox_id)),
        "the toggle should have reached the database"
    );
    assert!(
        settle_until(|| sidebar
            .mailboxes()
            .iter()
            .find(|m| m.id == inbox_id)
            .is_some_and(|m| m.backfill_excluded)),
        "the sidebar's own cached list should reflect the write immediately, \
         not wait for Event::MailboxesChanged from a sync pass that never runs here"
    );

    bridge.shutdown();
}

fn read_excluded(database: &postio_storage::Database, id: postio_model::ids::MailboxId) -> bool {
    let connection = database.connection().expect("a connection");
    MailboxRepository::new(&connection)
        .backfill_excluded(id)
        .expect("read")
}
