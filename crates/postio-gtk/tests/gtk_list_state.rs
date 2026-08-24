//! `postio-ma4`: offline and failing states must not hide mail that is
//! already synced and readable. `derive`/`State::placement`'s own logic is
//! unit-tested with no display in `list_state.rs`; what needs a real widget
//! is that `ListStateView::render` actually acts on `Placement` — becoming a
//! banner rather than the opaque full-pane plate the moment there are rows
//! loaded underneath it, and reverting once there are not.
//!
//! Its own file: GTK is single-threaded and initialised once, so one
//! `#[test]` per integration binary. See `gtk_composer.rs`.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use gtk::gdk;
use gtk::prelude::*;
use postio_core::ConnectionState;
use postio_gtk::list_state::ListStateView;
use postio_gtk::sidebar::SyncStatus;
use postio_gtk::{app, fonts, style};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

fn status(state: ConnectionState) -> SyncStatus {
    SyncStatus {
        state,
        ..SyncStatus::default()
    }
}

#[test]
fn offline_becomes_a_banner_over_rows_and_a_full_plate_over_none() {
    let state_dir = std::env::temp_dir().join(format!("postio-list-state-{}", std::process::id()));
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

    let view = ListStateView::new();
    settle();

    // ── Offline with rows already loaded: a banner, rows stay visible ────
    view.set_status(status(ConnectionState::Offline), 12, 4000, 2);
    settle();
    assert!(view.is_visible(), "offline still has something to say");
    assert!(
        view.has_css_class("postio-liststate-banner"),
        "rows are loaded, so this must not be the opaque full-pane plate"
    );
    assert_eq!(view.valign(), gtk::Align::Start);
    assert!(!view.vexpands());

    // ── Offline with nothing loaded: the full plate, nothing to protect ──
    view.set_status(status(ConnectionState::Offline), 0, 4000, 2);
    settle();
    assert!(view.is_visible());
    assert!(
        !view.has_css_class("postio-liststate-banner"),
        "an empty mailbox has nothing under the plate to hide"
    );
    assert_eq!(view.valign(), gtk::Align::Fill);
    assert!(view.vexpands());

    // ── Failing behaves the same way as Offline ───────────────────────────
    view.set_status(status(ConnectionState::Failing), 12, 4000, 0);
    settle();
    assert!(view.has_css_class("postio-liststate-banner"));

    // ── Back online with rows: hides entirely, neither treatment ─────────
    view.set_status(status(ConnectionState::Online), 12, 4000, 0);
    settle();
    assert!(!view.is_visible());
}
