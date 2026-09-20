//! `on_first_frame` fires for a window that is already up (#1434).
//!
//! It hooks `map`, and **a widget that is already mapped never emits `map`
//! again**. That was harmless while the only caller registered it while
//! building the window; it stopped being harmless when startup began hanging
//! real work off the first frame, because `activate` handlers run in
//! registration order and the one that presents the window runs first.
//!
//! Hooked the old way, the deferred work simply never runs, and the failure
//! is silent: no account opens, no mail appears, and nothing says why.
//!
//! Skips without a display. Nothing here touches the network.

use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

pub fn work_deferred_to_the_first_frame_runs_even_if_the_window_is_up() {
    if adw::init().is_err() || gtk::gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    // ── the ordinary way round: hooked before the window is shown ────────
    let early = gtk::Window::new();
    let ran_early = Rc::new(Cell::new(false));
    {
        let ran = Rc::clone(&ran_early);
        postio_gtk::startup::on_first_frame(&early, move || ran.set(true));
    }
    early.present();
    crate::settle_until("the early hook to run", || ran_early.get());
    early.destroy();

    // ── and the case that was broken: hooked after it is already up ──────
    let late = gtk::Window::new();
    late.present();
    crate::settle_until("the window to be mapped", || late.is_mapped());

    let ran_late = Rc::new(Cell::new(false));
    {
        let ran = Rc::clone(&ran_late);
        postio_gtk::startup::on_first_frame(&late, move || ran.set(true));
    }
    crate::settle_until("the late hook to run", || ran_late.get());
    assert!(
        ran_late.get(),
        "work deferred to the first frame never ran, because the window was \
         already mapped and `map` does not fire twice. Startup defers opening \
         the account this way, so this failing means no account opens at all \
         (#1434)"
    );

    late.destroy();
}
