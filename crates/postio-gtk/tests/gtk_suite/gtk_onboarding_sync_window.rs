//! The sync-window step (#876): the last question before Postio starts
//! talking to the server on its own.
//!
//! Its own file with one test function, for the same reason
//! `gtk_onboarding.rs` gives: two `#[test]`s here would race `adw::init()`.
//!
//! `postio-app`'s `write_sync_window` — whether the chosen window actually
//! reaches `SyncConfig.initial_sync_messages` — is proven in
//! `postio-app`'s own wiring test; this proves only what a display can:
//! that the step renders, that picking a window updates the estimate, and
//! that `Start sync` fires with the picker's own selection.

use postio_gtk::onboarding::{Onboarding, Status, SyncWindow};
use std::cell::RefCell;
use std::rc::Rc;

pub fn picking_a_window_updates_the_estimate_and_start_sync_fires_it() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }

    let screen = Onboarding::new();
    screen.set_address("ada@example.com");

    // Before the step shows, its section stays out of the way — the same
    // "not this status" absence every other step in this widget keeps.
    assert!(!screen.test_sync_window_shown());

    screen.set_status(Status::SyncWindow);
    assert!(
        screen.test_sync_window_shown(),
        "Status::SyncWindow must show the picker, the estimate and Start sync"
    );

    // The default is a year, matching SyncConfig::initial_sync_messages's
    // own default — picking it changes nothing a fresh install would not
    // already do.
    assert_eq!(screen.sync_window(), SyncWindow::LastYear);
    let year_estimate = screen.test_sync_estimate();
    assert!(
        year_estimate.contains("MB"),
        "a concrete window's estimate names a size: {year_estimate}"
    );

    screen.test_select_sync_window(SyncWindow::LastMonth);
    assert_eq!(screen.sync_window(), SyncWindow::LastMonth);
    let month_estimate = screen.test_sync_estimate();
    assert_ne!(
        month_estimate, year_estimate,
        "a smaller window must read as a smaller estimate: {month_estimate}"
    );

    screen.test_select_sync_window(SyncWindow::Everything);
    assert_eq!(screen.sync_window(), SyncWindow::Everything);
    assert!(
        !screen.test_sync_estimate().contains("MB"),
        "there is no size to name for an unbounded sync: {}",
        screen.test_sync_estimate()
    );

    // `Start sync` hands the handler exactly what the picker was showing —
    // not the default, and not whatever the last change happened to be.
    let fired: Rc<RefCell<Vec<SyncWindow>>> = Rc::new(RefCell::new(Vec::new()));
    screen.connect_start_sync({
        let fired = fired.clone();
        move |window| fired.borrow_mut().push(window)
    });
    screen.test_select_sync_window(SyncWindow::LastMonth);
    screen.start_sync();
    assert_eq!(fired.borrow().as_slice(), [SyncWindow::LastMonth]);
}
