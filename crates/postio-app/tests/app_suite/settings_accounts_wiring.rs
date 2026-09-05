//! Issue #464: the settings panel's account rows, wired to a real store.
//!
//! `gtk_settings_accounts.rs` (in `postio-gtk`) proves the panel's own
//! seam reacts to whatever a provider answers, and `gtk_toast.rs` proves
//! `Toast::show_removable`'s button actually calls back. Neither proves the
//! two ever meet: that `postio_app::settings_accounts::install`, run over a
//! real seeded database, actually shows both accounts, persists the enable
//! switch, and marks (never immediately deletes) a removed account.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle;
use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::EmailAddress;
use postio_session::Wiring;
use postio_storage::repository::AccountRepository;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

pub fn account_rows_persist_enable_and_mark_removal() {
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

    // A second account: "one row per account" proves nothing with only the
    // one `seed_small` itself creates.
    let connection = database.connection().expect("a connection");
    let mut second = postio_model::Account::new(
        "Work",
        EmailAddress::new(None::<String>, "work@example.com"),
    );
    let second_id = AccountRepository::new(&connection)
        .create(&mut second)
        .expect("insert a second account");
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
    settle();

    let _wired = feed_the_window(&window, &wiring).expect("the seeded store has an account");
    let panel = window.settings();

    // ── both accounts show up, without anyone telling the panel to look ──
    assert!(
        settle_until(|| rows(&panel).len() == 2),
        "expected both accounts drawn as rows, got {} row(s)",
        rows(&panel).len()
    );

    // ── flipping the second account's switch persists to its own row ────
    let switch = switch_in(&rows(&panel)[1]);
    let before = switch.is_active();
    assert!(before, "Account::new starts an account enabled");
    switch.set_active(!before);
    assert!(
        settle_until(|| !read_enabled(&database, second_id)),
        "the switch flip should have reached the database"
    );

    // Flip it back so the removal case below starts from a known state.
    switch.set_active(true);
    assert!(settle_until(|| read_enabled(&database, second_id)));

    // ── Remove marks the account rather than deleting it outright ───────
    // The panel starts hidden (`window.rs` builds it with `set_visible(false)`
    // until something asks for it), and a hidden row has no real geometry
    // for `row_at_y` to find -- `open_account_menu` silently finds nothing
    // there, which is exactly what happened before this was added.
    window.toggle_settings();
    assert!(
        frames(&window, 2),
        "the compositor never painted the settings panel"
    );
    let target_y = row_y(&rows(&panel)[1]);
    panel.test_open_account_menu(1.0, target_y);
    assert!(
        panel.activate_action("account.remove", None).is_ok(),
        "Remove should exist on an account row"
    );
    panel.test_close_account_menu();

    assert!(
        settle_until(|| read_pending(&database, second_id)),
        "Remove must mark the row pending, not delete it -- Q6's undo needs \
         something to undo"
    );
    assert!(
        settle_until(|| rows(&panel).len() == 1),
        "a removed account should stop showing immediately, got {} row(s)",
        rows(&panel).len()
    );

    bridge.shutdown();
}

fn read_enabled(database: &postio_storage::Database, id: postio_model::ids::AccountId) -> bool {
    let connection = database.connection().expect("a connection");
    AccountRepository::new(&connection)
        .get(id)
        .expect("get")
        .expect("still there")
        .enabled
}

fn read_pending(database: &postio_storage::Database, id: postio_model::ids::AccountId) -> bool {
    let connection = database.connection().expect("a connection");
    AccountRepository::new(&connection)
        .get(id)
        .expect("get")
        .expect("still there")
        .pending_deletion
}

fn switch_in(row: &gtk::ListBoxRow) -> gtk::Switch {
    collect(row.upcast_ref::<gtk::Widget>(), "")
        .into_iter()
        .find_map(|w| w.downcast::<gtk::Switch>().ok())
        .expect("every account row has a switch")
}

/// Run the main loop until `window` has actually painted `count` frames.
///
/// `is_mapped()` becoming true is not enough -- a row can be mapped with its
/// bounds still the pre-layout placeholder. Copied from
/// `gtk_sidebar_saved_searches.rs::frames` rather than shared, matching
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

fn row_y(row: &gtk::ListBoxRow) -> f64 {
    let parent = row.parent().expect("a row in a list has a parent");
    let bounds = row
        .compute_bounds(&parent)
        .expect("a mapped row has bounds");
    (bounds.y() + bounds.height() / 2.0) as f64
}

fn rows(panel: &postio_gtk::settings::SettingsPanel) -> Vec<gtk::ListBoxRow> {
    collect(
        panel.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-row",
    )
    .into_iter()
    .filter_map(|w| w.downcast().ok())
    .collect()
}

/// Every widget in the tree carrying `class` (or, when `class` is empty,
/// every widget), depth first -- copied from `gtk_settings_accounts.rs`
/// rather than shared, matching that file's own reason for copying it from
/// `gtk_sidebar_saved_searches.rs`: no dependency between the three.
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
