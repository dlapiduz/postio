//! Issue #880: the account detail view's edits, wired to a real store.
//!
//! `gtk_settings_account_detail.rs` (in `postio-gtk`) proves the panel's
//! own `connect_account_edited` seam fires with the right value; this
//! proves the other half -- that `postio_app::settings_accounts::install`
//! actually writes an edit through `AccountRepository::update` and the row
//! reflects it back, over a real seeded database. No `config.toml`
//! involved: ADR 0005 Q6b retired accounts from the file.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_gtk::settings::SettingsPanel;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_session::Wiring;
use postio_storage::repository::AccountRepository;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

pub fn editing_the_detail_view_writes_straight_to_the_accounts_table() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let database = test_support::memory();
    seed_small(&database, 41);
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    let connection = database.connection().expect("a connection");
    let seeded_id = AccountRepository::new(&connection)
        .list()
        .expect("list")
        .first()
        .expect("seed_small seeds one account")
        .id;
    drop(connection);

    let (bridge, _replies) =
        postio_core::bridge::Bridge::new(postio_core::bridge::handler_fn(|_, _| async {}))
            .expect("a runtime");
    let (sink, _events) = postio_core::bridge::event_channel();
    let wiring = Wiring::new(
        database.clone(),
        blobs.clone(),
        bridge.handle(),
        sink,
        bridge.commands(),
    );

    let window = Window::default();
    window.present();
    // Give the window a chance to actually map and pick up a frame clock
    // before anything below asks it to paint -- `present()` only schedules
    // that, and `frames()`'s own pumping cannot make up for zero prior
    // iterations on a runner slow to map a brand new window (matches
    // `settings_accounts_wiring.rs`'s own `window.present(); settle();`).
    pump();
    let _wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    let panel = window.settings();

    assert!(
        settle_until(|| !rows(&panel).is_empty()),
        "expected at least one account row"
    );

    window.toggle_settings();
    assert!(
        frames(&window, 2),
        "the compositor never painted the settings panel"
    );

    panel.open_account_detail(seeded_id);
    pump();

    let entry = display_name_entry(&panel);
    entry.set_text("Renamed");
    entry.emit_activate();

    assert!(
        settle_until(|| read_display_name(&database, seeded_id) == "Renamed"),
        "editing the display name should have reached the database"
    );

    let imap_host = imap_host_entry(&panel);
    imap_host.set_text("imap.new-host.example.com");
    imap_host.emit_activate();

    assert!(
        settle_until(|| read_imap_host(&database, seeded_id) == "imap.new-host.example.com"),
        "editing the IMAP host should have reached the database"
    );

    bridge.shutdown();
}

fn pump() {
    let context = glib::MainContext::default();
    while context.iteration(false) {}
}

fn read_display_name(
    database: &postio_storage::Database,
    id: postio_model::ids::AccountId,
) -> String {
    let connection = database.connection().expect("a connection");
    AccountRepository::new(&connection)
        .get(id)
        .expect("get")
        .expect("still there")
        .display_name
}

fn read_imap_host(database: &postio_storage::Database, id: postio_model::ids::AccountId) -> String {
    let connection = database.connection().expect("a connection");
    AccountRepository::new(&connection)
        .get(id)
        .expect("get")
        .expect("still there")
        .incoming
        .host
}

/// Run the main loop until `window` has actually painted `count` frames.
/// Copied from `settings_accounts_wiring.rs` rather than shared, matching
/// that file's own reason: no dependency between the two.
fn frames(window: &Window, count: u32) -> bool {
    let left = std::rc::Rc::new(std::cell::Cell::new(count));
    window.add_tick_callback({
        let left = left.clone();
        move |_, _| {
            left.set(left.get().saturating_sub(1));
            if left.get() == 0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        }
    });
    let context = glib::MainContext::default();
    let heartbeat = glib::timeout_add_local(std::time::Duration::from_millis(10), || {
        glib::ControlFlow::Continue
    });
    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(5));
    while left.get() > 0 && std::time::Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
    left.get() == 0
}

fn rows(panel: &SettingsPanel) -> Vec<gtk::ListBoxRow> {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-row",
    )
    .into_iter()
    .filter_map(|w| w.downcast().ok())
    .collect()
}

fn display_name_entry(panel: &SettingsPanel) -> gtk::Entry {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-detail-display-name",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Entry>().ok())
    .expect("the detail view has a display name entry")
}

fn imap_host_entry(panel: &SettingsPanel) -> gtk::Entry {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-detail-imap-host",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Entry>().ok())
    .expect("the detail view has an IMAP host entry")
}

/// Every widget in the tree carrying `class` (or, when `class` is empty,
/// every widget). Copied from `settings_accounts_wiring.rs` rather than
/// shared, matching that file's own reason: no dependency between the two.
fn collect(widget: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    if class.is_empty() || widget.has_css_class(class) {
        found.push(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        found.extend(collect(&current, class));
        child = current.next_sibling();
    }
    found
}
