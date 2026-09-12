//! A store that will not open, refused at a window somebody is already
//! looking at (#1114).
//!
//! ADR 0014 Q3 makes a store that does not open a hard stop rather than a
//! degraded mode, and #404 is the screen that says so. Neither of those
//! changes here. What changes is *when* the refusal arrives: until #1114 the
//! store was opened before `app::build_with` was called at all, so the answer
//! was known before there was an application, let alone a window. Now the
//! window is presented first and the store opens behind it, and the refusal
//! has to replace the content of a window that is already on screen.
//!
//! #1114's own issue text flags this as where the bug will be, which is why
//! the assertion is over the real composition root rather than over the
//! widget. `postio_gtk::unavailable` is unit-tested on its own; what needs
//! driving is `postio_app::present` choosing it, and the retry from it
//! reaching a store.
//!
//! One test function, for the reason `wiring.rs` gives: GTK is initialised
//! once, per process, from one thread. Nothing here dials anything — the
//! keyring is a `MemorySecretStore` over a scratch store path.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. Set before the code under test runs, which is the one
// moment it is sound.

use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::gdk;
use postio_account::secret::MemorySecretStore;
use postio_app::{Installation, present};
use postio_gtk::unavailable::Unavailable;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};

use crate::settle_until;

/// The sentence the opener produced, in `open_store_at`'s own words.
const REFUSED: &str = "Postio could not unlock its local store. it belongs to \
                       another installation";

pub fn a_store_refused_after_the_window_is_up_says_so_and_can_be_retried() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    let store_dir = tempfile::tempdir().expect("a store directory");
    // SAFETY: first statements of a single-threaded test, before anything
    // under test reads either.
    unsafe {
        std::env::set_var("XDG_STATE_HOME", state_dir.path());
        // A scratch store, because the retry below really does open one --
        // that is the half of this case that would otherwise prove nothing.
        std::env::set_var("POSTIO_STORE", store_dir.path().join("postio.db"));
    }

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `scripts/test-headless.sh`)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // A window on screen with nothing behind it, which before #1114 was not
    // a state this application had.
    let window = Window::default();
    window.set_waiting_on(postio_gtk::list_state::Waiting::Keyring);
    window.present();
    while gtk::glib::MainContext::default().iteration(false) {}
    assert!(window.is_mapped(), "the window is up before the store is");

    let context = Rc::new(Installation::new(Arc::new(MemorySecretStore::default())));
    let opened = Rc::new(std::cell::RefCell::new(None));
    let fed = Rc::new(std::cell::Cell::new(false));

    present(&window, &opened, &context, Some(REFUSED.to_owned()), &fed);
    while gtk::glib::MainContext::default().iteration(false) {}

    let screen = window
        .content()
        .and_downcast::<Unavailable>()
        .expect("a refusal at a window already on screen has to replace what it is showing");
    assert!(
        screen.reason().contains("belongs to another installation"),
        "the screen composed a sentence of its own instead of showing the \
         one the opener produced: {:?}",
        screen.reason()
    );

    // ── and the retry reaches a store ────────────────────────────────────
    //
    // The keyring is empty, so `store_key` mints one and the scratch path
    // opens clean. What the window shows afterwards is onboarding — there is
    // no account in a store that has just been created — and that is the
    // point: it is no longer the refusal.
    screen.retry();
    assert!(
        settle_until(|| window.content().and_downcast::<Unavailable>().is_none()),
        "the retry never got past the screen it was pressed on"
    );
    assert!(
        window.content().is_some(),
        "and it left a window with something in it"
    );

    window.close();
    while gtk::glib::MainContext::default().iteration(false) {}
}
