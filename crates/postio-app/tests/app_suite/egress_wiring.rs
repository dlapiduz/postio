//! The egress log, wired end to end (#151): the proof half of the privacy
//! claim.
//!
//! Three assertions, in the order the documents promise them:
//!
//! 1. **Opening the application costs zero connections.** `feed_the_window`
//!    over a seeded store — the panes, the search, the settings — leaves
//!    the egress log empty, because nothing here touches the network and
//!    now that is *shown*, not asserted.
//! 2. An event a transport reports crosses the wiring's recorder into the
//!    store, stamped with its account.
//! 3. Opening settings lists it — the audit surface a user reads.
//!
//! One test function, for the reason `wiring.rs` gives.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe; set before the app starts.

use crate::settle;
use gtk::prelude::*;
use gtk::gdk;
use postio_app::feed_the_window;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::egress::{EgressEvent, EgressOutcome, EgressSubsystem};
use postio_session::Wiring;
use postio_storage::repository::EgressLogRepository;
use postio_storage::seed::seed_small;
use postio_storage::{BlobStore, test_support};



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

pub fn opening_the_app_costs_zero_connections_and_the_log_is_auditable() {
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
    let report = seed_small(&database, 47);
    let account = report.account.id;
    let directory = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(directory.path().to_path_buf()).expect("a blob store");

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
    settle();

    // ── 1. nothing left this machine ─────────────────────────────────────
    {
        let connection = database.connection().expect("a connection");
        assert_eq!(
            EgressLogRepository::new(&connection)
                .count()
                .expect("count"),
            0,
            "feeding the whole window made an outbound connection — the \
             privacy claim just became false in the default suite"
        );
    }

    // ── 2. a transport's report reaches the store, account stamped ───────
    wiring.egress.for_account(account).record(EgressEvent {
        at: chrono::Utc::now(),
        subsystem: EgressSubsystem::Imap,
        account: None,
        host: "imap.example.com".to_string(),
        port: 993,
        outcome: EgressOutcome::Connected,
    });
    let landed = settle_until(|| {
        let connection = database.connection().expect("a connection");
        EgressLogRepository::new(&connection)
            .count()
            .expect("count")
            == 1
    });
    assert!(
        landed,
        "the recorder's writer thread never persisted the event"
    );
    {
        let connection = database.connection().expect("a connection");
        let rows = EgressLogRepository::new(&connection)
            .recent(10)
            .expect("recent");
        assert_eq!(rows[0].account, Some(account));
        assert_eq!(rows[0].host, "imap.example.com");
    }

    // ── 3. opening settings lists it ─────────────────────────────────────
    window.act(postio_core::Command::Settings);
    settle();
    let listed = find(&window.clone().upcast(), &|widget| {
        widget.has_css_class("postio-settings-egress-row")
    });
    assert!(
        listed.is_some(),
        "the settings panel lists no connections over a log that holds one"
    );

    bridge.shutdown();
}

/// Depth-first search of a widget tree.
fn find(widget: &gtk::Widget, wanted: &dyn Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if wanted(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find(&current, wanted) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}
