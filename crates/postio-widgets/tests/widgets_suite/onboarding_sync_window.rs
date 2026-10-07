//! The sync-window step (#876): the last question before Postio starts
//! talking to the server on its own.
//!
//! Its own file with one test function, for the same reason
//! `gtk_onboarding.rs` gives: two `#[test]`s here would race `adw::init()`.
//!
//! `write_sync_window` — whether the chosen window actually
//! reaches `SyncConfig.initial_sync_messages` — is proven in the
//! presenter's own tests (`present/onboarding.rs`); this proves only what a display can:
//! that the step renders, that picking a window updates the estimate, and
//! that `Start sync` fires with the picker's own selection.

use postio_widgets::onboarding::{Onboarding, Status, SyncWindow};
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

    // The step's one button says the key that presses it: it holds the
    // keyboard, so Return starts the sync.
    fn caps_in(widget: &gtk::Widget, found: &mut Vec<String>) {
        use gtk::prelude::*;
        if widget.has_css_class("postio-keyhint")
            && let Some(label) = widget.downcast_ref::<gtk::Label>()
        {
            found.push(label.label().to_string());
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            caps_in(&next, found);
            child = next.next_sibling();
        }
    }
    let mut caps = Vec::new();
    caps_in(gtk::prelude::Cast::upcast_ref(&screen), &mut caps);
    assert!(
        caps.iter().any(|cap| cap == "Return"),
        "Start sync shows no Return cap: {caps:?}"
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
