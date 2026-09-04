//! Issue #981: the account row's "Rebuild search index" action, wired to a
//! real store.
//!
//! `gtk_settings_accounts.rs` (in `postio-gtk`) proves the panel's own row
//! renders whatever progress it is told about; `postio-session`'s own
//! `reindex_account.rs` proves the pass itself rebuilds the right account's
//! rows and no other's. Neither proves the two meet: that clicking the row's
//! real menu item, over a real seeded database, actually clears and refills
//! that account's local index.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::{settle, settle_until};
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_app::feed_the_window;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::BodyState;
use postio_session::Wiring;
use postio_storage::repository::MessageRepository;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};

pub fn the_rows_own_action_clears_and_refills_its_accounts_local_index() {
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
    let report = seed_small(&database, 41);
    let account = report.account.id;
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(
        directory.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");

    // A message with a local, indexed body -- the state a rebuild has
    // something real to clear and refill.
    let connection = database.connection().expect("a connection");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let inbox = report
        .mailboxes
        .first()
        .expect("seed_small seeds at least one mailbox")
        .id;
    let mut message = postio_model::Message::new(account, inbox, chrono::Utc::now());
    message.sync.body_state = BodyState::Full;
    MessageRepository::new(&connection)
        .create(&mut message)
        .expect("create a message");
    postio_index::index::index_body_of(
        &connection,
        message.id.get(),
        &postio_model::MessageBody {
            text: Some("the analytical engine".to_owned()),
            html: None,
        },
    )
    .expect("index its body the ordinary way");
    assert!(
        postio_index::index::messages_missing_body_text_for_account(&connection, account.get(), 10)
            .expect("candidates")
            .is_empty(),
        "indexed once already, so nothing should be missing yet"
    );
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
    assert!(settle_until(|| rows(&panel).len() == 1));

    window.toggle_settings();
    assert!(
        frames(&window, 2),
        "the compositor never painted the settings panel"
    );
    let target_y = row_y(&rows(&panel)[0]);
    panel.test_open_account_menu(1.0, target_y);
    assert!(
        panel.activate_action("account.rebuild-index", None).is_ok(),
        "Rebuild search index should exist on an account row"
    );
    panel.test_close_account_menu();

    // The rebuild clears the row before it refills it (that is the whole
    // shape #981 asks for), so the message is genuinely missing for a
    // moment -- and by the time the pass reports itself finished, refilled
    // again.
    assert!(
        settle_until(|| {
            let connection = database.connection().expect("a connection");
            postio_index::index::messages_missing_body_text_for_account(
                &connection,
                account.get(),
                10,
            )
            .expect("candidates")
            .is_empty()
        }),
        "the rebuild should have refilled the account's own index"
    );
    assert!(
        settle_until(|| reindexing_in(&rows(&panel)[0]).is_none()),
        "the row's progress line should clear once the rebuild is over"
    );

    bridge.shutdown();
}

/// The row's own reindex-progress line, if a rebuild is running (#981) --
/// copied from `gtk_settings_accounts.rs` rather than shared, matching
/// every other helper in this file.
fn reindexing_in(row: &gtk::ListBoxRow) -> Option<String> {
    collect(
        row.upcast_ref::<gtk::Widget>(),
        "postio-settings-account-reindexing",
    )
    .into_iter()
    .find_map(|w| w.downcast::<gtk::Label>().ok())
    .filter(|label| label.is_visible())
    .map(|label| label.text().to_string())
}

/// Run the main loop until `window` has actually painted `count` frames.
///
/// Copied from `settings_accounts_wiring.rs::frames` rather than shared,
/// matching that file's own reason: no dependency between the two.
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

/// Every widget in the tree carrying `class`, depth first -- copied rather
/// than shared, matching `settings_accounts_wiring.rs`'s own reason.
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
